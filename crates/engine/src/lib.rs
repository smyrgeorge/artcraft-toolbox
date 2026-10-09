//! The ArtCraft Toolbox engine: where every behaviour lives.
//!
//! A [`Session`] holds the catalog, what is installed, the settings and the release feeds. Every
//! user-visible action is a command ([`command_specs`]) with an id, a params doc and a `run`
//! function; the desktop UI, the CLI and (later) the control channel and MCP all dispatch the
//! same commands by id through [`Session::execute`]. No UI toolkit below L6 (`cargo xtask layers`).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod build_info;
mod catalog_cmds;
mod commands;
mod params;
mod settings_cmds;
mod status_cmds;

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use artcraft_toolbox_catalog::Catalog;
use artcraft_toolbox_feed::Release;
use artcraft_toolbox_model::{Inventory, Settings};

use serde::Serialize;
use serde_json::Value;

pub use artcraft_toolbox_feed::Status;
pub use artcraft_toolbox_release::{Target, Version};
pub use commands::{CommandSpec, command_specs, find};

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
    #[error(transparent)]
    Catalog(#[from] artcraft_toolbox_catalog::Error),
    #[error(transparent)]
    Feed(#[from] artcraft_toolbox_feed::Error),
    #[error(transparent)]
    Model(#[from] artcraft_toolbox_model::Error),
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
    /// Unix seconds.
    pub fetched_at: u64,
}

/// One row of the app list: what the UI shows and `apps.status` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub status: Status,
}

pub struct Session {
    catalog: Catalog,
    inventory: Inventory,
    settings: Settings,
    feeds: BTreeMap<String, Feed>,
    host: Option<Target>,
}

impl Session {
    /// A session over the built-in catalog, on this machine, with nothing installed.
    pub fn new() -> Result<Session> {
        Ok(Session::with_catalog(Catalog::builtin()?))
    }

    pub fn with_catalog(catalog: Catalog) -> Session {
        Session { catalog, inventory: Inventory::default(), settings: Settings::default(), feeds: BTreeMap::new(), host: Target::host() }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Replace the inventory (the app loads it from its data directory at start).
    pub fn set_inventory(&mut self, inventory: Inventory) {
        self.inventory = inventory;
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    /// The machine releases are chosen for; `None` on a platform no craft ships for.
    pub fn host(&self) -> Option<Target> {
        self.host
    }

    /// Choose releases for another machine (tests, and agents asking "what would Windows get?").
    pub fn set_host(&mut self, host: Option<Target>) {
        self.host = host;
    }

    pub fn feed(&self, app: &str) -> Option<&Feed> {
        self.feeds.get(app)
    }

    /// Take a GitHub "list releases" response for `app`. Returns how many releases it held.
    pub fn ingest_releases(&mut self, app: &str, json: &str, fetched_at: u64) -> Result<usize> {
        let entry = self.catalog.get(app).ok_or_else(|| EngineError::UnknownApp(echo(app)))?;
        let releases = artcraft_toolbox_feed::parse_releases(json, &entry.slugs())?;
        let n = releases.len();
        self.feeds.insert(app.to_string(), Feed { releases, fetched_at });
        Ok(n)
    }

    pub fn app_status(&self, app: &str) -> Result<AppStatus> {
        let entry = self.catalog.get(app).ok_or_else(|| EngineError::UnknownApp(echo(app)))?;
        let installed = self.inventory.current(app).map(|i| &i.version);
        let releases = self.feeds.get(app).map(|f| f.releases.as_slice());
        let status = artcraft_toolbox_feed::status(installed, releases, self.settings.channel, self.host);
        Ok(AppStatus { id: entry.id.clone(), name: entry.name.clone(), tagline: entry.tagline.clone(), status })
    }

    /// Every catalog app, in catalog order.
    pub fn statuses(&self) -> Vec<AppStatus> {
        self.catalog.apps.iter().filter_map(|a| self.app_status(&a.id).ok()).collect()
    }

    /// Run a command by id. `params` must be a JSON object (or null for none). Never panics: a
    /// panic that escapes a command anyway is caught here and reported as [`EngineError::Internal`].
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        let spec = find(id).ok_or_else(|| EngineError::UnknownCommand(echo(id)))?;
        if let Err(reason) = (spec.enabled)(self) {
            return Err(EngineError::Disabled { id: spec.id.into(), reason });
        }
        let params = match params {
            Value::Null => Value::Object(Default::default()),
            Value::Object(_) => params,
            other => return Err(EngineError::BadParams(format!("params must be a JSON object, got {}", params::kind(&other)))),
        };
        log::debug!("execute {id}");
        catch_unwind(AssertUnwindSafe(|| (spec.run)(self, &params))).unwrap_or_else(|_| {
            log::error!("command `{id}` panicked");
            Err(EngineError::Internal(spec.id.into()))
        })
    }

    /// Why the command can't run now (`None` = it can, or it doesn't exist: see [`find`]).
    pub fn disabled_reason(&self, id: &str) -> Option<String> {
        find(id).and_then(|s| (s.enabled)(self).err())
    }
}
