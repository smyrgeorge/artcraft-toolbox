//! ArtCraft Toolbox's own files (L3): where they live ([`dirs`]), how they are written
//! ([`atomic::atomic_write`]), and the [`Store`] that reads and writes each of them.
//!
//! ```text
//! <data dir>/settings.json        Settings
//! <data dir>/inventory.json       Inventory: what is installed where (the only record of it)
//! <data dir>/feeds/<app>.json     the last release feed per app, with its ETag
//! <data dir>/logs/                the desktop app's log files
//! ```
//!
//! Every file is untrusted on read (size-capped, parsed without panics) and replaced atomically on
//! write, so a crash or a full disk never leaves a half-written file.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod atomic;
pub mod dirs;

use std::io::Read;
use std::path::{Path, PathBuf};

use artcraft_toolbox_model::{Inventory, Settings};
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";
pub const INVENTORY_FILE: &str = "inventory.json";
pub const FEEDS_DIR: &str = "feeds";
pub const LOGS_DIR: &str = "logs";
/// Largest cached feed file read back (a feed response is capped at 16 MiB; JSON escaping can
/// grow it).
pub const MAX_FEED_FILE_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{}: {message}", path.display())]
    Io { path: PathBuf, message: String },
    #[error("{} can't be read: {reason}", path.display())]
    Corrupt { path: PathBuf, reason: String },
    #[error("{} is too large ({len} bytes, limit {limit})", path.display())]
    TooLarge { path: PathBuf, len: usize, limit: usize },
    #[error("`{0}` can't be used as a file name")]
    BadName(String),
    #[error("no folder for the toolbox's data: set ARTCRAFT_TOOLBOX_CONFIG_DIR (no HOME or APPDATA found)")]
    NoDataDir,
}

pub type Result<T> = std::result::Result<T, Error>;

fn io_err(path: &Path, e: &std::io::Error) -> Error {
    Error::Io { path: path.to_path_buf(), message: e.to_string() }
}

/// One app's cached release feed: the raw GitHub response, so a newer parser can re-read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedFeed {
    pub etag: Option<String>,
    /// Unix seconds of the last successful check (a 304 counts).
    pub fetched_at: u64,
    pub body: String,
}

/// The toolbox's files under one data directory. Cheap to clone (a path), so jobs can write the
/// feed cache from their worker threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Use (and create) `root` and its subfolders.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        for dir in [root.clone(), root.join(FEEDS_DIR)] {
            std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, &e))?;
        }
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }

    /// `None` when there is no settings file yet.
    pub fn load_settings(&self) -> Result<Option<Settings>> {
        let path = self.root.join(SETTINGS_FILE);
        let Some(text) = read_capped(&path, artcraft_toolbox_model::MAX_FILE_BYTES)? else { return Ok(None) };
        Settings::from_json(&text).map(Some).map_err(|e| Error::Corrupt { path, reason: e.to_string() })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let path = self.root.join(SETTINGS_FILE);
        let json = settings.to_json().map_err(|e| Error::Io { path: path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&path, json.as_bytes()).map_err(|e| io_err(&path, &e))
    }

    /// `None` when nothing has been installed yet.
    pub fn load_inventory(&self) -> Result<Option<Inventory>> {
        let path = self.root.join(INVENTORY_FILE);
        let Some(text) = read_capped(&path, artcraft_toolbox_model::MAX_FILE_BYTES)? else { return Ok(None) };
        Inventory::from_json(&text).map(Some).map_err(|e| Error::Corrupt { path, reason: e.to_string() })
    }

    pub fn save_inventory(&self, inventory: &Inventory) -> Result<()> {
        let path = self.root.join(INVENTORY_FILE);
        let json = inventory.to_json().map_err(|e| Error::Io { path: path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&path, json.as_bytes()).map_err(|e| io_err(&path, &e))
    }

    /// Keep a copy of an unreadable file as `<file>.corrupt` before it is replaced. Returns the copy.
    pub fn back_up(&self, file: &str) -> Result<PathBuf> {
        let from = self.root.join(file);
        let to = self.root.join(format!("{file}.corrupt"));
        std::fs::copy(&from, &to).map_err(|e| io_err(&to, &e))?;
        Ok(to)
    }

    fn feed_path(&self, app: &str) -> Result<PathBuf> {
        // Catalog ids are already validated slugs; this keeps a hostile id from ever naming a path.
        let ok = (1..=64).contains(&app.len()) && app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !app.starts_with('-');
        if !ok {
            return Err(Error::BadName(app.chars().take(40).collect()));
        }
        Ok(self.root.join(FEEDS_DIR).join(format!("{app}.json")))
    }

    /// `None` when `app` was never checked.
    pub fn load_feed(&self, app: &str) -> Result<Option<CachedFeed>> {
        let path = self.feed_path(app)?;
        let Some(text) = read_capped(&path, MAX_FEED_FILE_BYTES)? else { return Ok(None) };
        serde_json::from_str(&text).map(Some).map_err(|e| Error::Corrupt { path, reason: e.to_string() })
    }

    pub fn save_feed(&self, app: &str, feed: &CachedFeed) -> Result<()> {
        let path = self.feed_path(app)?;
        let json = serde_json::to_string(feed).map_err(|e| Error::Io { path: path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&path, json.as_bytes()).map_err(|e| io_err(&path, &e))
    }

    /// Record a check that found the feed unchanged (`304`): only the time moves.
    pub fn touch_feed(&self, app: &str, fetched_at: u64) -> Result<()> {
        match self.load_feed(app)? {
            Some(feed) => self.save_feed(app, &CachedFeed { fetched_at, ..feed }),
            None => Ok(()),
        }
    }
}

/// Read a UTF-8 file of at most `limit` bytes; `None` if it doesn't exist.
fn read_capped(path: &Path, limit: usize) -> Result<Option<String>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path, &e)),
    };
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    file.take(cap).read_to_end(&mut bytes).map_err(|e| io_err(path, &e))?;
    if bytes.len() > limit {
        return Err(Error::TooLarge { path: path.to_path_buf(), len: bytes.len(), limit });
    }
    String::from_utf8(bytes).map(Some).map_err(|_| Error::Corrupt { path: path.to_path_buf(), reason: "not UTF-8 text".into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use artcraft_toolbox_model::{Channel, Installation};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("artcraft-toolbox-store-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn settings_and_inventory_round_trip() {
        let root = temp("roundtrip");
        let store = Store::open(&root).unwrap();
        assert!(root.join(FEEDS_DIR).is_dir());
        assert_eq!(store.load_settings().unwrap(), None);
        assert_eq!(store.load_inventory().unwrap(), None);
        let settings = Settings { channel: Channel::Prerelease, ..Settings::default() };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings().unwrap(), Some(settings));
        let mut inv = Inventory::default();
        inv.record(Installation {
            app: "photocraft".into(),
            version: artcraft_toolbox_model::Version::new(0, 5, 0),
            kind: artcraft_toolbox_model::PackageKind::Dmg,
            path: "/Applications/PhotoCraft.app".into(),
            installed_at: 1,
            active: true,
        })
        .unwrap();
        store.save_inventory(&inv).unwrap();
        assert_eq!(store.load_inventory().unwrap(), Some(inv));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_files_are_errors_and_can_be_backed_up() {
        let root = temp("corrupt");
        let store = Store::open(&root).unwrap();
        std::fs::write(root.join(SETTINGS_FILE), "{not json").unwrap();
        std::fs::write(root.join(INVENTORY_FILE), [0xff, 0xfe]).unwrap();
        assert!(matches!(store.load_settings(), Err(Error::Corrupt { .. })));
        assert!(matches!(store.load_inventory(), Err(Error::Corrupt { .. })));
        let copy = store.back_up(SETTINGS_FILE).unwrap();
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_files_are_refused() {
        let root = temp("big");
        let store = Store::open(&root).unwrap();
        std::fs::write(root.join(SETTINGS_FILE), " ".repeat(artcraft_toolbox_model::MAX_FILE_BYTES + 1)).unwrap();
        assert!(matches!(store.load_settings(), Err(Error::TooLarge { .. })));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn feeds_round_trip_and_touch() {
        let root = temp("feeds");
        let store = Store::open(&root).unwrap();
        assert_eq!(store.load_feed("photocraft").unwrap(), None);
        store.touch_feed("photocraft", 9).unwrap();
        assert_eq!(store.load_feed("photocraft").unwrap(), None, "touching a missing feed creates nothing");
        let feed = CachedFeed { etag: Some("W/\"x\"".into()), fetched_at: 5, body: "[]".into() };
        store.save_feed("photocraft", &feed).unwrap();
        store.touch_feed("photocraft", 9).unwrap();
        assert_eq!(store.load_feed("photocraft").unwrap(), Some(CachedFeed { fetched_at: 9, ..feed }));
        std::fs::write(root.join(FEEDS_DIR).join("vectorcraft.json"), "garbage").unwrap();
        assert!(matches!(store.load_feed("vectorcraft"), Err(Error::Corrupt { .. })));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn app_ids_never_name_paths_outside_the_feed_folder() {
        let store = Store::open(temp("names")).unwrap();
        for bad in ["", "../settings", "a/b", "a\\b", "Photo", "-x", "x.json", &"a".repeat(65), "photo craft", "nul\0"] {
            assert!(matches!(store.load_feed(bad), Err(Error::BadName(_))), "{bad:?}");
            assert!(matches!(store.save_feed(bad, &CachedFeed { etag: None, fetched_at: 0, body: String::new() }), Err(Error::BadName(_))), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn an_unusable_root_is_an_error() {
        let root = temp("blocked");
        std::fs::write(&root, "a file where the folder should be").unwrap();
        assert!(matches!(Store::open(&root), Err(Error::Io { .. })));
        let _ = std::fs::remove_file(&root);
    }
}
