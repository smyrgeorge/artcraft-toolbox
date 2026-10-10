//! Installs, updates, rolls back, uninstalls and launches Crafting Apps (L3), one package kind per
//! OS. A version is first **placed** (put on disk, nothing else changes), then **activated** (made
//! the one the user opens); several versions can be kept, and rollback is activating another.
//!
//! - **macOS:** the DMG is mounted read-only (`hdiutil`), its one `.app` checked (bundle id) and
//!   copied with `ditto` into `<kept>/<id>/<version>/`, and the DMG detached, whatever happens.
//!   Activating moves it to the apps folder (`~/Applications`), moving the active one out to its
//!   own version folder first (`dmg`).
//! - **Windows:** the portable zip is extracted into `<apps>\<Name>\<version>` (unsafe paths,
//!   symbolic links, duplicates and oversized archives refused), its `portable.txt` deleted so the
//!   app keeps its data in `%APPDATA%` across versions. Activating points the Start Menu shortcut
//!   at it.
//! - **Linux:** the AppImage (checked to be one) is placed at `<apps>/<id>/<version>/<id>.AppImage`
//!   and made executable. Activating writes the desktop entry and icon under `~/.local/share`.
//!
//! Everything goes in under a staging name and is renamed into place, so an interrupted install
//! leaves nothing half-done. Nothing the toolbox didn't put there is ever replaced: an app
//! installed by hand stays untouched ([`Error::Exists`]) until it is adopted ([`find_unmanaged`]).
//! Removal checks the path still looks like what install made, and refuses a running app.
//! Packages must be verified (SHA-256, [`sha256_file`]) before they get here; this crate doesn't
//! download. [`verify_signature`] reports the platform signature of a placed version.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod appimage;
mod dmg;
pub mod layout;
mod portable;
mod process;
pub mod selfupdate;
pub mod trust;

use std::io::Read;
use std::path::{Path, PathBuf};

use artcraft_toolbox_release::{PackageKind, Sha256};
use sha2::Digest;

pub use dmg::bundle_version;
pub use layout::Layout;
pub use portable::{Limits, extract as extract_zip};
pub use process::{MANAGED_ENV, is_running, launch};
pub use trust::verify as verify_signature;

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

/// The active version before an activation: where it is and which version it is (`app.version`
/// is the one being activated).
#[derive(Debug, Clone, Copy)]
pub struct Current<'a> {
    pub path: &'a Path,
    pub version: &'a str,
}

/// Where things are after an activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activated {
    /// The path to launch and record for the activated version.
    pub active: PathBuf,
    /// Where the previously active version went, when it had to move (macOS).
    pub previous: Option<PathBuf>,
}

/// A version found on disk ([`find_unmanaged`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// As the app says (macOS `CFBundleShortVersionString`) or as its folder is named.
    pub version: String,
}

/// Put the verified `package` of `kind` on disk as version `app.version`, without activating it.
/// Returns its path: the `.app` bundle, the `.exe`, or the AppImage.
pub fn place(layout: &Layout, app: &AppInfo, kind: PackageKind, package: &Path) -> Result<PathBuf> {
    app.validate()?;
    match kind {
        PackageKind::Dmg if cfg!(target_os = "macos") => dmg::place(layout, app, package),
        PackageKind::PortableZip => portable::place(layout, app, package, Limits::default()),
        PackageKind::AppImage => appimage::place(layout, app, package),
        other => Err(Error::Unsupported(other)),
    }
}

/// Make the placed or kept version at `path` (version `app.version`) the active one. `current` is
/// the active version until now. Refuses when that has to move and is running (macOS).
pub fn activate(layout: &Layout, app: &AppInfo, kind: PackageKind, path: &Path, current: Option<Current>) -> Result<Activated> {
    app.validate()?;
    if let Some(cur) = current {
        AppInfo { version: cur.version, ..*app }.validate()?;
    }
    match kind {
        PackageKind::Dmg if cfg!(target_os = "macos") => dmg::activate(layout, app, path, current),
        PackageKind::PortableZip => Ok(portable::activate(layout, app, path)),
        PackageKind::AppImage => Ok(appimage::activate(layout, app, path)),
        other => Err(Error::Unsupported(other)),
    }
}

/// Place and activate: the first install of an app. A placed version that can't be activated
/// (its target exists) is removed again.
pub fn install(layout: &Layout, app: &AppInfo, kind: PackageKind, package: &Path) -> Result<PathBuf> {
    let placed = place(layout, app, kind, package)?;
    match activate(layout, app, kind, &placed, None) {
        Ok(a) => Ok(a.active),
        Err(e) => {
            let _ = remove_version(layout, app, kind, &placed);
            Err(e)
        }
    }
}

/// Remove an inactive version (version `app.version`) at `path`: a kept bundle on macOS, a version
/// folder elsewhere. Shortcuts and desktop entries stay (they point at the active version).
pub fn remove_version(layout: &Layout, app: &AppInfo, kind: PackageKind, path: &Path) -> Result<()> {
    app.validate()?;
    match kind {
        PackageKind::Dmg => dmg::remove_version(layout, app, path),
        PackageKind::PortableZip => portable::remove_version(app, path),
        PackageKind::AppImage => appimage::remove_version(app, path),
        other => Err(Error::Unsupported(other)),
    }
}

/// Remove the active version at `path` (and its shortcut, desktop entry, icon). Refuses paths
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

/// Versions of `app` on disk where the toolbox would install them (`app.version` is ignored):
/// what was installed by hand, or by a toolbox whose inventory was lost. The caller leaves out
/// the ones it already knows.
pub fn find_unmanaged(layout: &Layout, app: &AppInfo, kind: PackageKind) -> Vec<Found> {
    if (AppInfo { version: "0", ..*app }).validate().is_err() {
        return Vec::new();
    }
    match kind {
        PackageKind::Dmg if cfg!(target_os = "macos") => dmg::find(layout, app),
        PackageKind::PortableZip => portable::find(layout, app),
        PackageKind::AppImage => appimage::find(layout, app),
        _ => Vec::new(),
    }
}

/// The sub-folders of `dir` named like a version (`<dir>/<version>`), at most 64 of them.
pub(crate) fn version_dirs(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let ok = !name.is_empty()
                && name.len() <= 64
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                && !name.starts_with(['.', '-']);
            ok.then(|| (e.path(), name))
        })
        .take(64)
        .collect()
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

/// Windows PowerShell running `script`: no profile, no prompts, and no `PSModulePath` inherited
/// from PowerShell 7 (a terminal or CI runner of that version would make Windows PowerShell load
/// PowerShell 7's modules, which it can't).
pub(crate) fn powershell(script: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]).env_remove("PSModulePath");
    cmd
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
