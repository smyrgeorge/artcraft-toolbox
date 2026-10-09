//! What the toolbox has installed. Several versions of one app can be installed side by side; one
//! of them is active (the one launched and updated from). Keeping the previous version makes
//! rollback a switch rather than a download.

use artcraft_toolbox_release::{PackageKind, Version};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Most installations an inventory may hold (all apps, all kept versions).
pub const MAX_INSTALLATIONS: usize = 1024;

/// What the platform says about who made an installed version (`codesign` and Gatekeeper on
/// macOS, Authenticode on Windows; AppImages carry no platform signature).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Trust {
    /// A valid platform signature. `signer` names the certificate ("Developer ID Application:
    /// Learning Machines LLC (DJ6XS33FX8)", or an Authenticode subject); `team` is the macOS
    /// team id; `notarized`: Gatekeeper accepts it (macOS only).
    Signed { signer: String, team: Option<String>, notarized: bool },
    /// No platform signature: AppImages, and unsigned or ad-hoc signed builds.
    Unsigned,
    /// The check couldn't be made (a tool missing or failing); `reason` says why.
    Unchecked { reason: String },
}

impl Trust {
    /// The identity every later version must keep: the macOS team id, else the signer.
    pub fn identity(&self) -> Option<&str> {
        match self {
            Trust::Signed { team: Some(t), .. } => Some(t),
            Trust::Signed { signer, .. } => Some(signer),
            _ => None,
        }
    }

    /// `Signed by Learning Machines LLC (DJ6XS33FX8) · notarized`.
    pub fn label(&self) -> String {
        match self {
            Trust::Signed { signer, notarized, .. } => {
                let who = signer.strip_prefix("Developer ID Application: ").unwrap_or(signer);
                if *notarized { format!("Signed by {who} · notarized") } else { format!("Signed by {who}") }
            }
            Trust::Unsigned => "No platform signature".into(),
            Trust::Unchecked { reason } => format!("Signature not checked: {reason}"),
        }
    }
}

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
    /// Its platform signature when it was installed (absent in older inventories).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<Trust>,
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

    /// `app`'s installation of `version`.
    pub fn get(&self, app: &str, version: &Version) -> Option<&Installation> {
        self.installations.iter().find(|i| i.app == app && &i.version == version)
    }

    /// Record where an installation's files moved to (macOS keeps inactive bundles elsewhere).
    pub fn set_path(&mut self, app: &str, version: &Version, path: String) -> Result<()> {
        let inst = self
            .installations
            .iter_mut()
            .find(|i| i.app == app && &i.version == version)
            .ok_or_else(|| Error::Invalid(format!("{app} {version} is not installed")))?;
        inst.path = path;
        Ok(())
    }

    /// Change the version an installation is recorded as (the app updated itself in place).
    pub fn set_version(&mut self, app: &str, from: &Version, to: Version) -> Result<()> {
        if self.get(app, &to).is_some() {
            return Err(Error::Invalid(format!("{app} {to} is already recorded")));
        }
        let inst = self
            .installations
            .iter_mut()
            .find(|i| i.app == app && &i.version == from)
            .ok_or_else(|| Error::Invalid(format!("{app} {from} is not installed")))?;
        inst.version = to;
        Ok(())
    }

    /// `app`'s inactive versions, the one installed most recently first: the order rollback and
    /// pruning go by.
    pub fn previous(&self, app: &str) -> Vec<&Installation> {
        let mut v: Vec<&Installation> = self.installations.iter().filter(|i| i.app == app && !i.active).collect();
        v.sort_by(|a, b| b.installed_at.cmp(&a.installed_at).then_with(|| b.version.cmp(&a.version)));
        v
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
            trust: None,
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
    fn previous_versions_go_by_install_time_and_paths_and_versions_can_change() {
        let mut inv = Inventory::default();
        for (v, at) in [("0.3.0", 10), ("0.4.0", 30), ("0.2.0", 20), ("0.5.0", 40)] {
            inv.record(Installation { installed_at: at, ..inst("x", v) }).unwrap();
        }
        let prev: Vec<String> = inv.previous("x").iter().map(|i| i.version.to_string()).collect();
        assert_eq!(prev, ["0.4.0", "0.2.0", "0.3.0"], "most recently installed first");
        inv.set_path("x", &Version::new(0, 4, 0), "/kept/x/0.4.0".into()).unwrap();
        assert_eq!(inv.get("x", &Version::new(0, 4, 0)).unwrap().path, "/kept/x/0.4.0");
        assert!(inv.set_path("x", &Version::new(9, 0, 0), String::new()).is_err());
        // An app that updated itself: its record follows, unless that version is recorded already.
        inv.set_version("x", &Version::new(0, 5, 0), Version::new(0, 6, 0)).unwrap();
        assert_eq!(inv.current("x").unwrap().version.to_string(), "0.6.0");
        assert!(inv.set_version("x", &Version::new(0, 6, 0), Version::new(0, 4, 0)).is_err());
    }

    #[test]
    fn trust_labels_and_identities() {
        let signed =
            Trust::Signed { signer: "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)".into(), team: Some("DJ6XS33FX8".into()), notarized: true };
        assert_eq!(signed.label(), "Signed by Learning Machines LLC (DJ6XS33FX8) · notarized");
        assert_eq!(signed.identity(), Some("DJ6XS33FX8"));
        let windows = Trust::Signed { signer: "CN=Learning Machines LLC".into(), team: None, notarized: false };
        assert_eq!((windows.label().as_str(), windows.identity()), ("Signed by CN=Learning Machines LLC", Some("CN=Learning Machines LLC")));
        assert_eq!((Trust::Unsigned.identity(), Trust::Unchecked { reason: "x".into() }.identity()), (None, None));
        // Recorded with the installation, and absent from older inventories.
        let mut inv = Inventory::default();
        inv.record(Installation { trust: Some(signed.clone()), ..inst("x", "1.0.0") }).unwrap();
        let back = Inventory::from_json(&inv.to_json().unwrap()).unwrap();
        assert_eq!(back.current("x").unwrap().trust, Some(signed));
        let old = Inventory::from_json(r#"{"installations":[{"app":"x","version":"1.0.0","kind":"dmg","path":"/a"}]}"#).unwrap();
        assert_eq!(old.current("x").unwrap().trust, None);
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
