//! ArtCraft Toolbox's own files (L3): where they live ([`dirs`]), how they are written
//! ([`atomic::atomic_write`]), and the [`Store`] that reads and writes each of them.
//!
//! ```text
//! <data dir>/settings.json        Settings
//! <data dir>/inventory.json       Inventory: what is installed where (the only record of it)
//! <data dir>/state.json           what the toolbox remembers (notifications already shown)
//! <data dir>/feeds/<app>.json     the last release feed per app, with its ETag
//! <data dir>/remote/catalog.json  the last signed remote catalog (its envelope), with its ETag
//! <data dir>/remote/feed.json     the aggregated feed's ETag and fetch time (its apps are in feeds/)
//! <data dir>/icons/<app>.png      the app's icon, and <app>.json with its ETag and fetch time
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

use artcraft_toolbox_model::{Inventory, Settings, ToolboxState};
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";
pub const INVENTORY_FILE: &str = "inventory.json";
pub const STATE_FILE: &str = "state.json";
pub const FEEDS_DIR: &str = "feeds";
/// The publisher's signed documents: `catalog` and `feed` (docs/architecture.md § 12).
pub const REMOTE_DIR: &str = "remote";
pub const ICONS_DIR: &str = "icons";
/// Largest icon accepted (a 128 px PNG is a few KB).
pub const MAX_ICON_BYTES: usize = 512 * 1024;
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

/// When an app's cached icon was fetched, and its ETag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconMeta {
    pub etag: Option<String>,
    pub fetched_at: u64,
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
        for dir in [root.clone(), root.join(FEEDS_DIR), root.join(REMOTE_DIR), root.join(ICONS_DIR)] {
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

    /// `<dir>/<app>.<ext>`. Catalog ids are already validated slugs; this keeps a hostile id
    /// from ever naming a path.
    fn app_path(&self, dir: &str, app: &str, ext: &str) -> Result<PathBuf> {
        let ok = (1..=64).contains(&app.len()) && app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !app.starts_with('-');
        if !ok {
            return Err(Error::BadName(app.chars().take(40).collect()));
        }
        Ok(self.root.join(dir).join(format!("{app}.{ext}")))
    }

    fn feed_path(&self, app: &str) -> Result<PathBuf> {
        self.app_path(FEEDS_DIR, app, "json")
    }

    /// `None` before anything was remembered.
    pub fn load_state(&self) -> Result<Option<ToolboxState>> {
        let path = self.root.join(STATE_FILE);
        let Some(text) = read_capped(&path, artcraft_toolbox_model::MAX_FILE_BYTES)? else { return Ok(None) };
        ToolboxState::from_json(&text).map(Some).map_err(|e| Error::Corrupt { path, reason: e.to_string() })
    }

    pub fn save_state(&self, state: &ToolboxState) -> Result<()> {
        let path = self.root.join(STATE_FILE);
        let json = state.to_json().map_err(|e| Error::Io { path: path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&path, json.as_bytes()).map_err(|e| io_err(&path, &e))
    }

    /// `app`'s cached icon (PNG bytes, unchecked) and its metadata; `None` if there is none or
    /// half of it is missing.
    pub fn load_icon(&self, app: &str) -> Result<Option<(Vec<u8>, IconMeta)>> {
        let meta_path = self.app_path(ICONS_DIR, app, "json")?;
        let png_path = self.app_path(ICONS_DIR, app, "png")?;
        let Some(meta) = read_capped(&meta_path, 4096)? else { return Ok(None) };
        let meta: IconMeta = serde_json::from_str(&meta).map_err(|e| Error::Corrupt { path: meta_path, reason: e.to_string() })?;
        let Some(png) = read_bytes_capped(&png_path, MAX_ICON_BYTES)? else { return Ok(None) };
        Ok(Some((png, meta)))
    }

    /// Store an icon: the image first, the metadata last, so a crash in between leaves no
    /// metadata and the icon is fetched again.
    pub fn save_icon(&self, app: &str, png: &[u8], meta: &IconMeta) -> Result<()> {
        if png.len() > MAX_ICON_BYTES {
            return Err(Error::TooLarge { path: PathBuf::from(format!("{app}.png")), len: png.len(), limit: MAX_ICON_BYTES });
        }
        let png_path = self.app_path(ICONS_DIR, app, "png")?;
        let meta_path = self.app_path(ICONS_DIR, app, "json")?;
        atomic::atomic_write(&png_path, png).map_err(|e| io_err(&png_path, &e))?;
        let json = serde_json::to_string(meta).map_err(|e| Error::Io { path: meta_path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&meta_path, json.as_bytes()).map_err(|e| io_err(&meta_path, &e))
    }

    /// Record an icon check that found it unchanged (`304`).
    pub fn touch_icon(&self, app: &str, fetched_at: u64) -> Result<()> {
        let meta_path = self.app_path(ICONS_DIR, app, "json")?;
        let Some(text) = read_capped(&meta_path, 4096)? else { return Ok(()) };
        let meta: IconMeta = serde_json::from_str(&text).map_err(|e| Error::Corrupt { path: meta_path.clone(), reason: e.to_string() })?;
        let json = serde_json::to_string(&IconMeta { fetched_at, ..meta }).map_err(|e| Error::Io { path: meta_path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&meta_path, json.as_bytes()).map_err(|e| io_err(&meta_path, &e))
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

    fn remote_path(&self, name: &str) -> Result<PathBuf> {
        self.app_path(REMOTE_DIR, name, "json")
    }

    /// A signed remote document (`catalog`, `feed`) as last fetched; `None` when never.
    pub fn load_remote(&self, name: &str) -> Result<Option<CachedFeed>> {
        let path = self.remote_path(name)?;
        let Some(text) = read_capped(&path, MAX_FEED_FILE_BYTES)? else { return Ok(None) };
        serde_json::from_str(&text).map(Some).map_err(|e| Error::Corrupt { path, reason: e.to_string() })
    }

    pub fn save_remote(&self, name: &str, doc: &CachedFeed) -> Result<()> {
        let path = self.remote_path(name)?;
        let json = serde_json::to_string(doc).map_err(|e| Error::Io { path: path.clone(), message: e.to_string() })?;
        atomic::atomic_write(&path, json.as_bytes()).map_err(|e| io_err(&path, &e))
    }

    /// Record a check that found the document unchanged (`304`): only the time moves.
    pub fn touch_remote(&self, name: &str, fetched_at: u64) -> Result<()> {
        match self.load_remote(name)? {
            Some(doc) => self.save_remote(name, &CachedFeed { fetched_at, ..doc }),
            None => Ok(()),
        }
    }
}

/// Read a UTF-8 file of at most `limit` bytes; `None` if it doesn't exist.
fn read_capped(path: &Path, limit: usize) -> Result<Option<String>> {
    let Some(bytes) = read_bytes_capped(path, limit)? else { return Ok(None) };
    String::from_utf8(bytes).map(Some).map_err(|_| Error::Corrupt { path: path.to_path_buf(), reason: "not UTF-8 text".into() })
}

/// Read a file of at most `limit` bytes; `None` if it doesn't exist.
fn read_bytes_capped(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
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
    Ok(Some(bytes))
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
            trust: None,
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
    fn state_and_icons_round_trip() {
        let root = temp("icons");
        let store = Store::open(&root).unwrap();
        assert!(root.join(ICONS_DIR).is_dir());
        assert_eq!(store.load_state().unwrap(), None);
        let mut state = ToolboxState::default();
        state.notified.insert("photocraft".into(), artcraft_toolbox_model::Version::new(0, 5, 0));
        store.save_state(&state).unwrap();
        assert_eq!(store.load_state().unwrap(), Some(state));

        assert_eq!(store.load_icon("photocraft").unwrap(), None);
        let meta = IconMeta { etag: Some("\"e\"".into()), fetched_at: 7 };
        store.save_icon("photocraft", b"\x89PNG...", &meta).unwrap();
        assert_eq!(store.load_icon("photocraft").unwrap(), Some((b"\x89PNG...".to_vec(), meta.clone())));
        store.touch_icon("photocraft", 9).unwrap();
        assert_eq!(store.load_icon("photocraft").unwrap().map(|(_, m)| m.fetched_at), Some(9));
        // An image without its metadata counts as missing; oversized images are refused.
        std::fs::remove_file(root.join(ICONS_DIR).join("photocraft.json")).unwrap();
        assert_eq!(store.load_icon("photocraft").unwrap(), None);
        assert!(matches!(store.save_icon("photocraft", &vec![0u8; MAX_ICON_BYTES + 1], &meta), Err(Error::TooLarge { .. })));
        assert!(matches!(store.load_icon("../x"), Err(Error::BadName(_))));
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

    #[test]
    fn remote_documents_are_cached_and_touched() {
        let root = std::env::temp_dir().join(format!("artcraft-toolbox-store-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::open(&root).unwrap();
        assert_eq!(store.load_remote("catalog").unwrap(), None);
        let doc = CachedFeed { etag: Some("W/\"c1\"".into()), fetched_at: 5, body: "{\"schema\":1}".into() };
        store.save_remote("catalog", &doc).unwrap();
        assert_eq!(store.load_remote("catalog").unwrap(), Some(doc.clone()));
        store.touch_remote("catalog", 9).unwrap();
        assert_eq!(store.load_remote("catalog").unwrap().unwrap().fetched_at, 9);
        store.touch_remote("feed", 9).unwrap();
        assert_eq!(store.load_remote("feed").unwrap(), None, "touching what was never saved saves nothing");
        assert!(matches!(store.load_remote("../x").unwrap_err(), Error::BadName(_)));
        assert!(root.join("remote/catalog.json").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
