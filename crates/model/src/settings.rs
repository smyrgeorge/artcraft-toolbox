//! User settings: which releases to follow and how updates are handled.

use std::collections::BTreeMap;

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
/// Most per-app override entries kept (the catalog has a dozen apps).
pub const MAX_APP_OVERRIDES: usize = 256;
/// The text sizes offered, in percent of the normal size.
pub const TEXT_SIZES: [u32; 5] = [90, 100, 110, 125, 150];

/// Light or dark: the system's, or one chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePref {
    pub fn parse(s: &str) -> Option<ThemePref> {
        match s {
            "system" => Some(ThemePref::System),
            "dark" => Some(ThemePref::Dark),
            "light" => Some(ThemePref::Light),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Dark => "dark",
            ThemePref::Light => "light",
        }
    }
}

fn lenient_theme<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<ThemePref, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_str().and_then(ThemePref::parse).unwrap_or_default())
}

/// Is `s` the `language` setting's `auto` or a language code (`ja`, `zh-hant`, `pt-br`)? Which
/// codes exist is the UI's business; an unknown one shows English.
pub fn valid_language(s: &str) -> bool {
    s == "auto"
        || ((2..=16).contains(&s.len())
            && s.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
            && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase))
}

/// One app's overrides of the global settings. `None` follows the global value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
    /// Never offer a version newer than this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<Version>,
}

impl AppSettings {
    pub fn is_default(&self) -> bool {
        self == &AppSettings::default()
    }
}

/// An app id fit to be a settings key: what the catalog accepts (a-z, 0-9, `-`).
fn valid_app_key(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

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
    /// Tell the user (an OS notification) when a check finds updates for installed apps.
    pub notifications: bool,
    /// Closing the window keeps the toolbox running in the menu bar or system tray (when there is one).
    pub close_to_tray: bool,
    /// The UI language: `auto` (the system's) or a language code (`ja`, `zh-hant`, `pt-br`).
    pub language: String,
    /// An unknown value (a newer version's theme) reads as `system`, not as a corrupt file.
    #[serde(deserialize_with = "lenient_theme")]
    pub theme: ThemePref,
    /// Text and controls, in percent of the normal size (one of [`TEXT_SIZES`]).
    pub text_size: u32,
    /// Per-app overrides, by catalog id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub apps: BTreeMap<String, AppSettings>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            channel: Channel::Stable,
            check_interval_hours: 6,
            auto_update: false,
            keep_previous: 1,
            install_dir: None,
            notifications: true,
            close_to_tray: true,
            language: "auto".into(),
            theme: ThemePref::System,
            text_size: 100,
            apps: BTreeMap::new(),
        }
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
        if !valid_language(&self.language) {
            self.language = "auto".into();
        }
        if !TEXT_SIZES.contains(&self.text_size) {
            self.text_size = 100;
        }
        self.apps.retain(|id, a| valid_app_key(id) && !a.is_default());
        while self.apps.len() > MAX_APP_OVERRIDES {
            self.apps.pop_last();
        }
        self
    }

    /// The channel `app` follows: its override, else the global one.
    pub fn channel_for(&self, app: &str) -> Channel {
        self.apps.get(app).and_then(|a| a.channel).unwrap_or(self.channel)
    }

    /// Whether `app` updates by itself: its override, else the global setting.
    pub fn auto_update_for(&self, app: &str) -> bool {
        self.apps.get(app).and_then(|a| a.auto_update).unwrap_or(self.auto_update)
    }

    /// The version `app` is pinned to, if any.
    pub fn pinned(&self, app: &str) -> Option<&Version> {
        self.apps.get(app).and_then(|a| a.pinned.as_ref())
    }

    /// Validate and set one of `app`'s overrides by its JSON key (`channel`, `autoUpdate`,
    /// `pinned`); `null` clears it, so the app follows the global setting again.
    pub fn set_app(&mut self, app: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        let bad = |m: String| Err(Error::Invalid(m));
        if !valid_app_key(app) {
            return bad(format!("`{}` is not an app id", app.chars().take(40).collect::<String>()));
        }
        if !self.apps.contains_key(app) && self.apps.len() >= MAX_APP_OVERRIDES {
            return bad(format!("too many apps with their own settings (limit {MAX_APP_OVERRIDES})"));
        }
        let mut entry = self.apps.get(app).cloned().unwrap_or_default();
        let null = value.is_null();
        match key {
            "channel" => match (null, value.as_str().and_then(Channel::parse)) {
                (true, _) => entry.channel = None,
                (false, Some(c)) => entry.channel = Some(c),
                (false, None) => return bad("channel must be \"stable\", \"prerelease\" or null".into()),
            },
            "autoUpdate" => match (null, value.as_bool()) {
                (true, _) => entry.auto_update = None,
                (false, Some(b)) => entry.auto_update = Some(b),
                (false, None) => return bad("autoUpdate must be true, false or null".into()),
            },
            "pinned" => match (null, value.as_str().map(Version::parse)) {
                (true, _) => entry.pinned = None,
                (false, Some(Ok(v))) => entry.pinned = Some(v),
                (false, Some(Err(e))) => return bad(format!("pinned: {e}")),
                (false, None) => return bad("pinned must be a version like \"0.5.0\" or null".into()),
            },
            other => return bad(format!("unknown app setting `{}`", other.chars().take(40).collect::<String>())),
        }
        if entry.is_default() {
            self.apps.remove(app);
        } else {
            self.apps.insert(app.to_string(), entry);
        }
        Ok(())
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
            "notifications" => match value.as_bool() {
                Some(b) => self.notifications = b,
                None => return bad("notifications must be true or false".into()),
            },
            "closeToTray" => match value.as_bool() {
                Some(b) => self.close_to_tray = b,
                None => return bad("closeToTray must be true or false".into()),
            },
            "language" => match value.as_str() {
                Some(l) if valid_language(l) => self.language = l.to_string(),
                _ => return bad("language must be \"auto\" or a language code such as \"en\" or \"ja\"".into()),
            },
            "theme" => match value.as_str().and_then(ThemePref::parse) {
                Some(t) => self.theme = t,
                None => return bad("theme must be \"system\", \"dark\" or \"light\"".into()),
            },
            "textSize" => match value.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| TEXT_SIZES.contains(n)) {
                Some(n) => self.text_size = n,
                None => return bad(format!("textSize must be one of {TEXT_SIZES:?} (percent)")),
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
    fn language_theme_and_text_size() {
        let mut s = Settings::default();
        assert_eq!((s.language.as_str(), s.theme, s.text_size), ("auto", ThemePref::System, 100));
        for (key, good, bad) in [
            (
                "language",
                vec![json!("ja"), json!("zh-hant"), json!("pt-br"), json!("auto")],
                vec![json!("JA"), json!(""), json!("../x"), json!("a"), json!(1), json!("x".repeat(20))],
            ),
            ("theme", vec![json!("light"), json!("dark"), json!("system")], vec![json!("Light"), json!("blue"), json!(true)]),
            ("textSize", vec![json!(90), json!(150), json!(100)], vec![json!(101), json!(0), json!("100"), json!(-1)]),
        ] {
            for v in good {
                s.set(key, &v).unwrap_or_else(|e| panic!("{key} = {v}: {e}"));
            }
            for v in bad {
                assert!(s.set(key, &v).is_err(), "{key} = {v}");
            }
        }
        // Files from elsewhere: bad values fall back, missing ones take defaults.
        let loaded = Settings::from_json(r#"{"language":"<script>","textSize":7,"theme":"light"}"#).unwrap();
        assert_eq!((loaded.language.as_str(), loaded.theme, loaded.text_size), ("auto", ThemePref::Light, 100));
        assert_eq!(Settings::from_json(r#"{"theme":"purple"}"#).unwrap().theme, ThemePref::System, "a newer version's theme isn't a corrupt file");
    }

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
    fn per_app_overrides() {
        let mut s = Settings::default();
        assert_eq!(s.channel_for("photocraft"), Channel::Stable);
        s.set_app("photocraft", "channel", &json!("prerelease")).unwrap();
        s.set_app("photocraft", "pinned", &json!("0.4.0")).unwrap();
        s.set_app("vectorcraft", "autoUpdate", &json!(true)).unwrap();
        assert_eq!(s.channel_for("photocraft"), Channel::Prerelease);
        assert_eq!(s.channel_for("vectorcraft"), Channel::Stable);
        assert_eq!(s.pinned("photocraft"), Some(&Version::new(0, 4, 0)));
        assert!(s.auto_update_for("vectorcraft") && !s.auto_update_for("photocraft"));
        // Round trip, then clearing an override drops the entry once nothing is left.
        let back = Settings::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back, s);
        s.set_app("photocraft", "channel", &json!(null)).unwrap();
        s.set_app("photocraft", "pinned", &json!(null)).unwrap();
        assert!(!s.apps.contains_key("photocraft"));
        let before = s.clone();
        for (app, k, v) in [
            ("photocraft", "channel", json!("nightly")),
            ("photocraft", "pinned", json!("latest")),
            ("photocraft", "pinned", json!(5)),
            ("photocraft", "autoUpdate", json!("yes")),
            ("photocraft", "colour", json!(1)),
            ("../x", "channel", json!("stable")),
            ("", "channel", json!("stable")),
        ] {
            assert!(s.set_app(app, k, &v).is_err(), "{app} {k} = {v}");
        }
        assert_eq!(s, before, "a rejected override changes nothing");
    }

    #[test]
    fn hostile_override_files_load_sanitised() {
        let many: serde_json::Map<String, serde_json::Value> = (0..MAX_APP_OVERRIDES + 10).map(|i| (format!("app{i}"), json!({"autoUpdate": true}))).collect();
        let s = Settings::from_json(&json!({ "apps": many }).to_string()).unwrap();
        assert_eq!(s.apps.len(), MAX_APP_OVERRIDES);
        let s = Settings::from_json(r#"{"apps":{"../evil":{"autoUpdate":true},"ok":{},"good":{"channel":"prerelease"}}}"#).unwrap();
        assert_eq!(s.apps.keys().collect::<Vec<_>>(), ["good"]);
        assert!(Settings::from_json(r#"{"apps":{"x":{"pinned":"not a version"}}}"#).is_err());
    }

    #[test]
    fn set_validates_every_key() {
        let mut s = Settings::default();
        s.set("channel", &json!("prerelease")).unwrap();
        s.set("checkIntervalHours", &json!(0)).unwrap();
        s.set("autoUpdate", &json!(true)).unwrap();
        s.set("keepPrevious", &json!(2)).unwrap();
        s.set("installDir", &json!("/opt/crafts")).unwrap();
        s.set("notifications", &json!(false)).unwrap();
        s.set("closeToTray", &json!(false)).unwrap();
        assert!(!s.notifications && !s.close_to_tray);
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
            ("notifications", json!(1)),
            ("closeToTray", json!(null)),
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
