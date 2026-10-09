//! macOS DMGs: `<id>-<v>-macos-universal.dmg`, holding `<Name>.app` and an `Applications` link.
//!
//! The image is attached read-only, without Finder and at a private mount point; the one `.app`
//! at its root must carry the catalog's bundle id; it is copied with `ditto` (which keeps the code
//! signature, extended attributes and permissions) and the image detached whatever happened.
//!
//! Only one version can be `~/Applications/<Name>.app`, the active one. The others are kept in
//! `<kept>/<id>/<version>/<Name>.app` (the toolbox's `versions.noindex` folder: Spotlight skips it,
//! so Launchpad and "Open With" never show duplicates). A new version is placed there first and
//! checked; activating it moves the active bundle out to its own version folder and the new one
//! in, and moves the old one back if that fails. A `<Name>.app` the toolbox didn't put there is
//! never replaced.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Activated, AppInfo, Current, Error, Found, Layout, Result, io, is_running, occupied, staging};

/// Copy the image's app to `<kept>/<id>/<version>/<Name>.app`.
pub(crate) fn place(layout: &Layout, app: &AppInfo, package: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(&layout.downloads).map_err(|e| io(layout.downloads.display(), &e))?;
    let mount = staging(&layout.downloads, "mount");
    std::fs::create_dir(&mount).map_err(|e| io(mount.display(), &e))?;
    let _detach = Detach(mount.clone());
    run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(package), "hdiutil attach")?;
    let bundle = find_app(&mount)?;
    let id = bundle_id(&bundle)?;
    if id != app.bundle_id {
        return Err(Error::BadPackage(format!("it holds {id}, not {}", app.bundle_id)));
    }
    let name = bundle.file_name().and_then(|n| n.to_str()).ok_or_else(|| Error::BadPackage("its app has no usable name".into()))?.to_string();
    let dir = layout.kept.join(app.id).join(app.version);
    std::fs::create_dir_all(&dir).map_err(|e| io(dir.display(), &e))?;
    let dest = dir.join(&name);
    let stage = staging(&dir, &name);
    let copied = run(Command::new("ditto").arg(&bundle).arg(&stage), "ditto").and_then(|()| {
        // The kept folder is the toolbox's own: a leftover of this version from an interrupted
        // update is replaced.
        if occupied(&dest) {
            remove_bundle(app, &dest)?;
        }
        std::fs::rename(&stage, &dest).map_err(|e| io(dest.display(), &e))
    });
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    Ok(dest)
}

/// Make the bundle at `path` (a placed or kept version) the one in the apps folder, moving the
/// `current` active one (if it is there) out to its version folder in the kept tree.
pub(crate) fn activate(layout: &Layout, app: &AppInfo, path: &Path, current: Option<Current>) -> Result<Activated> {
    let name = path.file_name().ok_or_else(|| Error::Refused(format!("{} is not an app bundle", path.display())))?;
    let target = layout.apps.join(name);
    if path == target {
        return Ok(Activated { active: target, previous: None });
    }
    std::fs::create_dir_all(&layout.apps).map_err(|e| io(layout.apps.display(), &e))?;
    // (where the active bundle was, where it went)
    let moved_out = match current {
        Some(cur) if cur.path.parent() == Some(layout.apps.as_path()) && occupied(cur.path) => {
            if is_running(cur.path) {
                return Err(Error::Refused(format!("{} is running; quit it first", app.name)));
            }
            let dir = layout.kept.join(app.id).join(cur.version);
            std::fs::create_dir_all(&dir).map_err(|e| io(dir.display(), &e))?;
            let keep = dir.join(cur.path.file_name().unwrap_or(name));
            if occupied(&keep) {
                remove_bundle(app, &keep)?;
            }
            move_bundle(cur.path, &keep)?;
            Some((cur.path.to_path_buf(), keep))
        }
        _ => None,
    };
    let restore = |moved: &Option<(PathBuf, PathBuf)>| {
        if let Some((was, keep)) = moved {
            let _ = move_bundle(keep, was);
        }
    };
    if occupied(&target) {
        restore(&moved_out);
        return Err(Error::Exists(target));
    }
    if let Err(e) = move_bundle(path, &target) {
        restore(&moved_out);
        return Err(e);
    }
    // The new version's own (now empty) folders in the kept tree.
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(Activated { active: target, previous: moved_out.map(|(_, keep)| keep) })
}

/// Rename, or copy and delete when `to` is on another volume (a custom `installDir`).
fn move_bundle(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let stage = staging(to.parent().unwrap_or(to), "move");
            run(Command::new("ditto").arg(from).arg(&stage), "ditto")
                .and_then(|()| std::fs::rename(&stage, to).map_err(|e| io(to.display(), &e)))
                .inspect_err(|_| {
                    let _ = std::fs::remove_dir_all(&stage);
                })?;
            std::fs::remove_dir_all(from).map_err(|e| io(from.display(), &e))
        }
        Err(e) => Err(io(to.display(), &e)),
    }
}

/// Remove an inactive version kept in the toolbox's folder.
pub(crate) fn remove_version(layout: &Layout, app: &AppInfo, bundle: &Path) -> Result<()> {
    if !bundle.starts_with(&layout.kept) {
        return Err(Error::Refused(format!("{} is not a kept version; not removing it", bundle.display())));
    }
    if !occupied(bundle) {
        return Ok(());
    }
    remove_bundle(app, bundle)?;
    if let Some(dir) = bundle.parent() {
        let _ = std::fs::remove_dir(dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(())
}

/// The app in the apps folder, if one with the catalog's bundle id is there.
pub(crate) fn find(layout: &Layout, app: &AppInfo) -> Vec<Found> {
    let path = layout.apps.join(format!("{}.app", app.name));
    let ours = path.is_dir() && bundle_id(&path).is_ok_and(|id| id == app.bundle_id);
    match (ours, bundle_version(&path)) {
        (true, Some(version)) => vec![Found { path, version }],
        _ => Vec::new(),
    }
}

/// The single `.app` at the image's root.
fn find_app(mount: &Path) -> Result<PathBuf> {
    let entries = std::fs::read_dir(mount).map_err(|e| io(mount.display(), &e))?;
    let apps: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path().extension().is_some_and(|x| x == "app"))
        .map(|e| e.path())
        .collect();
    match apps.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Error::BadPackage("there is no .app in the disk image".into())),
        _ => Err(Error::BadPackage("there is more than one .app in the disk image".into())),
    }
}

/// One key of a bundle's `Info.plist` (through `plutil`, part of macOS).
fn plist_value(bundle: &Path, key: &str) -> Option<String> {
    let plist = bundle.join("Contents").join("Info.plist");
    let out = Command::new("plutil").args(["-extract", key, "raw", "-o", "-"]).arg(&plist).stdin(Stdio::null()).output().ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().chars().take(128).collect::<String>();
    (out.status.success() && !value.is_empty()).then_some(value)
}

/// `CFBundleIdentifier`.
pub(crate) fn bundle_id(bundle: &Path) -> Result<String> {
    plist_value(bundle, "CFBundleIdentifier").ok_or_else(|| Error::BadPackage(format!("{} has no bundle id", bundle.display())))
}

/// `CFBundleShortVersionString`: the version an installed bundle says it is.
pub fn bundle_version(bundle: &Path) -> Option<String> {
    plist_value(bundle, "CFBundleShortVersionString")
}

/// Delete a bundle after checking it is the app it is supposed to be and isn't running.
fn remove_bundle(app: &AppInfo, bundle: &Path) -> Result<()> {
    let is_bundle = bundle.extension().is_some_and(|x| x == "app") && std::fs::symlink_metadata(bundle).is_ok_and(|m| m.is_dir());
    if !is_bundle {
        return Err(Error::Refused(format!("{} is not an app bundle; not removing it", bundle.display())));
    }
    let id = bundle_id(bundle)?;
    if id != app.bundle_id {
        return Err(Error::Refused(format!("{} is {id}, not {}; not removing it", bundle.display(), app.bundle_id)));
    }
    if is_running(bundle) {
        return Err(Error::Refused(format!("{} is running; quit it first", app.name)));
    }
    std::fs::remove_dir_all(bundle).map_err(|e| io(bundle.display(), &e))
}

pub(crate) fn uninstall(app: &AppInfo, bundle: &Path) -> Result<()> {
    if !occupied(bundle) {
        return Ok(());
    }
    remove_bundle(app, bundle)
}

/// Detaches the image and removes the mount point when dropped.
struct Detach(PathBuf);

impl Drop for Detach {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil").arg("detach").arg(&self.0).arg("-force").stdin(Stdio::null()).output();
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Run a tool; its first line of stderr explains a failure.
pub(crate) fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| io(what, &e))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(Error::Io(format!("{what} failed: {}", err.lines().next().unwrap_or("no details").chars().take(200).collect::<String>())))
}
