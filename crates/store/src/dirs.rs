//! Where the toolbox keeps its data, in order:
//!
//! 1. `ARTCRAFT_TOOLBOX_CONFIG_DIR`, when set (tests, agents, custom setups).
//! 2. The platform convention: macOS `~/Library/Application Support/ArtCraft Toolbox`, Windows
//!    `%APPDATA%\ArtCraft Toolbox`, Linux and BSD `$XDG_CONFIG_HOME/artcraft-toolbox` or
//!    `~/.config/artcraft-toolbox`.
//!
//! Ported from PhotoCraft's `app_dirs.rs`, without portable mode: the toolbox installs per user.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::{Error, Result};

/// Overrides the data directory.
pub const ENV_CONFIG_DIR: &str = "ARTCRAFT_TOOLBOX_CONFIG_DIR";

/// How [`resolve`] chose the directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Override,
    Platform,
}

/// The data directory for this process's environment.
pub fn data_dir() -> Result<(PathBuf, Mode)> {
    resolve(|k| std::env::var_os(k)).ok_or(Error::NoDataDir)
}

/// Resolve the data directory from `env` (so tests can fake `HOME`, `APPDATA`, `XDG_CONFIG_HOME`).
pub fn resolve(env: impl Fn(&str) -> Option<OsString>) -> Option<(PathBuf, Mode)> {
    if let Some(d) = env(ENV_CONFIG_DIR).filter(|d| !d.is_empty()) {
        return Some((PathBuf::from(d), Mode::Override));
    }
    platform_dir(&env).map(|d| (d, Mode::Platform))
}

/// The platform convention.
pub fn platform_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/ArtCraft Toolbox"));
    }
    if cfg!(windows) {
        return env("APPDATA").filter(|a| !a.is_empty()).map(|a| PathBuf::from(a).join("ArtCraft Toolbox"));
    }
    env("XDG_CONFIG_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|c| c.join("artcraft-toolbox"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let m: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn the_override_wins() {
        let env = env_of(&[(ENV_CONFIG_DIR, "/tmp/tb"), ("HOME", "/home/u"), ("APPDATA", "C:\\Users\\u\\AppData\\Roaming")]);
        assert_eq!(resolve(env), Some((PathBuf::from("/tmp/tb"), Mode::Override)));
        // An empty override is ignored.
        let env = env_of(&[(ENV_CONFIG_DIR, ""), ("HOME", "/home/u"), ("APPDATA", "/appdata")]);
        assert_eq!(resolve(env).map(|(_, m)| m), Some(Mode::Platform));
    }

    #[test]
    fn the_platform_convention() {
        let env = env_of(&[("HOME", "/home/u"), ("APPDATA", "/appdata")]);
        let dir = platform_dir(&env).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(dir, PathBuf::from("/home/u/Library/Application Support/ArtCraft Toolbox"));
        } else if cfg!(windows) {
            assert_eq!(dir, PathBuf::from("/appdata").join("ArtCraft Toolbox"));
        } else {
            assert_eq!(dir, PathBuf::from("/home/u/.config/artcraft-toolbox"));
            let xdg = env_of(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/xdg")]);
            assert_eq!(platform_dir(&xdg).unwrap(), PathBuf::from("/xdg/artcraft-toolbox"));
        }
    }

    #[test]
    fn no_home_no_dir() {
        assert_eq!(resolve(env_of(&[])), None);
        assert_eq!(resolve(env_of(&[("HOME", ""), ("APPDATA", "")])), None);
    }
}
