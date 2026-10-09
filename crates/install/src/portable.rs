//! Windows portable zips: `<id>-<v>-windows-<arch>-portable.zip`, holding one folder with
//! `<id>.exe`, `<id>-cli.exe`, licences and `portable.txt` (docs/release-contract.md).
//!
//! Extraction treats the archive as hostile: entry names go through `enclosed_name` (no `..`, no
//! absolute or drive paths), symbolic links and duplicate names are refused, and the entry count
//! and total size are capped ([`Limits`]); an entry can't write more than it declared. The one
//! shared top-level folder is dropped, `portable.txt` deleted (with it the app would keep its
//! settings beside the exe, inside a version folder the next update replaces), and the result
//! renamed into `<apps>\<Name>\<version>`. Versions live side by side; the active one is the one
//! the Start Menu shortcut opens.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use crate::{Activated, AppInfo, Error, Found, Layout, Result, io, is_running, occupied, staging};

/// How much an archive may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_entries: 10_000, max_total_bytes: 4 << 30 }
    }
}

/// The files that switch a craft to portable mode (PhotoCraft's `app_dirs.rs`).
fn portable_markers(app: &AppInfo) -> [String; 2] {
    ["portable.txt".to_string(), format!("{}.portable", app.name)]
}

/// Extract the version into `<apps>\<Name>\<version>`; returns its `<id>.exe`.
pub(crate) fn place(layout: &Layout, app: &AppInfo, package: &Path, limits: Limits) -> Result<PathBuf> {
    let parent = layout.apps.join(app.name);
    let dir = parent.join(app.version);
    if occupied(&dir) {
        return Err(Error::Exists(dir));
    }
    std::fs::create_dir_all(&parent).map_err(|e| io(parent.display(), &e))?;
    let stage = staging(&parent, app.version);
    let result = (|| {
        extract(package, &stage, limits)?;
        for marker in portable_markers(app) {
            let p = stage.join(marker);
            if p.is_file() {
                std::fs::remove_file(&p).map_err(|e| io(p.display(), &e))?;
            }
        }
        let exe = format!("{}.exe", app.id);
        if !stage.join(&exe).is_file() {
            return Err(Error::BadPackage(format!("it has no {exe}")));
        }
        std::fs::rename(&stage, &dir).map_err(|e| io(dir.display(), &e))?;
        Ok(dir.join(exe))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// Point the Start Menu shortcut at `exe`. The app works without it (the toolbox can still open
/// it), so a failure is a warning.
pub(crate) fn activate(layout: &Layout, app: &AppInfo, exe: &Path) -> Activated {
    if let Some(menu) = &layout.start_menu
        && let Err(e) = shortcut(menu, app, exe)
    {
        log::warn!("no Start Menu shortcut for {}: {e}", app.name);
    }
    Activated { active: exe.to_path_buf(), previous: None }
}

/// Versions in `<apps>\<Name>\` with an `<id>.exe`.
pub(crate) fn find(layout: &Layout, app: &AppInfo) -> Vec<Found> {
    let exe = format!("{}.exe", app.id);
    crate::version_dirs(&layout.apps.join(app.name))
        .into_iter()
        .map(|(dir, version)| Found { path: dir.join(&exe), version })
        .filter(|f| f.path.is_file())
        .collect()
}

/// Extract `package` into the new folder `dest` (see the module docs).
pub fn extract(package: &Path, dest: &Path, limits: Limits) -> Result<()> {
    let bad = |m: &str| Error::BadPackage(m.to_string());
    let file = File::open(package).map_err(|e| io(package.display(), &e))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| Error::BadPackage(format!("not a zip archive ({e})")))?;
    if zip.len() > limits.max_entries {
        return Err(bad("it has too many entries"));
    }
    let mut names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| Error::BadPackage(e.to_string()))?;
        // `enclosed_name` would quietly make `/etc/x` relative; an absolute or drive path means
        // a malformed archive, which is refused rather than reinterpreted.
        let raw = entry.name().map_err(|e| Error::BadPackage(e.to_string()))?;
        let drive = raw.as_bytes().get(1) == Some(&b':') && raw.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        if raw.starts_with(['/', '\\']) || drive {
            return Err(bad("it has an entry with an absolute path"));
        }
        names.push(entry.enclosed_name().ok_or_else(|| bad("it has an entry outside its folder"))?);
    }
    let prefix = common_folder(&names);
    std::fs::create_dir(dest).map_err(|e| io(dest.display(), &e))?;
    let mut total: u64 = 0;
    for (i, name) in names.iter().enumerate() {
        let mut entry = zip.by_index(i).map_err(|e| Error::BadPackage(e.to_string()))?;
        if entry.is_symlink() {
            return Err(bad("it contains a symbolic link"));
        }
        let rel = match &prefix {
            Some(p) => name.strip_prefix(p).unwrap_or(name),
            None => name,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| io(out.display(), &e))?;
            continue;
        }
        let size = entry.size();
        total = total.checked_add(size).filter(|t| *t <= limits.max_total_bytes).ok_or_else(|| bad("it unpacks to more than the size limit"))?;
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
        }
        // `create_new`: a duplicate name is an error, never an overwrite.
        let mut f = File::options().write(true).create_new(true).open(&out).map_err(|e| io(out.display(), &e))?;
        let written = std::io::copy(&mut (&mut entry).take(size.saturating_add(1)), &mut f).map_err(|e| io(out.display(), &e))?;
        if written != size {
            return Err(bad("an entry is not the size it declares"));
        }
    }
    Ok(())
}

/// The single top-level folder every entry is under, if there is one.
fn common_folder(names: &[PathBuf]) -> Option<PathBuf> {
    let first = |p: &PathBuf| match p.components().next() {
        Some(Component::Normal(c)) => Some(PathBuf::from(c)),
        _ => None,
    };
    let top = first(names.first()?)?;
    // Everything under one name, and something actually inside it (a lone file at the root is
    // not a folder to drop).
    let all_under = names.iter().all(|n| first(n).as_ref() == Some(&top));
    let has_nested = names.iter().any(|n| n.components().count() > 1);
    (all_under && has_nested).then_some(top)
}

/// A Start Menu shortcut `<menu>\<Name>.lnk` to `exe`, written by PowerShell's WScript.Shell
/// (the paths travel in environment variables, so no quoting can break the command).
fn shortcut(menu: &Path, app: &AppInfo, exe: &Path) -> Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    std::fs::create_dir_all(menu).map_err(|e| io(menu.display(), &e))?;
    let lnk = menu.join(format!("{}.lnk", app.name));
    let dir = exe.parent().unwrap_or(exe);
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:TOOLBOX_LNK); $s.TargetPath = $env:TOOLBOX_TARGET; $s.WorkingDirectory = $env:TOOLBOX_DIR; $s.Description = $env:TOOLBOX_DESC; $s.Save()";
    let out = crate::powershell(script)
        .env("TOOLBOX_LNK", &lnk)
        .env("TOOLBOX_TARGET", exe)
        .env("TOOLBOX_DIR", dir)
        .env("TOOLBOX_DESC", app.tagline)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| io("powershell", &e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Io(format!("powershell: {}", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("failed"))))
    }
}

/// Remove one version's folder (not the shortcut).
pub(crate) fn remove_version(app: &AppInfo, exe: &Path) -> Result<()> {
    // `<…>\<Name>\<version>\<id>.exe`, nothing else.
    let ok = exe.file_name().is_some_and(|f| f.eq_ignore_ascii_case(format!("{}.exe", app.id).as_str()))
        && exe.parent().and_then(Path::file_name).is_some_and(|v| v == app.version)
        && exe.parent().and_then(Path::parent).and_then(Path::file_name).is_some_and(|n| n == app.name);
    let (Some(dir), true) = (exe.parent(), ok) else {
        return Err(Error::Refused(format!("{} doesn't look like {} {}; not removing it", exe.display(), app.name, app.version)));
    };
    // A running exe is locked: removing the folder around it would leave half of it behind.
    if is_running(exe) {
        return Err(Error::Refused(format!("{} {} is running; quit it first", app.name, app.version)));
    }
    if occupied(dir) {
        std::fs::remove_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
    }
    if let Some(parent) = dir.parent() {
        // Only if nothing else is left in `<Name>`.
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

/// Remove the active version and its shortcut.
pub(crate) fn uninstall(layout: &Layout, app: &AppInfo, exe: &Path) -> Result<()> {
    remove_version(app, exe)?;
    if let Some(menu) = &layout.start_menu {
        let lnk = menu.join(format!("{}.lnk", app.name));
        if lnk.is_file() {
            std::fs::remove_file(&lnk).map_err(|e| io(lnk.display(), &e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_top_folder_is_found() {
        let p = |v: &[&str]| v.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(common_folder(&p(&["app-1/", "app-1/a.exe", "app-1/b/c.txt"])), Some(PathBuf::from("app-1")));
        assert_eq!(common_folder(&p(&["app-1/a.exe", "app-1/b.txt"])), Some(PathBuf::from("app-1")));
        assert_eq!(common_folder(&p(&["a.exe", "b.txt"])), None);
        assert_eq!(common_folder(&p(&["a/x", "b/y"])), None);
        assert_eq!(common_folder(&p(&["a.exe"])), None);
        assert_eq!(common_folder(&[]), None);
    }
}
