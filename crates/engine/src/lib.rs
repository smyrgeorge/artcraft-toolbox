//! The ArtCraft Toolbox engine: where every behaviour lives.
//!
//! A [`Session`] holds the catalog, what is installed, the settings and the release feeds. Every
//! user-visible action is a command ([`command_specs`]) with an id, a params doc and a `run`
//! function; the desktop UI, the CLI and (later) the control channel and MCP all dispatch the
//! same commands by id through [`Session::execute`] (blocking) or [`Session::start`] (long
//! commands such as `updates.check` run as background [jobs](crate::jobs)). No UI toolkit below
//! L6 (`cargo xtask layers`).
//!
//! A session made with [`Session::new`] lives in memory and has no network access (tests, the
//! snapshot example). [`setup::open_user_session`] opens the real one: the data directory's
//! settings, inventory and feed cache, and a GitHub client.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app_settings_cmds;
pub mod build_info;
mod catalog_cmds;
mod commands;
pub mod icons;
pub mod install_cmds;
pub mod jobs;
mod params;
mod settings_cmds;
pub mod setup;
mod status_cmds;
pub mod time;
pub mod update_cmds;

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use artcraft_toolbox_feed::Release;
use artcraft_toolbox_model::{Channel, Inventory, Settings, ToolboxState};
use artcraft_toolbox_net::Transport;
use artcraft_toolbox_store::Store;
use serde::Serialize;
use serde_json::Value;

pub use artcraft_toolbox_catalog::Catalog;
pub use artcraft_toolbox_feed::Status;
pub use artcraft_toolbox_install::Layout;
/// The network layer, for callers that pass a [`Transport`] (the apps, test fakes).
pub use artcraft_toolbox_net as net;
pub use artcraft_toolbox_release::{Target, Version};
pub use commands::{CommandSpec, command_specs, find};
pub use icons::IconImage;
pub use jobs::{JobEvent, JobId, JobInfo, Started};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("`{id}` can't run now: {reason}")]
    Disabled { id: String, reason: String },
    #[error("bad params: {0}")]
    BadParams(String),
    #[error("unknown app `{0}`")]
    UnknownApp(String),
    #[error("`{0}` failed with an internal error (logged); nothing else was changed")]
    Internal(String),
    #[error("{0}")]
    Locked(String),
    #[error("{0}")]
    Job(String),
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Install(#[from] artcraft_toolbox_install::Error),
    #[error(transparent)]
    Catalog(#[from] artcraft_toolbox_catalog::Error),
    #[error(transparent)]
    Feed(#[from] artcraft_toolbox_feed::Error),
    #[error(transparent)]
    Model(#[from] artcraft_toolbox_model::Error),
    #[error(transparent)]
    Store(#[from] artcraft_toolbox_store::Error),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// At most 80 characters of a caller-supplied string, for error messages.
pub(crate) fn echo(s: &str) -> String {
    s.chars().take(80).collect()
}

/// The releases fetched for one app.
#[derive(Debug, Clone)]
pub struct Feed {
    /// Newest first.
    pub releases: Vec<Release>,
    /// Unix seconds of the last successful check.
    pub fetched_at: u64,
    /// For the next conditional request.
    pub etag: Option<String>,
}

/// One row of the app list: what the UI shows and `apps.status` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub status: Status,
    /// The channel this app follows (its own, or the global one).
    pub channel: Channel,
    /// The version this app is pinned to: nothing newer is offered.
    pub pinned: Option<Version>,
    /// Unix seconds of the last successful check of this app.
    pub checked_at: Option<u64>,
    /// A check of this app is in flight.
    pub checking: bool,
    /// Why the last check of this app failed, if it did.
    pub error: Option<String>,
}

pub struct Session {
    catalog: Catalog,
    inventory: Inventory,
    settings: Settings,
    feeds: BTreeMap<String, Feed>,
    host: Option<Target>,
    store: Option<Store>,
    transport: Option<Arc<dyn Transport>>,
    clock: fn() -> u64,
    jobs: jobs::Jobs,
    /// Unix seconds until which GitHub refuses requests from here, when it said so.
    rate_limited_until: Option<u64>,
    /// The last check's error, per app.
    check_errors: BTreeMap<String, String>,
    /// Set when the inventory file couldn't be read: it must not be overwritten.
    inventory_locked: Option<String>,
    /// When the last update check started (any outcome): automatic checks back off from it.
    last_check_started: Option<u64>,
    icons: BTreeMap<String, icons::Icon>,
    icon_revision: u64,
    icons_attempted_at: Option<u64>,
    /// What is remembered between runs (`state.json`).
    state: ToolboxState,
    /// Where apps are installed; `None`: this session doesn't install (tests, the snapshot).
    layout: Option<Layout>,
    /// The layout follows the platform and the `installDir` setting (the apps' sessions).
    platform_layout: bool,
}

/// After a check, an automatic one waits at least this long, whatever the outcome (an offline
/// machine doesn't retry every minute).
pub const AUTO_RETRY_SECS: u64 = 15 * 60;

impl Session {
    /// A session over the built-in catalog, on this machine, with nothing installed, nothing
    /// saved and no network access.
    pub fn new() -> Result<Session> {
        Ok(Session::with_catalog(Catalog::builtin()?))
    }

    pub fn with_catalog(catalog: Catalog) -> Session {
        Session {
            catalog,
            inventory: Inventory::default(),
            settings: Settings::default(),
            feeds: BTreeMap::new(),
            host: Target::host(),
            store: None,
            transport: None,
            clock: time::now_unix,
            jobs: jobs::Jobs::default(),
            rate_limited_until: None,
            check_errors: BTreeMap::new(),
            inventory_locked: None,
            last_check_started: None,
            icons: BTreeMap::new(),
            icon_revision: 0,
            icons_attempted_at: None,
            state: ToolboxState::default(),
            layout: None,
            platform_layout: false,
        }
    }

    /// A session that persists to `store` and reaches the network through `transport`. Loads the
    /// settings, the inventory and the cached feeds; what can't be read is reported in the
    /// returned warnings, never a reason not to start:
    ///
    /// - unreadable settings: kept as `settings.json.corrupt`, defaults used;
    /// - unreadable inventory: left untouched and locked (nothing is written over the only record
    ///   of what is installed where);
    /// - unreadable cached feed or icon: ignored, fetched again;
    /// - unreadable state: forgotten (at worst, one notification is shown again).
    pub fn open(catalog: Catalog, store: Option<Store>, transport: Option<Arc<dyn Transport>>) -> (Session, Vec<String>) {
        let mut s = Session::with_catalog(catalog);
        s.transport = transport;
        let mut warnings = Vec::new();
        if let Some(store) = &store {
            match store.load_settings() {
                Ok(Some(settings)) => s.settings = settings,
                Ok(None) => {}
                Err(e) => {
                    let kept =
                        store.back_up(artcraft_toolbox_store::SETTINGS_FILE).map(|p| format!(" (a copy is kept as {})", p.display())).unwrap_or_default();
                    warnings.push(format!("{e}; using the default settings{kept}"));
                }
            }
            match store.load_inventory() {
                Ok(Some(inventory)) => s.inventory = inventory,
                Ok(None) => {}
                Err(e) => {
                    let msg = format!("{e}; it is left untouched, and installing is disabled until it can be read");
                    s.inventory_locked = Some(msg.clone());
                    warnings.push(msg);
                }
            }
            match store.load_state() {
                Ok(Some(state)) => s.state = state,
                Ok(None) => {}
                Err(e) => log::warn!("{e}; starting afresh"),
            }
            s.load_cached_icons(store);
            let apps: Vec<(String, Vec<String>)> = s.catalog.apps.iter().map(|a| (a.id.clone(), a.slugs().iter().map(|x| x.to_string()).collect())).collect();
            for (id, slugs) in apps {
                let slugs: Vec<&str> = slugs.iter().map(String::as_str).collect();
                match store.load_feed(&id) {
                    Ok(Some(cached)) => match artcraft_toolbox_feed::parse_releases(&cached.body, &slugs) {
                        Ok(releases) => {
                            s.feeds.insert(id, Feed { releases, fetched_at: cached.fetched_at, etag: cached.etag });
                        }
                        Err(e) => log::warn!("cached feed of {id}: {e}; it will be fetched again"),
                    },
                    Ok(None) => {}
                    Err(e) => log::warn!("{e}; it will be fetched again"),
                }
            }
        }
        s.store = store;
        (s, warnings)
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Replace the inventory in memory (tests, the snapshot example).
    pub fn set_inventory(&mut self, inventory: Inventory) {
        self.inventory = inventory;
    }

    /// Write the inventory, unless it is locked because the file on disk couldn't be read.
    pub fn save_inventory(&self) -> Result<()> {
        if let Some(why) = &self.inventory_locked {
            return Err(EngineError::Locked(why.clone()));
        }
        match &self.store {
            Some(store) => Ok(store.save_inventory(&self.inventory)?),
            None => Ok(()),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Persist `settings`, then use them. If they can't be saved nothing changes.
    pub fn set_settings(&mut self, settings: Settings) -> Result<()> {
        if let Some(store) = &self.store {
            store.save_settings(&settings)?;
        }
        let moved = settings.install_dir != self.settings.install_dir;
        self.settings = settings;
        if moved && self.platform_layout {
            self.use_platform_layout();
        }
        Ok(())
    }

    /// Where apps are installed, when this session installs.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// Install into `layout` (tests; `None` turns installing off).
    pub fn set_layout(&mut self, layout: Option<Layout>) {
        self.layout = layout;
        self.platform_layout = false;
    }

    /// Install where the platform and the `installDir` setting say (docs/architecture.md § 5).
    /// Needs the data folder (downloads are staged there).
    pub fn use_platform_layout(&mut self) {
        self.platform_layout = true;
        self.layout = self
            .store
            .as_ref()
            .and_then(|st| Layout::platform(st.root(), self.settings.install_dir.as_deref().map(std::path::Path::new), |k| std::env::var_os(k)));
    }

    /// The machine releases are chosen for; `None` on a platform no craft ships for.
    pub fn host(&self) -> Option<Target> {
        self.host
    }

    /// Choose releases for another machine (tests, and agents asking "what would Windows get?").
    pub fn set_host(&mut self, host: Option<Target>) {
        self.host = host;
    }

    /// Where the session's files live, when it has any.
    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    /// Can this session reach the network?
    pub fn online(&self) -> bool {
        self.transport.is_some()
    }

    /// Unix seconds now, from the session's clock.
    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Replace the clock (tests).
    pub fn set_clock(&mut self, clock: fn() -> u64) {
        self.clock = clock;
    }

    pub fn feed(&self, app: &str) -> Option<&Feed> {
        self.feeds.get(app)
    }

    /// Take a GitHub "list releases" response for `app`. Returns how many releases it held.
    pub fn ingest_releases(&mut self, app: &str, json: &str, fetched_at: u64) -> Result<usize> {
        let entry = self.catalog.get(app).ok_or_else(|| EngineError::UnknownApp(echo(app)))?;
        let releases = artcraft_toolbox_feed::parse_releases(json, &entry.slugs())?;
        let n = releases.len();
        self.feeds.insert(app.to_string(), Feed { releases, fetched_at, etag: None });
        Ok(n)
    }

    pub fn app_status(&self, app: &str) -> Result<AppStatus> {
        let entry = self.catalog.get(app).ok_or_else(|| EngineError::UnknownApp(echo(app)))?;
        let installed = self.inventory.current(app).map(|i| &i.version);
        let feed = self.feeds.get(app);
        let (channel, pinned) = (self.settings.channel_for(app), self.settings.pinned(app));
        let status = artcraft_toolbox_feed::status(installed, feed.map(|f| f.releases.as_slice()), channel, self.host, pinned);
        Ok(AppStatus {
            id: entry.id.clone(),
            name: entry.name.clone(),
            tagline: entry.tagline.clone(),
            status,
            channel,
            pinned: pinned.cloned(),
            checked_at: feed.map(|f| f.fetched_at),
            checking: self.jobs.working_on(update_cmds::CHECK, app),
            error: self.check_errors.get(app).cloned(),
        })
    }

    /// Every catalog app, in catalog order.
    pub fn statuses(&self) -> Vec<AppStatus> {
        self.catalog.apps.iter().filter_map(|a| self.app_status(&a.id).ok()).collect()
    }

    /// When the most recent check ended (Unix seconds), if there ever was one.
    pub fn last_checked(&self) -> Option<u64> {
        self.feeds.values().map(|f| f.fetched_at).max()
    }

    /// Until when GitHub refuses requests from here (Unix seconds), if it does now.
    pub fn rate_limited_until(&self) -> Option<u64> {
        let now = self.now();
        self.rate_limited_until.filter(|t| *t > now)
    }

    /// Is an automatic update check due? When checks are on (`checkIntervalHours` > 0), the
    /// session is online, no check runs, GitHub isn't refusing requests, the last check started
    /// more than [`AUTO_RETRY_SECS`] ago, and some app's feed is older than the interval (or was
    /// never fetched).
    pub fn check_due(&self) -> bool {
        let hours = u64::from(self.settings.check_interval_hours);
        let now = self.now();
        if hours == 0 || !self.online() || self.jobs.runs(update_cmds::CHECK) || self.rate_limited_until().is_some() {
            return false;
        }
        if self.last_check_started.is_some_and(|t| now.saturating_sub(t) < AUTO_RETRY_SECS) {
            return false;
        }
        let oldest = self.catalog.apps.iter().map(|a| self.feeds.get(&a.id).map_or(0, |f| f.fetched_at)).min().unwrap_or(0);
        now.saturating_sub(oldest) >= hours.saturating_mul(3600)
    }

    /// Updates the user hasn't been told about: installed apps with an update whose version is
    /// newer than the last one notified. Returns `(name, version)` pairs and remembers them (in
    /// `state.json`), so each version is announced once.
    pub fn take_new_updates(&mut self) -> Vec<(String, Version)> {
        let mut fresh = Vec::new();
        for st in self.statuses() {
            if let Status::UpdateAvailable { latest, .. } = st.status
                && self.state.notified.get(&st.id).is_none_or(|v| *v < latest)
            {
                self.state.notified.insert(st.id, latest.clone());
                fresh.push((st.name, latest));
            }
        }
        if !fresh.is_empty()
            && let Some(store) = &self.store
            && let Err(e) = store.save_state(&self.state)
        {
            log::warn!("{e}");
        }
        fresh
    }

    /// Run a command by id and wait for it, background-capable ones included. `params` must be a
    /// JSON object (or null for none). Never panics: a panic that escapes a command anyway is
    /// caught here and reported as [`EngineError::Internal`].
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        let (spec, params) = self.prepare(id, params)?;
        log::debug!("execute {id}");
        catch_unwind(AssertUnwindSafe(|| (spec.run)(self, &params))).unwrap_or_else(|_| {
            log::error!("command `{id}` panicked");
            Err(EngineError::Internal(spec.id.into()))
        })
    }

    /// Find a command, check that it can run, and normalise its params.
    pub(crate) fn prepare(&self, id: &str, params: Value) -> Result<(&'static CommandSpec, Value)> {
        let spec = find(id).ok_or_else(|| EngineError::UnknownCommand(echo(id)))?;
        if let Err(reason) = (spec.enabled)(self) {
            return Err(EngineError::Disabled { id: spec.id.into(), reason });
        }
        let params = match params {
            Value::Null => Value::Object(Default::default()),
            Value::Object(_) => params,
            other => return Err(EngineError::BadParams(format!("params must be a JSON object, got {}", params::kind(&other)))),
        };
        Ok((spec, params))
    }

    /// Why the command can't run now (`None` = it can, or it doesn't exist: see [`find`]).
    pub fn disabled_reason(&self, id: &str) -> Option<String> {
        find(id).and_then(|s| (s.enabled)(self).err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artcraft_toolbox_store::CachedFeed;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("artcraft-toolbox-engine-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const FEED: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

    #[test]
    fn open_loads_settings_inventory_and_cached_feeds() {
        let root = temp("open");
        let store = Store::open(&root).unwrap();
        store.save_settings(&Settings { check_interval_hours: 2, ..Settings::default() }).unwrap();
        store.save_feed("photocraft", &CachedFeed { etag: Some("W/\"e\"".into()), fetched_at: 42, body: FEED.into() }).unwrap();
        let (s, warnings) = Session::open(Catalog::builtin().unwrap(), Some(store), None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.settings().check_interval_hours, 2);
        let f = s.feed("photocraft").unwrap();
        assert_eq!((f.fetched_at, f.etag.as_deref(), f.releases.len()), (42, Some("W/\"e\""), 4));
        assert_eq!(s.last_checked(), Some(42));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_files_are_warnings_not_failures() {
        let root = temp("corrupt");
        let store = Store::open(&root).unwrap();
        std::fs::write(root.join("settings.json"), "{broken").unwrap();
        std::fs::write(root.join("inventory.json"), "[not an inventory]").unwrap();
        std::fs::write(root.join("feeds/photocraft.json"), "garbage").unwrap();
        let (mut s, warnings) = Session::open(Catalog::builtin().unwrap(), Some(store), None);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("default settings") && warnings[0].contains("settings.json.corrupt"), "{}", warnings[0]);
        assert!(root.join("settings.json.corrupt").exists());
        assert!(warnings[1].contains("left untouched"), "{}", warnings[1]);
        assert!(s.feed("photocraft").is_none());
        // The unreadable inventory is never written over.
        assert!(matches!(s.save_inventory(), Err(EngineError::Locked(_))));
        assert_eq!(std::fs::read_to_string(root.join("inventory.json")).unwrap(), "[not an inventory]");
        // Settings still save (the corrupt one was backed up).
        s.execute("settings.set", serde_json::json!({"channel": "prerelease"})).unwrap();
        assert!(std::fs::read_to_string(root.join("settings.json")).unwrap().contains("prerelease"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_due_follows_the_interval() {
        fn t0() -> u64 {
            1_000_000
        }
        struct Never;
        impl Transport for Never {
            fn get(&self, _: &artcraft_toolbox_net::Request<'_>) -> std::result::Result<artcraft_toolbox_net::Response, artcraft_toolbox_net::NetError> {
                Err(artcraft_toolbox_net::NetError::Timeout)
            }
        }
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Never)));
        s.set_clock(t0);
        assert!(s.check_due(), "never checked");
        let ids: Vec<String> = s.catalog().apps.iter().map(|a| a.id.clone()).collect();
        for id in &ids {
            s.feeds.insert(id.clone(), Feed { releases: Vec::new(), fetched_at: t0() - 3600, etag: None });
        }
        assert!(!s.check_due(), "checked an hour ago, interval 6 h");
        if let Some(f) = s.feeds.get_mut("photocraft") {
            f.fetched_at = t0() - 7 * 3600;
        }
        assert!(s.check_due(), "one app is older than the interval");
        s.settings.check_interval_hours = 0;
        assert!(!s.check_due(), "0 turns automatic checks off");
        s.settings.check_interval_hours = 6;
        s.rate_limited_until = Some(t0() + 60);
        assert!(!s.check_due(), "not while GitHub refuses requests");
        assert!(!Session::new().unwrap().check_due(), "never without network access");
        // A check that just started (even one that failed) holds automatic ones back.
        s.rate_limited_until = None;
        if let Some(f) = s.feeds.get_mut("photocraft") {
            f.fetched_at = 0;
        }
        s.last_check_started = Some(t0() - 60);
        assert!(!s.check_due(), "within AUTO_RETRY_SECS of the last attempt");
        s.last_check_started = Some(t0() - AUTO_RETRY_SECS);
        assert!(s.check_due());
    }

    #[test]
    fn each_update_is_announced_once_even_across_restarts() {
        use artcraft_toolbox_model::{Installation, PackageKind};
        let root = temp("notify");
        let open = || {
            let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(Store::open(&root).unwrap()), None);
            s.set_host(Target::from_consts("linux", "x86_64"));
            s.ingest_releases("photocraft", FEED, 1).unwrap();
            let mut inv = Inventory::default();
            inv.record(Installation {
                app: "photocraft".into(),
                version: Version::new(0, 3, 0),
                kind: PackageKind::AppImage,
                path: "/x".into(),
                installed_at: 0,
                active: true,
            })
            .unwrap();
            s.set_inventory(inv);
            s
        };
        let mut s = open();
        assert_eq!(s.take_new_updates(), [("PhotoCraft".to_string(), Version::new(0, 5, 0))]);
        assert!(s.take_new_updates().is_empty(), "announced once");
        assert!(open().take_new_updates().is_empty(), "and not again after a restart");
        // Apps that aren't installed have no "update" to announce.
        assert!(!s.statuses().iter().any(|r| r.id == "vectorcraft" && matches!(r.status, Status::UpdateAvailable { .. })));
        let _ = std::fs::remove_dir_all(&root);
    }
}
