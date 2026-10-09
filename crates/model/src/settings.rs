//! User settings: which releases to follow and how updates are handled.

use artcraft_toolbox_release::Version;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Which releases to offer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Published releases only.
    #[default]
    Stable,
    /// Also pre-releases (GitHub's pre-release flag, or a `-rc.1`-style version).
    Prerelease,
}

impl Channel {
    /// Does this channel offer a release with this version and GitHub pre-release flag?
    pub fn offers(self, version: &Version, prerelease_flag: bool) -> bool {
        match self {
            Channel::Stable => !prerelease_flag && !version.is_prerelease(),
            Channel::Prerelease => true,
        }
    }

    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "stable" => Some(Channel::Stable),
            "prerelease" => Some(Channel::Prerelease),
            _ => None,
        }
    }
}

/// Hours between automatic update checks; 0 turns them off.
pub const MAX_CHECK_INTERVAL_HOURS: u32 = 24 * 7;
/// Previous versions kept per app for rollback.
pub const MAX_KEEP_PREVIOUS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub channel: Channel,
    /// Hours between background update checks (0 = only when asked).
    pub check_interval_hours: u32,
    /// Install updates as soon as they are found (apps that are running are updated on next quit).
    pub auto_update: bool,
    /// Previous versions kept per app, for rollback.
    pub keep_previous: u32,
    /// Where apps are installed; `None` is the platform default (docs/architecture.md › Install layout).
    pub install_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { channel: Channel::Stable, check_interval_hours: 6, auto_update: false, keep_previous: 1, install_dir: None }
    }
}

impl Settings {
    /// Load `settings.json`. Unknown keys are ignored and missing ones take their defaults, so
    /// files written by other versions still load; out-of-range values are clamped.
    pub fn from_json(text: &str) -> Result<Settings> {
        let s: Settings = crate::from_json("settings", text)?;
        Ok(s.clamped())
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))
    }

    fn clamped(mut self) -> Settings {
        self.check_interval_hours = self.check_interval_hours.min(MAX_CHECK_INTERVAL_HOURS);
        self.keep_previous = self.keep_previous.min(MAX_KEEP_PREVIOUS);
        self
    }

    /// Validate a new value for one setting, by its JSON key. Unlike loading, this rejects out-of-range
    /// values: a caller asking for 1000 hours should hear that it can't have them.
    pub fn set(&mut self, key: &str, value: &serde_json::Value) -> Result<()> {
        let bad = |m: String| Err(Error::Invalid(m));
        match key {
            "channel" => match value.as_str().and_then(Channel::parse) {
                Some(c) => self.channel = c,
                None => return bad("channel must be \"stable\" or \"prerelease\"".into()),
            },
            "checkIntervalHours" => match value.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= MAX_CHECK_INTERVAL_HOURS) {
                Some(n) => self.check_interval_hours = n,
                None => return bad(format!("checkIntervalHours must be a whole number from 0 to {MAX_CHECK_INTERVAL_HOURS}")),
            },
            "autoUpdate" => match value.as_bool() {
                Some(b) => self.auto_update = b,
                None => return bad("autoUpdate must be true or false".into()),
            },
            "keepPrevious" => match value.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= MAX_KEEP_PREVIOUS) {
                Some(n) => self.keep_previous = n,
                None => return bad(format!("keepPrevious must be a whole number from 0 to {MAX_KEEP_PREVIOUS}")),
            },
            "installDir" => match value {
                serde_json::Value::Null => self.install_dir = None,
                serde_json::Value::String(s) if !s.trim().is_empty() && s.len() <= 4096 && !s.contains('\0') => self.install_dir = Some(s.clone()),
                _ => return bad("installDir must be a non-empty path or null".into()),
            },
            other => return bad(format!("unknown setting `{}`", other.chars().take(40).collect::<String>())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn channels() {
        let rc = Version::parse("1.0.0-rc.1").unwrap();
        let rel = Version::new(1, 0, 0);
        assert!(Channel::Stable.offers(&rel, false));
        assert!(!Channel::Stable.offers(&rel, true));
        assert!(!Channel::Stable.offers(&rc, false));
        assert!(Channel::Prerelease.offers(&rc, true));
    }

    #[test]
    fn loads_partial_and_clamps() {
        let s = Settings::from_json(r#"{"channel":"prerelease","checkIntervalHours":100000,"unknown":1}"#).unwrap();
        assert_eq!(s.channel, Channel::Prerelease);
        assert_eq!(s.check_interval_hours, MAX_CHECK_INTERVAL_HOURS);
        assert_eq!(s.keep_previous, 1);
        assert_eq!(Settings::from_json(&Settings::default().to_json().unwrap()).unwrap(), Settings::default());
        assert!(Settings::from_json(r#"{"channel":"nightly"}"#).is_err());
        assert!(Settings::from_json("not json").is_err());
    }

    #[test]
    fn set_validates_every_key() {
        let mut s = Settings::default();
        s.set("channel", &json!("prerelease")).unwrap();
        s.set("checkIntervalHours", &json!(0)).unwrap();
        s.set("autoUpdate", &json!(true)).unwrap();
        s.set("keepPrevious", &json!(2)).unwrap();
        s.set("installDir", &json!("/opt/crafts")).unwrap();
        assert_eq!((s.channel, s.check_interval_hours, s.auto_update, s.keep_previous), (Channel::Prerelease, 0, true, 2));
        s.set("installDir", &json!(null)).unwrap();
        assert_eq!(s.install_dir, None);
        let before = s.clone();
        for (k, v) in [
            ("channel", json!("nightly")),
            ("channel", json!(1)),
            ("checkIntervalHours", json!(-1)),
            ("checkIntervalHours", json!(1.5)),
            ("checkIntervalHours", json!(u64::MAX)),
            ("autoUpdate", json!("yes")),
            ("keepPrevious", json!(99)),
            ("installDir", json!("")),
            ("installDir", json!("a\u{0}b")),
            ("nope", json!(1)),
        ] {
            assert!(s.set(k, &v).is_err(), "{k} = {v}");
        }
        assert_eq!(s, before, "a rejected value changes nothing");
    }
}
