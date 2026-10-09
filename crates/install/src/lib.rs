//! Installs, uninstalls and launches Crafting Apps (L3), one package kind per OS:
//!
//! - **macOS:** the DMG is mounted read-only (`hdiutil`), its one `.app` checked (bundle id) and
//!   copied with `ditto` into the apps folder (`~/Applications`) through a staging name, and the
//!   DMG detached, whatever happens.
//! - **Windows:** the portable zip is extracted into `<apps>\<Name>\<version>` (unsafe paths,
//!   symbolic links, duplicates and oversized archives refused), its `portable.txt` deleted so the
//!   app keeps its data in `%APPDATA%` across versions, and a Start Menu shortcut made.
//! - **Linux:** the AppImage (checked to be one) is placed at `<apps>/<id>/<version>/<id>.AppImage`,
//!   made executable, with a desktop entry and icon under `~/.local/share`.
//!
//! Everything goes in under a staging name and is renamed into place, so an interrupted install
//! leaves nothing half-done. Nothing that already exists is ever replaced: an app installed by
//! hand stays untouched ([`Error::Exists`]). Uninstall removes exactly what install made, after
//! checking the path still looks like it. Packages must be verified (SHA-256, [`sha256_file`])
//! before they get here; this crate doesn't download.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod appimage;
mod dmg;
pub mod layout;
mod portable;
mod process;

use std::io::Read;
use std::path::{Path, PathBuf};

use artcraft_toolbox_release::{PackageKind, Sha256};
use sha2::Digest;

pub use layout::Layout;
pub use portable::{Limits, extract as extract_zip};
pub use process::{is_running, launch};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(String),
    #[error("{} already exists; the toolbox doesn't replace what it didn't install", .0.display())]
    Exists(PathBuf),
    #[error("the package is not usable: {0}")]
    BadPackage(String),
    #[error("installing {0:?} packages isn't supported on this system")]
    Unsupported(PackageKind),
    #[error("{0}")]
    Refused(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(what: impl std::fmt::Display, e: &std::io::Error) -> Error {
    Error::Io(format!("{what}: {e}"))
}

/// What the installers need to know about the app. Every field is checked before it names a
/// path ([`AppInfo::validate`]).
#[derive(Debug, Clone, Copy)]
pub struct AppInfo<'a> {
    /// `photocraft`: file names, the executable's name.
    pub id: &'a str,
    /// `PhotoCraft`: folder and shortcut names.
    pub name: &'a str,
    /// `ai.storyteller.photocraft`: checked against the macOS bundle; desktop entry and icon names.
    pub bundle_id: &'a str,
    pub tagline: &'a str,
    /// `0.5.0`: the version folder.
    pub version: &'a str,
    /// The app's icon (PNG), for the Linux desktop entry.
    pub icon_png: Option<&'a [u8]>,
}

impl AppInfo<'_> {
    pub fn validate(&self) -> Result<()> {
        let bad = |what: &str, v: &str| Err(Error::Refused(format!("{what} `{}` can't be used in a file name", v.chars().take(40).collect::<String>())));
        let slug =
            |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !s.starts_with('-');
        if !slug(self.id) {
            return bad("app id", self.id);
        }
        if self.name.is_empty()
            || self.name.len() > 64
            || !self.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-')
            || self.name.starts_with([' ', '-', '.'])
        {
            return bad("app name", self.name);
        }
        if self.bundle_id.is_empty()
            || self.bundle_id.len() > 128
            || !self.bundle_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || self.bundle_id.starts_with('.')
        {
            return bad("bundle id", self.bundle_id);
        }
        if self.version.is_empty()
            || self.version.len() > 64
            || !self.version.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || self.version.starts_with(['.', '-'])
        {
            return bad("version", self.version);
        }
        Ok(())
    }
}

/// Install the verified `package` of `kind`. Returns the path to launch and to record: the
/// `.app` bundle, the `.exe`, or the AppImage.
pub fn install(layout: &Layout, app: &AppInfo, kind: PackageKind, package: &Path) -> Result<PathBuf> {
    app.validate()?;
    match kind {
        PackageKind::Dmg if cfg!(target_os = "macos") => dmg::install(layout, app, package),
        PackageKind::PortableZip => portable::install(layout, app, package, Limits::default()),
        PackageKind::AppImage => appimage::install(layout, app, package),
        other => Err(Error::Unsupported(other)),
    }
}

/// Remove what [`install`] put at `path` (and its shortcut, desktop entry, icon). Refuses paths
/// that don't look like an installation of `app`, and apps that are running.
pub fn uninstall(layout: &Layout, app: &AppInfo, kind: PackageKind, path: &Path) -> Result<()> {
    app.validate()?;
    if is_running(path) {
        return Err(Error::Refused(format!("{} is running; quit it first", app.name)));
    }
    match kind {
        PackageKind::Dmg => dmg::uninstall(app, path),
        PackageKind::PortableZip => portable::uninstall(layout, app, path),
        PackageKind::AppImage => appimage::uninstall(layout, app, path),
        other => Err(Error::Unsupported(other)),
    }
}

/// The SHA-256 of a file, read in chunks.
pub fn sha256_file(path: &Path) -> Result<Sha256> {
    let mut file = std::fs::File::open(path).map_err(|e| io(path.display(), &e))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| io(path.display(), &e))?;
        let Some(chunk) = buf.get(..n) else { break };
        if n == 0 {
            break;
        }
        hasher.update(chunk);
    }
    Ok(Sha256(hasher.finalize().into()))
}

/// A unique staging name beside `final_name` in `dir`: `.<final>.toolbox-<pid>-<n>`.
pub(crate) fn staging(dir: &Path, final_name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!(".{final_name}.toolbox-{}-{n}", std::process::id()))
}

/// Does anything exist at `path` (a dangling symbolic link included)?
pub(crate) fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn info() -> AppInfo<'static> {
        AppInfo { id: "fakecraft", name: "FakeCraft", bundle_id: "ai.storyteller.fakecraft", tagline: "Testing", version: "1.2.0", icon_png: None }
    }

    #[test]
    fn app_info_never_names_a_path_outside() {
        assert!(info().validate().is_ok());
        for (field, v) in [
            ("id", "../x"),
            ("id", "A"),
            ("id", ""),
            ("name", "../x"),
            ("name", "a/b"),
            ("name", ".hidden"),
            ("version", "../1"),
            ("version", "1/2"),
            ("bundle", "a/b"),
        ] {
            let mut a = info();
            match field {
                "id" => a.id = v,
                "name" => a.name = v,
                "version" => a.version = v,
                _ => a.bundle_id = v,
            }
            assert!(a.validate().is_err(), "{field} = {v}");
        }
    }

    #[test]
    fn sha256_of_a_file() {
        let p = std::env::temp_dir().join(format!("artcraft-toolbox-sha-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap().to_hex(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&p);
        assert!(sha256_file(&p).is_err());
    }
}
