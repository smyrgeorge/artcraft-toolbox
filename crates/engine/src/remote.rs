//! The publisher's signed documents in a check (docs/architecture.md § 12): the remote catalog
//! and the aggregated release feed, fetched with two conditional GETs that don't count against
//! GitHub's API limit, verified against the pinned key (`release::signing`), and applied
//! through the check job's messages. Runs on the check's coordinator thread, before the per-app
//! workers; whatever it can't cover goes to the GitHub API as before.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

use artcraft_toolbox_catalog::Catalog;
use artcraft_toolbox_feed::{parse_aggregate, parse_releases_for};
use artcraft_toolbox_net::{NetError, Request, Response, Transport};
use artcraft_toolbox_release::signing::{self, Kind, MAX_ENVELOPE_BYTES, PublicKey};
use artcraft_toolbox_store::{CachedFeed, Store};

use crate::patterns_of;
use crate::update_cmds::CheckMsg;

/// The store's name for the cached catalog envelope.
pub const CATALOG_DOC: &str = "catalog";
/// The store's name for the aggregated feed's ETag and time.
pub const FEED_DOC: &str = "feed";
const ACCEPT: &str = "application/json";

/// A snapshot of what the coordinator needs from the session.
pub(crate) struct RemoteTask {
    /// The current catalog: its `[remote]` URLs, and what a new one is checked against.
    pub catalog: Catalog,
    pub key: PublicKey,
    /// Apps whose feed the session already holds: an unchanged aggregated feed keeps them.
    pub have_feed: BTreeSet<String>,
}

/// Fetch both documents. Returns the messages for the session and the apps the feed covered
/// (which the per-app workers then skip).
pub(crate) fn run(
    task: &RemoteTask,
    transport: &dyn Transport,
    store: Option<&Store>,
    clock: fn() -> u64,
    cancel: &AtomicBool,
) -> (Vec<CheckMsg>, BTreeSet<String>) {
    let mut msgs = Vec::new();
    let mut covered = BTreeSet::new();
    let Some(remote) = task.catalog.remote.clone() else { return (msgs, covered) };
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
        return (msgs, covered);
    }
    // 2. The aggregated feed, from wherever the (possibly new) catalog says.
    let Some(remote) = catalog.remote.clone() else { return (msgs, covered) };
    match fetch(transport, store, clock, FEED_DOC, &remote.feed) {
        Fetched::Unchanged(at) => {
            for app in catalog.apps.iter().chain(&catalog.toolbox) {
                if task.have_feed.contains(&app.id) {
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
                    for app in catalog.apps.iter().chain(&catalog.toolbox) {
                        let Some(text) = aggregate.apps.get(&app.id) else { continue };
                        match parse_releases_for(text, &app.slugs(), &patterns_of(app)) {
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
                        && let Err(e) = store.save_remote(FEED_DOC, &CachedFeed { etag, fetched_at: at, body: String::new() })
                    {
                        log::warn!("{e}");
                    }
                    msgs.push(CheckMsg::FeedFetched { at, apps: covered.iter().cloned().collect() });
                }
            }
        }
    }
    (msgs, covered)
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
