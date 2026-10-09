//! Where installed apps go, per OS (docs/architecture.md § 5):
//!
//! | | apps | also |
//! |---|---|---|
//! | macOS | `~/Applications/<Name>.app` | |
//! | Windows | `%LOCALAPPDATA%\Programs\ArtCraft\<Name>\<version>\` | Start Menu `ArtCraft\<Name>.lnk` |
//! | Linux | `~/.local/share/artcraft-toolbox/apps/<id>/<version>/<id>.AppImage` | `~/.local/share/applications/<bundle id>.desktop`, hicolor icon |
//!
//! The `installDir` setting replaces the apps folder. Downloads are staged in the toolbox's data
//! folder (`downloads/`). On macOS, where only one version can be in the apps folder, the others
//! are kept in `versions.noindex/<id>/<version>/` there (Spotlight skips `.noindex` folders).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Where apps go.
    pub apps: PathBuf,
    /// Linux: desktop entries (`~/.local/share/applications`).
    pub desktop_entries: Option<PathBuf>,
    /// Linux: the icon theme root (`~/.local/share/icons`); icons go in `hicolor/128x128/apps`.
    pub icons: Option<PathBuf>,
    /// Windows: the Start Menu folder for shortcuts.
    pub start_menu: Option<PathBuf>,
    /// Partial and verified downloads, before they are installed.
    pub downloads: PathBuf,
    /// macOS: the versions that aren't active (`<kept>/<id>/<version>/<Name>.app`).
    pub kept: PathBuf,
}

impl Layout {
    /// The platform's layout, from the toolbox's data folder, the `installDir` setting and the
    /// environment (`HOME`, `LOCALAPPDATA`, `APPDATA`, `XDG_DATA_HOME`). `None` when the
    /// environment names no home.
    pub fn platform(data_dir: &Path, install_dir: Option<&Path>, env: impl Fn(&str) -> Option<OsString>) -> Option<Layout> {
        let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let (downloads, kept) = (data_dir.join("downloads"), data_dir.join("versions.noindex"));
        if cfg!(target_os = "macos") {
            let apps = install_dir.map(Path::to_path_buf).or_else(|| var("HOME").map(|h| h.join("Applications")))?;
            return Some(Layout { apps, desktop_entries: None, icons: None, start_menu: None, downloads, kept });
        }
        if cfg!(windows) {
            let apps = install_dir.map(Path::to_path_buf).or_else(|| var("LOCALAPPDATA").map(|l| l.join("Programs").join("ArtCraft")))?;
            let start_menu = var("APPDATA").map(|a| a.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("ArtCraft"));
            return Some(Layout { apps, desktop_entries: None, icons: None, start_menu, downloads, kept });
        }
        let data_home = var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))?;
        let apps = install_dir.map(Path::to_path_buf).unwrap_or_else(|| data_home.join("artcraft-toolbox").join("apps"));
        Some(Layout { apps, desktop_entries: Some(data_home.join("applications")), icons: Some(data_home.join("icons")), start_menu: None, downloads, kept })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<OsString> {
        match k {
            "HOME" => Some("/home/u".into()),
            "LOCALAPPDATA" => Some("C:\\Users\\u\\AppData\\Local".into()),
            "APPDATA" => Some("C:\\Users\\u\\AppData\\Roaming".into()),
            _ => None,
        }
    }

    #[test]
    fn platform_layouts() {
        let l = Layout::platform(Path::new("/data"), None, env).unwrap();
        assert_eq!(l.downloads, PathBuf::from("/data/downloads"));
        if cfg!(target_os = "macos") {
            assert_eq!(l.apps, PathBuf::from("/home/u/Applications"));
            assert!(l.desktop_entries.is_none() && l.start_menu.is_none());
        } else if cfg!(windows) {
            assert!(l.apps.ends_with("Programs\\ArtCraft") || l.apps.ends_with("Programs/ArtCraft"));
            assert!(l.start_menu.is_some());
        } else {
            assert_eq!(l.apps, PathBuf::from("/home/u/.local/share/artcraft-toolbox/apps"));
            assert_eq!(l.desktop_entries, Some(PathBuf::from("/home/u/.local/share/applications")));
        }
        let custom = Layout::platform(Path::new("/data"), Some(Path::new("/opt/crafts")), env).unwrap();
        assert_eq!(custom.apps, PathBuf::from("/opt/crafts"));
        assert_eq!(Layout::platform(Path::new("/data"), None, |_| None), None);
    }
}
