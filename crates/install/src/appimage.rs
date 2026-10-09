//! Linux AppImages: `<id>-<v>-linux-<arch>.AppImage`, a type 2 AppImage (an ELF with `AI\x02`
//! at byte 8). Placed at `<apps>/<id>/<version>/<id>.AppImage`, made executable, and announced to
//! the desktop with `<bundle id>.desktop` (marked as the toolbox's, so uninstall removes only its
//! own) and the app's icon in the hicolor theme.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{AppInfo, Error, Layout, Result, io, occupied, staging};

/// Marks desktop entries the toolbox wrote.
const MARKER: &str = "X-ArtCraft-Toolbox=true";

pub(crate) fn install(layout: &Layout, app: &AppInfo, package: &Path) -> Result<PathBuf> {
    check_magic(package)?;
    let parent = layout.apps.join(app.id);
    let dir = parent.join(app.version);
    if occupied(&dir) {
        return Err(Error::Exists(dir));
    }
    std::fs::create_dir_all(&parent).map_err(|e| io(parent.display(), &e))?;
    let stage = staging(&parent, app.version);
    let file = format!("{}.AppImage", app.id);
    let result = (|| {
        std::fs::create_dir(&stage).map_err(|e| io(stage.display(), &e))?;
        let staged = stage.join(&file);
        std::fs::copy(package, &staged).map_err(|e| io(staged.display(), &e))?;
        make_executable(&staged)?;
        std::fs::rename(&stage, &dir).map_err(|e| io(dir.display(), &e))?;
        Ok(dir.join(&file))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    let path = result?;
    // The app works without its menu entry (the toolbox opens it): failures are warnings.
    if let Err(e) = integrate(layout, app, &path) {
        log::warn!("no desktop entry for {}: {e}", app.name);
    }
    Ok(path)
}

fn check_magic(package: &Path) -> Result<()> {
    let mut head = [0u8; 11];
    let mut f = std::fs::File::open(package).map_err(|e| io(package.display(), &e))?;
    f.read_exact(&mut head).map_err(|_| Error::BadPackage("it is too short to be an AppImage".into()))?;
    if head.get(..4) != Some(b"\x7fELF") || head.get(8..11) != Some(b"AI\x02") {
        return Err(Error::BadPackage("it is not a type 2 AppImage".into()));
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| io(path.display(), &e))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn entry_path(layout: &Layout, app: &AppInfo) -> Option<PathBuf> {
    layout.desktop_entries.as_ref().map(|d| d.join(format!("{}.desktop", app.bundle_id)))
}

fn icon_path(layout: &Layout, app: &AppInfo) -> Option<PathBuf> {
    layout.icons.as_ref().map(|d| d.join("hicolor").join("128x128").join("apps").join(format!("{}.png", app.bundle_id)))
}

fn integrate(layout: &Layout, app: &AppInfo, appimage: &Path) -> Result<()> {
    if let (Some(path), Some(png)) = (icon_path(layout, app), app.icon_png) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
        }
        std::fs::write(&path, png).map_err(|e| io(path.display(), &e))?;
    }
    if let Some(path) = entry_path(layout, app) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
        }
        // Someone else's entry under this name (a system package's override, a hand-made one)
        // is left alone.
        if path.is_file() && !std::fs::read_to_string(&path).is_ok_and(|t| t.lines().any(|l| l.trim() == MARKER)) {
            return Err(Error::Exists(path));
        }
        std::fs::write(&path, desktop_entry(app, appimage)?).map_err(|e| io(path.display(), &e))?;
        // Menus pick the entry up faster with a refreshed cache; not having the tool is fine.
        let _ = std::process::Command::new("update-desktop-database").arg(path.parent().unwrap_or(&path)).stdin(std::process::Stdio::null()).output();
    }
    Ok(())
}

/// The desktop entry (freedesktop Desktop Entry Specification 1.5).
pub fn desktop_entry(app: &AppInfo, appimage: &Path) -> Result<String> {
    let text = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={} %F\nIcon={}\nTerminal=false\nStartupWMClass={}\n{MARKER}\nX-ArtCraft-Toolbox-Version={}\n",
        text(app.name),
        text(app.tagline),
        exec_quote(appimage)?,
        app.bundle_id,
        app.bundle_id,
        app.version
    ))
}

/// A path as one quoted `Exec` argument. The spec quotes with `"…"` and escapes `"`, `` ` ``,
/// `$` and `\` with a backslash; the string escaping applied first doubles every backslash again.
/// `%` is doubled (field codes). Control characters can't be represented: refused.
pub fn exec_quote(path: &Path) -> Result<String> {
    let s = path.to_str().ok_or_else(|| Error::Refused("the install path is not UTF-8".into()))?;
    if s.chars().any(char::is_control) {
        return Err(Error::Refused("the install path contains control characters".into()));
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

pub(crate) fn uninstall(layout: &Layout, app: &AppInfo, appimage: &Path) -> Result<()> {
    // `<…>/<id>/<version>/<id>.AppImage`, nothing else.
    let ok = appimage.file_name().is_some_and(|f| f == format!("{}.AppImage", app.id).as_str())
        && appimage.parent().and_then(Path::file_name).is_some_and(|v| v == app.version)
        && appimage.parent().and_then(Path::parent).and_then(Path::file_name).is_some_and(|n| n == app.id);
    let (Some(dir), true) = (appimage.parent(), ok) else {
        return Err(Error::Refused(format!("{} doesn't look like {} {}; not removing it", appimage.display(), app.name, app.version)));
    };
    if occupied(dir) {
        std::fs::remove_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
    }
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    if let Some(entry) = entry_path(layout, app)
        && std::fs::read_to_string(&entry)
            .is_ok_and(|t| t.lines().any(|l| l.trim() == MARKER) && t.contains(&format!("X-ArtCraft-Toolbox-Version={}", app.version)))
    {
        std::fs::remove_file(&entry).map_err(|e| io(entry.display(), &e))?;
        if let Some(icon) = icon_path(layout, app) {
            let _ = std::fs::remove_file(icon);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_arguments_are_quoted_by_the_spec() {
        assert_eq!(exec_quote(Path::new("/home/u/apps/photocraft.AppImage")).unwrap(), "\"/home/u/apps/photocraft.AppImage\"");
        assert_eq!(exec_quote(Path::new("/home/a b/x")).unwrap(), "\"/home/a b/x\"");
        assert_eq!(exec_quote(Path::new("/h/$HOME/`x`/\"q\"")).unwrap(), r#""/h/\\$HOME/\\`x\\`/\\"q\\"""#);
        assert_eq!(exec_quote(Path::new("/h/a\\b")).unwrap(), r#""/h/a\\\\b""#);
        assert_eq!(exec_quote(Path::new("/h/100%")).unwrap(), "\"/h/100%%\"");
        assert!(exec_quote(Path::new("/h/a\nb")).is_err());
    }
}
