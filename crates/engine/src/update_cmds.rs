//! `updates.check`: fetch every app's release feed (or one app's) in the background. The
//! toolbox's own feed (`Catalog::toolbox`, how it updates itself) is one more entry of the list.
//!
//! First, when the settings follow the publisher (`remoteCatalog`), the signed remote catalog
//! and the aggregated feed are fetched with two conditional GETs that don't touch the API
//! (`remote`, docs/architecture.md § 12): a newer catalog replaces the session's, and every app
//! the feed covers is done. The rest, or everything when the feed can't be used, goes to GitHub:
//! per app, one conditional GET of `/repos/<repo>/releases?per_page=20` with the cached ETag.
//! A new body is parsed on the worker and cached (raw, with its ETag) before it reaches the
//! session; a `304` only moves the check time.
//!
//! GitHub allows 60 anonymous API requests per hour per IP address, and a `304` still counts
//! (measured 2026-10-09, docs/release-contract.md), so one full check costs 12 of them. Hence:
//! apps checked within the last [`FRESH_SECS`] are not requested again (`force` overrides), and
//! once GitHub says the limit is used up (`x-ratelimit-remaining: 0`, or a `403`/`429` refusal)
//! the remaining apps are skipped and checks stay disabled until GitHub's reset time. A token in
//! `ARTCRAFT_TOOLBOX_GITHUB_TOKEN` raises the limit to 5,000 per hour.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use artcraft_toolbox_catalog::Catalog;
use artcraft_toolbox_feed::{Release, Status};
use artcraft_toolbox_net::{ACCEPT_GITHUB_JSON, NetError, Request, Response, Transport};
use artcraft_toolbox_store::{CachedFeed, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::CommandSpec;
use crate::jobs::{JobMsg, JobState, PoolTask, Running, spawn_pool};
use crate::remote::{self, RemoteTask};
use crate::{CatalogSource, EngineError, Feed, JobId, Result, Session, params, setup, time};

pub const CHECK: &str = "updates.check";
/// Releases asked for per app: enough to find the newest stable build behind a run of
/// pre-releases, without downloading years of release notes.
pub const PER_PAGE: u32 = 20;
/// An app checked this recently is not requested again (unless `force`).
pub const FRESH_SECS: u64 = 60;
/// Feeds fetched at the same time.
pub const WORKERS: usize = 4;
/// How long to back off when GitHub refuses without saying until when.
const DEFAULT_BACKOFF_SECS: u64 = 60;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec { id: CHECK, label: "Check for Updates", params: r#"{"app"?:"<id>","force"?:bool}"#, enabled: can_check, run, start: Some(start) }]
}

fn can_check(s: &Session) -> std::result::Result<(), String> {
    if !s.online() {
        return Err("this session has no network access".into());
    }
    if s.jobs.runs(CHECK) {
        return Err("a check is already running".into());
    }
    if let Some(until) = s.rate_limited_until() {
        return Err(rate_limit_text(s.now(), until));
    }
    Ok(())
}

/// The user-facing explanation of GitHub's rate limit.
pub fn rate_limit_text(now: u64, until: u64) -> String {
    format!(
        "GitHub's request limit for this computer is used up; checking can resume {}. A GitHub token in {} raises the limit.",
        time::until(now, until),
        setup::ENV_GITHUB_TOKEN
    )
}

/// What a check did, per app. Also the command's JSON result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSummary {
    /// New release data arrived.
    pub fetched: Vec<String>,
    /// Unchanged since the last check (`304`).
    pub unchanged: Vec<String>,
    /// Checked less than [`FRESH_SECS`] ago: not requested again.
    pub fresh: Vec<String>,
    /// Not attempted: GitHub refused further requests, or the check was cancelled.
    pub skipped: Vec<String>,
    pub failed: Vec<Failure>,
    /// Apps with an update available once the check ended.
    pub updates_available: Vec<String>,
    /// Set when GitHub's rate limit stopped the check.
    pub rate_limited_until: Option<u64>,
    /// The publisher's signed catalog: what happened to it in this check.
    pub catalog: RemoteState,
    /// The publisher's aggregated feed: what happened to it in this check.
    pub feed: RemoteState,
    /// Apps whose releases came from the aggregated feed (the rest were asked from GitHub).
    pub aggregated: Vec<String>,
}

/// What a check did with one of the publisher's documents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RemoteState {
    /// Not used: the setting is off, or the catalog names no publisher.
    #[default]
    Off,
    /// Fetched and in use.
    Fetched,
    /// `304`: the cached one is current.
    Unchanged,
    /// A newer catalog replaced the session's.
    Applied { revision: u64 },
    /// Couldn't be fetched, verified or accepted; the check went on without it.
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub app: String,
    pub error: String,
}

impl CheckSummary {
    /// Apps the check has accounted for, whatever the outcome.
    pub fn done(&self) -> usize {
        self.fetched.len() + self.unchanged.len() + self.fresh.len() + self.skipped.len() + self.failed.len()
    }

    fn accounted(&self, app: &str) -> bool {
        [&self.fetched, &self.unchanged, &self.fresh, &self.skipped].iter().any(|v| v.iter().any(|a| a == app)) || self.failed.iter().any(|f| f.app == app)
    }
}

/// What a check worker reports about one app, or about the publisher's documents.
pub(crate) enum CheckMsg {
    Fetched {
        app: String,
        releases: Vec<Release>,
        etag: Option<String>,
        at: u64,
    },
    Unchanged {
        app: String,
        at: u64,
    },
    Failed {
        app: String,
        error: String,
    },
    Skipped(String),
    RateLimited {
        until: u64,
    },
    /// A verified, newer remote catalog.
    Catalog {
        catalog: Box<Catalog>,
        at: u64,
    },
    CatalogUnchanged {
        at: u64,
    },
    CatalogFailed {
        error: String,
    },
    /// The aggregated feed covered `apps` (each also gets its own `Fetched`).
    FeedFetched {
        at: u64,
        apps: Vec<String>,
    },
    FeedUnchanged {
        at: u64,
    },
    FeedFailed {
        error: String,
    },
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = start(s, p)?;
    s.wait_job(id)
}

struct Task {
    app: String,
    url: String,
    slugs: Vec<String>,
    etag: Option<String>,
}

impl PoolTask for Task {
    fn app(&self) -> &str {
        &self.app
    }
    fn panicked(&self) -> JobMsg {
        JobMsg::Check(CheckMsg::Failed { app: self.app.clone(), error: "internal error (logged)".into() })
    }
}

fn start(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["app", "force"])?;
    let force = match params::object(p)?.get("force") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(EngineError::BadParams(format!("`force` must be true or false, got {}", params::kind(other)))),
    };
    let ids: Vec<String> = match params::object(p)?.get("app") {
        None => s.catalog().apps.iter().chain(&s.catalog().toolbox).map(|a| a.id.clone()).collect(),
        Some(_) => vec![params::feed_app(s, p)?.id.clone()],
    };
    let transport = s.transport.clone().ok_or_else(|| EngineError::Disabled { id: CHECK.into(), reason: "this session has no network access".into() })?;
    let now = s.now();
    let mut summary = CheckSummary::default();
    let mut tasks = Vec::new();
    for id in &ids {
        let Some(app) = s.feed_app(id) else { continue };
        let feed = s.feeds.get(id);
        if !force && feed.is_some_and(|f| now.saturating_sub(f.fetched_at) < FRESH_SECS) {
            summary.fresh.push(id.clone());
            continue;
        }
        tasks.push(Task {
            app: id.clone(),
            url: format!("{}?per_page={PER_PAGE}", app.releases_api_url()),
            slugs: app.slugs().iter().map(|x| x.to_string()).collect(),
            etag: feed.and_then(|f| f.etag.clone()),
        });
    }
    s.last_check_started = Some(now);
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (store, clock) = (s.store.clone(), s.clock);
    // The publisher's documents first, when followed and when something is to be checked.
    let remote_task = match (s.remote_enabled() && !tasks.is_empty(), s.pinned_key.clone()) {
        (true, Some(key)) => Some(RemoteTask { catalog: s.catalog.clone(), key, have_feed: s.feeds.keys().cloned().collect() }),
        _ => None,
    };
    if remote_task.is_none() {
        summary.catalog = RemoteState::Off;
        summary.feed = RemoteState::Off;
    }
    let pool_cancel = Arc::clone(&cancel);
    let spawned = std::thread::Builder::new().name("updates-check".into()).spawn(move || {
        let mut covered = BTreeSet::new();
        if let Some(task) = remote_task {
            let outcome = catch_unwind(AssertUnwindSafe(|| remote::run(&task, transport.as_ref(), store.as_ref(), clock, &pool_cancel)));
            let (msgs, done) = outcome.unwrap_or_else(|_| {
                log::error!("the remote catalog phase panicked");
                (vec![CheckMsg::FeedFailed { error: "internal error (logged)".into() }], BTreeSet::new())
            });
            for m in msgs {
                if tx.send(JobMsg::Check(m)).is_err() {
                    return;
                }
            }
            covered = done;
        }
        let tasks: Vec<Task> = tasks.into_iter().filter(|t| !covered.contains(&t.app)).collect();
        // Set once GitHub refuses: the remaining apps are skipped, not requested.
        let stop = AtomicBool::new(false);
        spawn_pool("updates-check", tasks, WORKERS, pool_cancel, tx, move |task: &Task| {
            if stop.load(Ordering::Relaxed) {
                return vec![JobMsg::Check(CheckMsg::Skipped(task.app.clone()))];
            }
            check_one(task, transport.as_ref(), store.as_ref(), clock, &stop).into_iter().map(JobMsg::Check).collect()
        });
    });
    if let Err(e) = spawned {
        return Err(EngineError::Job(format!("couldn't start the check: {e}")));
    }
    let id = s.jobs.next_id();
    s.jobs.push(Running {
        id,
        command: CHECK,
        label: "Checking for updates".into(),
        rx,
        cancel,
        items: ids,
        in_flight: BTreeSet::new(),
        state: JobState::Check(summary),
    });
    Ok(id)
}

/// Apply one worker message to the session (on the session's thread).
pub(crate) fn apply(s: &mut Session, job: &mut Running, msg: CheckMsg) {
    let JobState::Check(summary) = &mut job.state else { return };
    match msg {
        CheckMsg::Fetched { app, releases, etag, at } => {
            job.in_flight.remove(&app);
            s.check_errors.remove(&app);
            s.feeds.insert(app.clone(), Feed { releases, fetched_at: at, etag });
            summary.fetched.push(app);
        }
        CheckMsg::Unchanged { app, at } => {
            job.in_flight.remove(&app);
            s.check_errors.remove(&app);
            if let Some(f) = s.feeds.get_mut(&app) {
                f.fetched_at = at;
            }
            summary.unchanged.push(app);
        }
        CheckMsg::Failed { app, error } => {
            job.in_flight.remove(&app);
            s.check_errors.insert(app.clone(), error.clone());
            summary.failed.push(Failure { app, error });
        }
        CheckMsg::Skipped(app) => {
            job.in_flight.remove(&app);
            summary.skipped.push(app);
        }
        CheckMsg::RateLimited { until } => {
            s.rate_limited_until = Some(s.rate_limited_until.map_or(until, |t| t.max(until)));
            summary.rate_limited_until = Some(until);
        }
        CheckMsg::Catalog { catalog, at } => {
            summary.catalog = match s.set_catalog(*catalog, at) {
                Ok(revision) => {
                    // Apps the new catalog adds join this check (the feed may cover them).
                    for app in s.catalog.apps.iter().chain(&s.catalog.toolbox) {
                        if !job.items.contains(&app.id) {
                            job.items.push(app.id.clone());
                        }
                    }
                    RemoteState::Applied { revision }
                }
                Err(error) => RemoteState::Failed { error },
            };
        }
        CheckMsg::CatalogUnchanged { at } => {
            if let CatalogSource::Remote { .. } = s.catalog_source {
                s.catalog_source = CatalogSource::Remote { fetched_at: at };
            }
            summary.catalog = RemoteState::Unchanged;
        }
        CheckMsg::CatalogFailed { error } => summary.catalog = RemoteState::Failed { error },
        CheckMsg::FeedFetched { at, apps } => {
            s.set_feed_fetched_at(at);
            summary.feed = RemoteState::Fetched;
            summary.aggregated = apps;
        }
        CheckMsg::FeedUnchanged { at } => {
            s.set_feed_fetched_at(at);
            summary.feed = RemoteState::Unchanged;
        }
        CheckMsg::FeedFailed { error } => summary.feed = RemoteState::Failed { error },
    }
}

/// The job's result once its workers are gone.
pub(crate) fn finish(s: &Session, job: Running) -> Value {
    let JobState::Check(mut summary) = job.state else { return Value::Null };
    // Items no worker reported on (cancelled, or a worker thread couldn't start) were skipped.
    for app in &job.items {
        if !summary.accounted(app) {
            summary.skipped.push(app.clone());
        }
    }
    summary.updates_available = s.statuses().into_iter().filter(|r| matches!(r.status, Status::UpdateAvailable { .. })).map(|r| r.id).collect();
    serde_json::to_value(&summary).unwrap_or(Value::Null)
}

impl Session {
    /// One line for the user about a finished check (`updates.check`'s result), or `None` when
    /// every app was checked.
    pub fn check_notice(&self, result: &Value) -> Option<String> {
        let summary: CheckSummary = serde_json::from_value(result.clone()).ok()?;
        if let Some(until) = summary.rate_limited_until {
            return Some(rate_limit_text(self.now(), until));
        }
        let name = |id: &str| self.feed_app(id).map_or_else(|| id.to_string(), |a| a.name.clone());
        match summary.failed.as_slice() {
            [] => None,
            [f] => Some(format!("Couldn't check {}: {}", name(&f.app), f.error)),
            [f, rest @ ..] => Some(format!("Couldn't check {} apps: {}", rest.len() + 1, f.error)),
        }
    }
}

/// Fetch, parse and cache one app's feed.
fn check_one(task: &Task, transport: &dyn Transport, store: Option<&Store>, clock: fn() -> u64, stop: &AtomicBool) -> Vec<CheckMsg> {
    let req = Request { url: &task.url, accept: ACCEPT_GITHUB_JSON, etag: task.etag.as_deref(), max_bytes: artcraft_toolbox_feed::github::MAX_RESPONSE_BYTES };
    let result = transport.get(&req);
    let now = clock();
    let app = task.app.clone();
    let mut out = Vec::new();
    match result {
        Ok(resp) => {
            let rate = resp.rate();
            out.push(match resp {
                Response::NotModified { .. } => {
                    if let Some(store) = store
                        && let Err(e) = store.touch_feed(&app, now)
                    {
                        log::warn!("{e}");
                    }
                    CheckMsg::Unchanged { app, at: now }
                }
                Response::Ok { body, etag, .. } => match String::from_utf8(body) {
                    Err(_) => CheckMsg::Failed { app, error: "the release feed is not UTF-8 text".into() },
                    Ok(text) => {
                        let slugs: Vec<&str> = task.slugs.iter().map(String::as_str).collect();
                        match artcraft_toolbox_feed::parse_releases(&text, &slugs) {
                            Err(e) => CheckMsg::Failed { app, error: e.to_string() },
                            Ok(releases) => {
                                if let Some(store) = store
                                    && let Err(e) = store.save_feed(&app, &CachedFeed { etag: etag.clone(), fetched_at: now, body: text })
                                {
                                    log::warn!("{e}");
                                }
                                CheckMsg::Fetched { app, releases, etag, at: now }
                            }
                        }
                    }
                },
            });
            if rate.remaining == Some(0) {
                stop.store(true, Ordering::Relaxed);
                out.push(CheckMsg::RateLimited { until: rate.reset_at.unwrap_or(now.saturating_add(DEFAULT_BACKOFF_SECS)) });
            }
        }
        Err(NetError::RateLimited { reset_at, retry_after }) => {
            stop.store(true, Ordering::Relaxed);
            let until = reset_at.or(retry_after.map(|r| now.saturating_add(r))).unwrap_or(now.saturating_add(DEFAULT_BACKOFF_SECS));
            out.push(CheckMsg::Failed { app, error: NetError::RateLimited { reset_at, retry_after }.to_string() });
            out.push(CheckMsg::RateLimited { until });
        }
        // Public release feeds need no credentials: a 401 means the token the user set is bad.
        Err(NetError::Status { status: 401, message }) => {
            out.push(CheckMsg::Failed { app, error: format!("GitHub rejected the token in {} ({message})", setup::ENV_GITHUB_TOKEN) })
        }
        Err(e) => out.push(CheckMsg::Failed { app, error: e.to_string() }),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Started;
    use crate::tests::temp;
    use artcraft_toolbox_catalog::Catalog;
    use artcraft_toolbox_net::{NetError, RateLimit};
    use artcraft_toolbox_release::{Arch, Os, Target};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

    /// Answers by app (the repo name in the URL); records every request.
    struct Fake {
        answers: HashMap<&'static str, std::result::Result<Response, NetError>>,
        seen: Mutex<Vec<(String, Option<String>)>>,
    }

    impl Fake {
        fn new(answers: Vec<(&'static str, std::result::Result<Response, NetError>)>) -> Arc<Fake> {
            Arc::new(Fake { answers: answers.into_iter().collect(), seen: Mutex::new(Vec::new()) })
        }
        fn requests(&self) -> Vec<(String, Option<String>)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, req: &Request<'_>) -> std::result::Result<Response, NetError> {
            self.seen.lock().unwrap().push((req.url.to_string(), req.etag.map(str::to_string)));
            let repo = req.url.split('/').nth(5).unwrap_or_default();
            self.answers.get(repo).cloned().unwrap_or(Ok(Response::Ok { body: b"[]".to_vec(), etag: None, rate: RateLimit::default() }))
        }
    }

    fn ok(body: &str, etag: &str, remaining: u32) -> std::result::Result<Response, NetError> {
        Ok(Response::Ok {
            body: body.as_bytes().to_vec(),
            etag: Some(etag.into()),
            rate: RateLimit { limit: Some(60), remaining: Some(remaining), reset_at: Some(2_000_000) },
        })
    }

    fn t0() -> u64 {
        1_000_000
    }
    fn t1() -> u64 {
        1_000_000 + 3600
    }

    fn session(fake: Arc<Fake>, store: Option<Store>) -> Session {
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), store, Some(fake));
        s.set_clock(t0);
        s.set_host(Some(Target::new(Os::Linux, Arch::X86_64)));
        // These tests are about the GitHub API path; `tests/remote_catalog.rs` covers the feed.
        s.execute("settings.set", json!({"remoteCatalog": false})).unwrap();
        s
    }

    #[test]
    fn a_check_fetches_every_app_and_caches_the_feeds() {
        let root = temp("check");
        let store = Store::open(&root).unwrap();
        let fake = Fake::new(vec![("photocraft", ok(PHOTOCRAFT, "W/\"p1\"", 50))]);
        let mut s = session(Arc::clone(&fake), Some(store.clone()));
        let r = s.execute(CHECK, json!({})).unwrap();
        let summary: CheckSummary = serde_json::from_value(r.clone()).unwrap();
        assert_eq!(summary.fetched.len(), 14, "13 apps and the toolbox itself: {summary:?}");
        assert!(summary.failed.is_empty() && summary.rate_limited_until.is_none());
        assert_eq!((summary.catalog.clone(), summary.feed.clone()), (RemoteState::Off, RemoteState::Off));
        assert_eq!(fake.requests().len(), 14);
        assert!(fake.requests().iter().any(|(url, _)| url.contains("/artcraft-toolbox/releases")), "the toolbox's own feed");
        assert!(fake.requests().iter().all(|(url, etag)| url.ends_with("/releases?per_page=20") && etag.is_none()));
        assert_eq!(s.app_status("photocraft").unwrap().status, Status::NotInstalled { latest: artcraft_toolbox_release::Version::new(0, 5, 0) });
        assert_eq!(s.app_status("photocraft").unwrap().checked_at, Some(t0()));
        assert_eq!(s.check_notice(&r), None);
        // Cached with its ETag.
        let cached = store.load_feed("photocraft").unwrap().unwrap();
        assert_eq!((cached.etag.as_deref(), cached.fetched_at), (Some("W/\"p1\""), t0()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_next_check_is_conditional_and_recently_checked_apps_are_not_requested() {
        let fake = Fake::new(vec![("photocraft", ok(PHOTOCRAFT, "W/\"p1\"", 50))]);
        let mut s = session(Arc::clone(&fake), None);
        s.execute(CHECK, json!({})).unwrap();
        // Seconds later: everything is fresh, nothing is requested.
        let r: CheckSummary = serde_json::from_value(s.execute(CHECK, json!({})).unwrap()).unwrap();
        assert_eq!((r.fresh.len(), fake.requests().len()), (14, 14));
        // An hour later: requested again, with the ETag.
        s.set_clock(t1);
        let fake2 = Fake::new(vec![("photocraft", Ok(Response::NotModified { rate: RateLimit::default() }))]);
        s.transport = Some(fake2.clone());
        let r: CheckSummary = serde_json::from_value(s.execute(CHECK, json!({"app": "photocraft"})).unwrap()).unwrap();
        assert_eq!(r.unchanged, ["photocraft"]);
        assert_eq!(fake2.requests(), [("https://api.github.com/repos/storytold/photocraft/releases?per_page=20".to_string(), Some("W/\"p1\"".to_string()))]);
        assert_eq!(s.feed("photocraft").unwrap().fetched_at, t1(), "a 304 moves the check time");
        assert_eq!(s.feed("photocraft").unwrap().releases.len(), 4, "and keeps the releases");
        // `force` ignores freshness.
        let r: CheckSummary = serde_json::from_value(s.execute(CHECK, json!({"app": "photocraft", "force": true})).unwrap()).unwrap();
        assert_eq!(r.unchanged, ["photocraft"]);
    }

    #[test]
    fn the_rate_limit_stops_the_check_and_disables_checking_until_the_reset() {
        let fake = Fake::new(vec![
            ("photocraft", Err(NetError::RateLimited { reset_at: Some(t0() + 1200), retry_after: None })),
            ("vectorcraft", Err(NetError::RateLimited { reset_at: Some(t0() + 1200), retry_after: None })),
            ("filmcraft", Err(NetError::RateLimited { reset_at: Some(t0() + 1200), retry_after: None })),
            ("lightcraft", Err(NetError::RateLimited { reset_at: Some(t0() + 1200), retry_after: None })),
        ]);
        let mut s = session(Arc::clone(&fake), None);
        let r = s.execute(CHECK, json!({})).unwrap();
        let summary: CheckSummary = serde_json::from_value(r.clone()).unwrap();
        assert_eq!(summary.rate_limited_until, Some(t0() + 1200));
        assert!(!summary.skipped.is_empty(), "apps after the refusal are not requested: {summary:?}");
        assert!(fake.requests().len() <= WORKERS, "{} requests", fake.requests().len());
        assert_eq!(summary.done(), 14);
        let notice = s.check_notice(&r).unwrap();
        assert!(notice.contains("request limit") && notice.contains("in 20 min") && notice.contains("ARTCRAFT_TOOLBOX_GITHUB_TOKEN"), "{notice}");
        let err = s.execute(CHECK, json!({})).unwrap_err().to_string();
        assert!(err.contains("can't run now") && err.contains("in 20 min"), "{err}");
        assert!(!s.check_due());
        s.set_clock(t1);
        assert!(s.disabled_reason(CHECK).is_none(), "checking is back after the reset");
    }

    #[test]
    fn the_last_request_of_the_window_stops_the_rest() {
        // GitHub answers but says no requests are left.
        let fake = Fake::new(vec![("photocraft", ok(PHOTOCRAFT, "W/\"p\"", 0))]);
        let mut s = session(Arc::clone(&fake), None);
        let summary: CheckSummary = serde_json::from_value(s.execute(CHECK, json!({"app": "photocraft"})).unwrap()).unwrap();
        assert_eq!(summary.fetched, ["photocraft"]);
        assert_eq!(summary.rate_limited_until, Some(2_000_000));
        assert!(s.rate_limited_until().is_some());
    }

    #[test]
    fn failures_are_per_app_and_keep_the_previous_feed() {
        let fake = Fake::new(vec![("photocraft", ok(PHOTOCRAFT, "W/\"p\"", 50))]);
        let mut s = session(fake, None);
        s.execute(CHECK, json!({"app": "photocraft"})).unwrap();
        s.set_clock(t1);
        s.transport = Some(Fake::new(vec![("photocraft", Err(NetError::Timeout)), ("vectorcraft", ok("not json", "e", 40))]));
        let r = s.execute(CHECK, json!({})).unwrap();
        let summary: CheckSummary = serde_json::from_value(r.clone()).unwrap();
        assert_eq!(summary.failed.len(), 2, "{summary:?}");
        let pc = s.app_status("photocraft").unwrap();
        assert_eq!(pc.error.as_deref(), Some("the server took too long to answer"));
        assert_eq!(pc.checked_at, Some(t0()), "the old feed stays");
        assert!(matches!(pc.status, Status::NotInstalled { .. }));
        // Which failure comes first depends on worker timing; the notice names the first.
        assert_eq!(s.check_notice(&r).unwrap(), format!("Couldn't check 2 apps: {}", summary.failed[0].error));
    }

    #[test]
    fn start_runs_in_the_background_and_poll_applies_results() {
        let fake = Fake::new(vec![("photocraft", ok(PHOTOCRAFT, "W/\"p\"", 50))]);
        let mut s = session(fake, None);
        let Started::Job(id) = s.start(CHECK, json!({})).unwrap() else { panic!("a job") };
        assert!(s.has_jobs());
        assert_eq!(s.jobs()[0].total, 14);
        assert!(s.execute(CHECK, json!({})).unwrap_err().to_string().contains("already running"));
        let mut events = Vec::new();
        for _ in 0..500 {
            events.extend(s.poll_jobs());
            if !s.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, id);
        assert!(s.statuses().iter().all(|r| !r.checking));
        // Inline commands come back done.
        assert!(matches!(s.start("catalog.list", json!({})).unwrap(), Started::Done(_)));
    }

    #[test]
    fn cancel_stops_a_check() {
        struct Slow;
        impl Transport for Slow {
            fn get(&self, _: &Request<'_>) -> std::result::Result<Response, NetError> {
                std::thread::sleep(std::time::Duration::from_millis(50));
                Err(NetError::Timeout)
            }
        }
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Slow)));
        s.execute("settings.set", json!({"remoteCatalog": false})).unwrap();
        let Started::Job(id) = s.start(CHECK, json!({})).unwrap() else { panic!("a job") };
        assert!(s.cancel_job(id));
        assert!(!s.has_jobs() && !s.cancel_job(id));
        assert!(s.disabled_reason(CHECK).is_none(), "a new check can start");
    }

    #[test]
    fn a_rejected_token_says_which_setting_to_fix() {
        let fake = Fake::new(vec![("photocraft", Err(NetError::Status { status: 401, message: "Bad credentials".into() }))]);
        let mut s = session(fake, None);
        let r = s.execute(CHECK, json!({"app": "photocraft"})).unwrap();
        assert_eq!(s.check_notice(&r).unwrap(), "Couldn't check PhotoCraft: GitHub rejected the token in ARTCRAFT_TOOLBOX_GITHUB_TOKEN (Bad credentials)");
    }

    #[test]
    fn bad_params() {
        let mut s = session(Fake::new(vec![]), None);
        for (p, want) in [
            (json!({"app": "nope"}), "unknown app"),
            (json!({"app": 3}), "must be a string"),
            (json!({"force": "yes"}), "true or false"),
            (json!({"apps": []}), "unknown param"),
        ] {
            let e = s.execute(CHECK, p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        assert!(Session::new().unwrap().execute(CHECK, json!({})).unwrap_err().to_string().contains("no network access"));
    }
}
