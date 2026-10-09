//! What the toolbox has installed. Several versions of one app can be installed side by side; one
//! of them is active (the one launched and updated from). Keeping the previous version makes
//! rollback a switch rather than a download.

use artcraft_toolbox_release::{PackageKind, Version};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Most installations an inventory may hold (all apps, all kept versions).
pub const MAX_INSTALLATIONS: usize = 1024;

/// One installed version of one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    /// Catalog id (`photocraft`).
    pub app: String,
    pub version: Version,
    /// The package it came from.
    pub kind: PackageKind,
    /// Where it lives: the `.app` bundle, the AppImage, or the portable directory.
    pub path: String,
    /// Unix seconds.
    #[serde(default)]
    pub installed_at: u64,
    /// The version the toolbox launches and updates from.
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default)]
    pub installations: Vec<Installation>,
}

impl Inventory {
    /// Load `inventory.json`. A file that doesn't parse is an error the caller reports; it is
    /// never silently replaced, because it is the only record of what is installed where.
    pub fn from_json(text: &str) -> Result<Inventory> {
        let inv: Inventory = crate::from_json("inventory", text)?;
        if inv.installations.len() > MAX_INSTALLATIONS {
            return Err(Error::Corrupt { what: "inventory", reason: format!("{} installations (limit {MAX_INSTALLATIONS})", inv.installations.len()) });
        }
        Ok(inv)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Every installed version of `app`, newest first.
    pub fn versions(&self, app: &str) -> Vec<&Installation> {
        let mut v: Vec<&Installation> = self.installations.iter().filter(|i| i.app == app).collect();
        v.sort_by(|a, b| b.version.cmp(&a.version));
        v
    }

    /// The version of `app` in use: the active one, else the newest installed.
    pub fn current(&self, app: &str) -> Option<&Installation> {
        let versions = self.versions(app);
        versions.iter().find(|i| i.active).or(versions.first()).copied()
    }

    /// Record a new installation and make it the active version of its app. Re-installing a
    /// version that is already recorded replaces that record.
    pub fn record(&mut self, inst: Installation) -> Result<()> {
        let replacing = self.installations.iter().any(|i| i.app == inst.app && i.version == inst.version);
        if !replacing && self.installations.len() >= MAX_INSTALLATIONS {
            return Err(Error::Invalid(format!("too many installations (limit {MAX_INSTALLATIONS})")));
        }
        self.installations.retain(|i| !(i.app == inst.app && i.version == inst.version));
        for i in self.installations.iter_mut().filter(|i| i.app == inst.app) {
            i.active = false;
        }
        self.installations.push(Installation { active: true, ..inst });
        Ok(())
    }

    /// Make an installed version the active one (rollback, or roll forward again).
    pub fn activate(&mut self, app: &str, version: &Version) -> Result<()> {
        if !self.installations.iter().any(|i| i.app == app && &i.version == version) {
            return Err(Error::Invalid(format!("{app} {version} is not installed")));
        }
        for i in self.installations.iter_mut().filter(|i| i.app == app) {
            i.active = &i.version == version;
        }
        Ok(())
    }

    /// Forget an installation (after its files are removed). Returns whether one was removed.
    pub fn remove(&mut self, app: &str, version: &Version) -> bool {
        let before = self.installations.len();
        self.installations.retain(|i| !(i.app == app && &i.version == version));
        before != self.installations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(app: &str, v: &str) -> Installation {
        Installation {
            app: app.into(),
            version: Version::parse(v).unwrap(),
            kind: PackageKind::AppImage,
            path: format!("/apps/{app}/{v}"),
            installed_at: 0,
            active: false,
        }
    }

    #[test]
    fn record_activate_and_current() {
        let mut inv = Inventory::default();
        assert!(inv.current("photocraft").is_none());
        inv.record(inst("photocraft", "0.3.0")).unwrap();
        inv.record(inst("photocraft", "0.5.0")).unwrap();
        inv.record(inst("vectorcraft", "0.7.0")).unwrap();
        assert_eq!(inv.current("photocraft").unwrap().version.to_string(), "0.5.0");
        assert_eq!(inv.versions("photocraft").len(), 2);

        // Roll back, then forward.
        inv.activate("photocraft", &Version::new(0, 3, 0)).unwrap();
        assert_eq!(inv.current("photocraft").unwrap().version.to_string(), "0.3.0");
        assert!(inv.activate("photocraft", &Version::new(9, 9, 9)).is_err());
        assert_eq!(inv.current("vectorcraft").unwrap().version.to_string(), "0.7.0");

        // Re-recording a version replaces it rather than duplicating it.
        inv.record(inst("photocraft", "0.5.0")).unwrap();
        assert_eq!(inv.versions("photocraft").len(), 2);
        assert!(inv.remove("photocraft", &Version::new(0, 3, 0)));
        assert!(!inv.remove("photocraft", &Version::new(0, 3, 0)));
    }

    #[test]
    fn current_falls_back_to_newest_without_an_active_flag() {
        let inv = Inventory { installations: vec![inst("x", "1.0.0"), inst("x", "1.2.0"), inst("x", "1.1.0")] };
        assert_eq!(inv.current("x").unwrap().version.to_string(), "1.2.0");
    }

    #[test]
    fn json_round_trip_and_corrupt_files() {
        let mut inv = Inventory::default();
        inv.record(inst("photocraft", "0.5.0")).unwrap();
        let back = Inventory::from_json(&inv.to_json().unwrap()).unwrap();
        assert_eq!(back, inv);
        assert_eq!(Inventory::from_json("{}").unwrap(), Inventory::default());
        for bad in [
            "",
            "[",
            "null",
            r#"{"installations":[{"app":"x","version":"nope","kind":"dmg","path":""}]}"#,
            r#"{"installations":[{"app":"x","version":"1.0.0","kind":"exe","path":""}]}"#,
        ] {
            assert!(matches!(Inventory::from_json(bad), Err(Error::Corrupt { .. })), "{bad}");
        }
        assert!(matches!(Inventory::from_json(&" ".repeat(crate::MAX_FILE_BYTES + 1)), Err(Error::TooLarge { .. })));
    }
}
