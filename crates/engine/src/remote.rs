//! The publisher's signed documents in a check (docs/architecture.md § 12): the remote catalog
//! and the aggregated release feed, fetched with two conditional GETs that don't count against
//! GitHub's API limit, verified against the pinned key (`release::signing`), and applied
//! through the check job's messages. Runs on the check's coordinator thread, before the per-app
//! workers; whatever it can't cover goes to the GitHub API as before.
//!
//! The feed is rebuilt every hour, but GitHub runs scheduled workflows late, by hours at a time,
//! so a feed can predate an app's newest release. One older than [`FEED_MAX_AGE_SECS`] doesn't
//! vouch for the apps whose updates matter (the installed ones and the toolbox): those go to
//! the API too, with the feed's list as their [`Fallback`] if GitHub can't answer.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use artcraft_toolbox_catalog::Catalog;
use artcraft_toolbox_feed::{Release, parse_aggregate, parse_releases_for};
use artcraft_toolbox_net::{NetError, Request, Response, Transport};
use artcraft_toolbox_release::signing::{self, Kind, MAX_ENVELOPE_BYTES, PublicKey};
use artcraft_toolbox_store::{CachedFeed, Store};

use crate::patterns_of;
use crate::update_cmds::CheckMsg;

/// The store's name for the cached catalog envelope.
pub const CATALOG_DOC: &str = "catalog";
/// The store's name for the aggregated feed's ETag and time. Its body is the feed's `generated`
/// time (Unix seconds), so an unchanged feed (`304`) still knows how old it is.
pub const FEED_DOC: &str = "feed";
/// An aggregated feed built longer ago than this missed at least one hourly rebuild (with
/// margin for the run and raw.githubusercontent.com's five-minute cache): installed apps and the
/// toolbox are confirmed with the API. Issue #3: the feed was 5 hours behind a PhotoCraft release.
pub const FEED_MAX_AGE_SECS: u64 = 90 * 60;
const ACCEPT: &str = "application/json";

/// A snapshot of what the coordinator needs from the session.
pub(crate) struct RemoteTask {
    /// The current catalog: its `[remote]` URLs, and what a new one is checked against.
    pub catalog: Catalog,
    pub key: PublicKey,
    /// Apps whose feed the session already holds: an unchanged aggregated feed keeps them.
    pub have_feed: BTreeSet<String>,
    /// Apps whose updates matter (installed, or the toolbox): a stale feed doesn't cover them.
    pub confirm: BTreeSet<String>,
}

/// What the publisher's documents did for a check.
#[derive(Default)]
pub(crate) struct Outcome {
    /// For the session, in order.
    pub msgs: Vec<CheckMsg>,
    /// Apps the feed covered: the per-app workers skip them.
    pub covered: BTreeSet<String>,
    /// Apps a stale feed left to the API, and what they get if GitHub can't answer.
    pub fallback: BTreeMap<String, Fallback>,
}

/// What a confirmed app falls back to when GitHub can't answer: a stale feed is still better
/// than an error.
pub(crate) enum Fallback {
    /// The aggregated feed's list for it.
    Feed { releases: Vec<Release>, body: String },
    /// The aggregated feed is unchanged (`304`): the session's list stands.
    Keep,
}

impl Fallback {
    /// The message that applies it (and caches it, like the feed would have).
    pub(crate) fn msg(&self, app: &str, store: Option<&Store>, at: u64) -> CheckMsg {
        match self {
            Fallback::Feed { releases, body } => {
                if let Some(store) = store
                    && let Err(e) = store.save_feed(app, &CachedFeed { etag: None, fetched_at: at, body: body.clone() })
                {
                    log::warn!("{e}");
                }
                CheckMsg::Fetched { app: app.into(), releases: releases.clone(), etag: None, at }
            }
            Fallback::Keep => {
                if let Some(store) = store
                    && let Err(e) = store.touch_feed(app, at)
                {
                    log::warn!("{e}");
                }
                CheckMsg::Unchanged { app: app.into(), at }
            }
        }
    }
}

/// Is a feed built at `generated` (unknown: a cache from before it was recorded) too old at `now`?
fn stale(generated: Option<u64>, now: u64) -> bool {
    generated.is_none_or(|g| now.saturating_sub(g) > FEED_MAX_AGE_SECS)
}

/// Fetch both documents.
pub(crate) fn run(task: &RemoteTask, transport: &dyn Transport, store: Option<&Store>, clock: fn() -> u64, cancel: &AtomicBool) -> Outcome {
    let mut msgs = Vec::new();
    let mut covered = BTreeSet::new();
    let mut fallback = BTreeMap::new();
    let Some(remote) = task.catalog.remote.clone() else { return Outcome { msgs, covered, fallback } };
    // 1. The catalog: a newer one is used for the feed step right away.
    let mut catalog = task.catalog.clone();
    match fetch(transport, store, clock, CATALOG_DOC, &remote.catalog) {
        Fetched::Unchanged(at) => msgs.push(CheckMsg::CatalogUnchanged { at }),
        Fetched::Failed(error) => {
            log::warn!("remote catalog: {error}");
            msgs.push(CheckMsg::CatalogFailed { error });
        }
        Fetched::Body { body, etag, at } => {
            let verified = signing::verify(&body, &task.key, Kind::Catalog)
                .map_err(|e| e.to_string())
                .and_then(|payload| Catalog::parse(&payload).map_err(|e| e.to_string()))
                .and_then(|c| task.catalog.accepts(&c).map(|()| c));
            match verified {
                Ok(new) => {
                    if let Some(store) = store
                        && let Err(e) = store.save_remote(CATALOG_DOC, &CachedFeed { etag, fetched_at: at, body })
                    {
                        log::warn!("{e}");
                    }
                    catalog = new.clone();
                    msgs.push(CheckMsg::Catalog { catalog: Box::new(new), at });
                }
                Err(error) => {
                    log::warn!("remote catalog refused: {error}");
                    msgs.push(CheckMsg::CatalogFailed { error });
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Outcome { msgs, covered, fallback };
    }
    // 2. The aggregated feed, from wherever the (possibly new) catalog says.
    let Some(remote) = catalog.remote.clone() else { return Outcome { msgs, covered, fallback } };
    match fetch(transport, store, clock, FEED_DOC, &remote.feed) {
        Fetched::Unchanged(at) => {
            let generated = store.and_then(|s| s.load_remote(FEED_DOC).ok().flatten()).and_then(|d| d.body.trim().parse().ok());
            let stale = stale(generated, at);
            for app in catalog.apps.iter().chain(&catalog.toolbox) {
                if task.have_feed.contains(&app.id) {
                    if stale && task.confirm.contains(&app.id) {
                        fallback.insert(app.id.clone(), Fallback::Keep);
                        continue;
                    }
                    if let Some(store) = store
                        && let Err(e) = store.touch_feed(&app.id, at)
                    {
                        log::warn!("{e}");
                    }
                    msgs.push(CheckMsg::Unchanged { app: app.id.clone(), at });
                    covered.insert(app.id.clone());
                }
            }
            msgs.push(CheckMsg::FeedUnchanged { at });
        }
        Fetched::Failed(error) => {
            log::warn!("aggregated feed: {error}");
            msgs.push(CheckMsg::FeedFailed { error });
        }
        Fetched::Body { body, etag, at } => {
            let parsed = signing::verify(&body, &task.key, Kind::Feed)
                .map_err(|e| e.to_string())
                .and_then(|payload| parse_aggregate(&payload).map_err(|e| e.to_string()));
            match parsed {
                Err(error) => {
                    log::warn!("aggregated feed refused: {error}");
                    msgs.push(CheckMsg::FeedFailed { error });
                }
                Ok(aggregate) => {
                    let stale = stale(Some(aggregate.generated), at);
                    for app in catalog.apps.iter().chain(&catalog.toolbox) {
                        let Some(text) = aggregate.apps.get(&app.id) else { continue };
                        match parse_releases_for(text, &app.slugs(), &patterns_of(app)) {
                            Ok(releases) if stale && task.confirm.contains(&app.id) => {
                                fallback.insert(app.id.clone(), Fallback::Feed { releases, body: text.clone() });
                            }
                            Ok(releases) => {
                                if let Some(store) = store
                                    && let Err(e) = store.save_feed(&app.id, &CachedFeed { etag: None, fetched_at: at, body: text.clone() })
                                {
                                    log::warn!("{e}");
                                }
                                msgs.push(CheckMsg::Fetched { app: app.id.clone(), releases, etag: None, at });
                                covered.insert(app.id.clone());
                            }
                            // Not covered: this app is asked from GitHub instead.
                            Err(e) => log::warn!("aggregated feed, {}: {e}", app.id),
                        }
                    }
                    if let Some(store) = store
                        && let Err(e) = store.save_remote(FEED_DOC, &CachedFeed { etag, fetched_at: at, body: aggregate.generated.to_string() })
                    {
                        log::warn!("{e}");
                    }
                    msgs.push(CheckMsg::FeedFetched { at, apps: covered.iter().cloned().collect() });
                }
            }
        }
    }
    if !fallback.is_empty() {
        log::info!(
            "the aggregated feed is more than {} minutes old: asking GitHub for {}",
            FEED_MAX_AGE_SECS / 60,
            fallback.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    Outcome { msgs, covered, fallback }
}

enum Fetched {
    Body { body: String, etag: Option<String>, at: u64 },
    Unchanged(u64),
    Failed(String),
}

/// One conditional GET of a signed document, with the ETag the store remembers for it.
fn fetch(transport: &dyn Transport, store: Option<&Store>, clock: fn() -> u64, doc: &str, url: &str) -> Fetched {
    let etag = store.and_then(|s| s.load_remote(doc).ok().flatten()).and_then(|d| d.etag);
    let req = Request { url, accept: ACCEPT, etag: etag.as_deref(), max_bytes: MAX_ENVELOPE_BYTES };
    let result = transport.get(&req);
    let at = clock();
    match result {
        Ok(Response::NotModified { .. }) => {
            if let Some(store) = store
                && let Err(e) = store.touch_remote(doc, at)
            {
                log::warn!("{e}");
            }
            Fetched::Unchanged(at)
        }
        Ok(Response::Ok { body, etag, .. }) => match String::from_utf8(body) {
            Ok(body) => Fetched::Body { body, etag, at },
            Err(_) => Fetched::Failed("not UTF-8 text".into()),
        },
        Err(NetError::Status { status: 404, .. }) => Fetched::Failed(format!("not published yet ({url} is 404)")),
        Err(e) => Fetched::Failed(e.to_string()),
    }
}
