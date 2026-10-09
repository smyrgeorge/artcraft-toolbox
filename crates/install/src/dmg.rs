//! macOS DMGs: `<id>-<v>-macos-universal.dmg`, holding `<Name>.app` and an `Applications` link.
//!
//! The image is attached read-only, without Finder and at a private mount point; the one `.app`
//! at its root must carry the catalog's bundle id; it is copied with `ditto` (which keeps the code
//! signature, extended attributes and permissions) to a staging name in the apps folder, renamed
//! into place, and the image detached whatever happened. An existing `<Name>.app` is never
//! replaced: an app installed by hand stays untouched.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{AppInfo, Error, Layout, Result, io, occupied, staging};

pub(crate) fn install(layout: &Layout, app: &AppInfo, package: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(&layout.apps).map_err(|e| io(layout.apps.display(), &e))?;
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
    let dest = layout.apps.join(&name);
    if occupied(&dest) {
        return Err(Error::Exists(dest));
    }
    let stage = staging(&layout.apps, &name);
    let copied = run(Command::new("ditto").arg(&bundle).arg(&stage), "ditto").and_then(|()| std::fs::rename(&stage, &dest).map_err(|e| io(dest.display(), &e)));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    Ok(dest)
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

/// `CFBundleIdentifier` from a bundle's `Info.plist` (through `plutil`, part of macOS).
pub(crate) fn bundle_id(bundle: &Path) -> Result<String> {
    let plist = bundle.join("Contents").join("Info.plist");
    let out = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(&plist)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| io("plutil", &e))?;
    if !out.status.success() {
        return Err(Error::BadPackage(format!("{} has no bundle id", bundle.display())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn uninstall(app: &AppInfo, bundle: &Path) -> Result<()> {
    let is_bundle = bundle.extension().is_some_and(|x| x == "app") && std::fs::symlink_metadata(bundle).is_ok_and(|m| m.is_dir());
    if !is_bundle {
        if !occupied(bundle) {
            return Ok(());
        }
        return Err(Error::Refused(format!("{} is not an app bundle; not removing it", bundle.display())));
    }
    // Only the app it is supposed to be.
    let id = bundle_id(bundle)?;
    if id != app.bundle_id {
        return Err(Error::Refused(format!("{} is {id}, not {}; not removing it", bundle.display(), app.bundle_id)));
    }
    std::fs::remove_dir_all(bundle).map_err(|e| io(bundle.display(), &e))
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
fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| io(what, &e))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(Error::Io(format!("{what} failed: {}", err.lines().next().unwrap_or("no details").chars().take(200).collect::<String>())))
}
