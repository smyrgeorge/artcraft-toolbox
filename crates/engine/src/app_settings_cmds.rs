//! Per-app commands beyond status: `app.settings.get` / `app.settings.set` (overrides of the
//! global channel and auto-update, and a pinned version) and `app.releases` (versions, notes).

use artcraft_toolbox_release::asset;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session, params};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "app.settings.get", label: "App Settings", params: r#"{"app":"<id>"}"#, enabled: always, run: get, start: None },
        CommandSpec {
            id: "app.settings.set",
            label: "Change App Settings",
            params: r#"{"app":"<id>","channel"?:"stable|prerelease"|null,"autoUpdate"?:bool|null,"pinned"?:"x.y.z"|null}"#,
            enabled: always,
            run: set,
            start: None,
        },
        CommandSpec { id: "app.releases", label: "App Releases", params: r#"{"app":"<id>","limit"?:1..100=20}"#, enabled: always, run: releases, start: None },
    ]
}

/// The effective settings of `app`, and which of them are its own.
fn describe(s: &Session, app: &str) -> Value {
    let st = s.settings();
    json!({
        "app": app,
        "channel": st.channel_for(app),
        "autoUpdate": st.auto_update_for(app),
        "pinned": st.pinned(app),
        "overrides": st.apps.get(app).cloned().unwrap_or_default(),
    })
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let id = params::app(s, p)?.id.clone();
    Ok(describe(s, &id))
}

/// All or nothing, like `settings.set`; `null` clears an override.
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app", "channel", "autoUpdate", "pinned"])?;
    let id = params::app(s, p)?.id.clone();
    let changes: Vec<(&String, &Value)> = params::object(p)?.iter().filter(|(k, _)| k.as_str() != "app").collect();
    if changes.is_empty() {
        return Err(EngineError::BadParams("no settings given".into()));
    }
    let mut next = s.settings().clone();
    for (k, v) in changes {
        next.set_app(&id, k, v)?;
    }
    s.set_settings(next)?;
    Ok(describe(s, &id))
}

/// The app's releases, newest first, with notes and whether this computer can install them.
fn releases(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app", "limit"])?;
    let id = params::app(s, p)?.id.clone();
    let limit = match params::object(p)?.get("limit") {
        None => 20,
        Some(v) => v.as_u64().filter(|n| (1..=100).contains(n)).ok_or_else(|| EngineError::BadParams("`limit` must be a whole number from 1 to 100".into()))?,
    };
    let (channel, pin, host) = (s.settings().channel_for(&id), s.settings().pinned(&id).cloned(), s.host());
    let Some(feed) = s.feed(&id) else { return Ok(json!({"app": id, "checkedAt": null, "releases": []})) };
    let list: Vec<Value> = feed
        .releases
        .iter()
        .take(usize::try_from(limit).unwrap_or(20))
        .map(|r| {
            let installable = host.is_some_and(|h| asset::select(&r.assets, h, |a| &a.name).is_some());
            let offered = channel.offers(&r.version, r.prerelease) && pin.as_ref().is_none_or(|p| r.version <= *p);
            json!({
                "version": r.version,
                "tag": r.tag,
                "prerelease": r.is_prerelease(),
                "publishedAt": r.published_at,
                "pageUrl": r.page_url,
                "notes": r.notes,
                "installable": installable,
                "offered": offered,
            })
        })
        .collect();
    Ok(json!({"app": id, "checkedAt": feed.fetched_at, "releases": list}))
}

#[cfg(test)]
mod tests {
    use crate::{Session, Status, Target, Version};
    use serde_json::json;

    const FEED: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

    fn session() -> Session {
        let mut s = Session::new().unwrap();
        s.set_host(Target::from_consts("linux", "x86_64"));
        s.ingest_releases("photocraft", FEED, 1).unwrap();
        s
    }

    #[test]
    fn overrides_change_what_is_offered() {
        let mut s = session();
        assert_eq!(s.app_status("photocraft").unwrap().status, Status::NotInstalled { latest: Version::new(0, 5, 0) });
        let out = s.execute("app.settings.set", json!({"app": "photocraft", "pinned": "0.3.0", "channel": "prerelease"})).unwrap();
        assert_eq!((out["pinned"].as_str(), out["channel"].as_str()), (Some("0.3.0"), Some("prerelease")));
        assert_eq!(out["overrides"], json!({"channel": "prerelease", "pinned": "0.3.0"}));
        let st = s.app_status("photocraft").unwrap();
        assert_eq!(st.status, Status::NotInstalled { latest: Version::new(0, 3, 0) });
        assert_eq!(st.pinned, Some(Version::new(0, 3, 0)));
        // Other apps are untouched; clearing restores the global behaviour.
        assert_eq!(s.execute("app.settings.get", json!({"app": "vectorcraft"})).unwrap()["overrides"], json!({}));
        s.execute("app.settings.set", json!({"app": "photocraft", "pinned": null, "channel": null})).unwrap();
        assert!(s.settings().apps.is_empty());
        assert_eq!(s.app_status("photocraft").unwrap().status, Status::NotInstalled { latest: Version::new(0, 5, 0) });
    }

    #[test]
    fn settings_changes_are_all_or_nothing() {
        let mut s = session();
        for (p, want) in [
            (json!({"app": "photocraft"}), "no settings given"),
            (json!({"app": "photocraft", "pinned": "0.3.0", "channel": "nightly"}), "channel must be"),
            (json!({"app": "nope", "pinned": "0.3.0"}), "unknown app"),
            (json!({"app": "photocraft", "colour": 1}), "unknown param"),
        ] {
            let e = s.execute("app.settings.set", p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        assert!(s.settings().apps.is_empty());
    }

    #[test]
    fn releases_carry_notes_and_what_this_computer_gets() {
        let mut s = session();
        let out = s.execute("app.releases", json!({"app": "photocraft", "limit": 2})).unwrap();
        let list = out["releases"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["version"], "0.5.0");
        assert_eq!(list[0]["installable"], true);
        assert_eq!(list[0]["notes"], "Release notes trimmed for this test fixture.");
        assert!(list[0]["pageUrl"].as_str().unwrap().starts_with("https://github.com/storytold/photocraft/releases/tag/"));
        s.execute("app.settings.set", json!({"app": "photocraft", "pinned": "0.3.0"})).unwrap();
        let out = s.execute("app.releases", json!({"app": "photocraft"})).unwrap();
        assert_eq!(out["releases"][0]["offered"], false);
        assert_eq!(s.execute("app.releases", json!({"app": "wordcraft"})).unwrap()["releases"], json!([]));
        for bad in [json!({"app": "photocraft", "limit": 0}), json!({"app": "photocraft", "limit": 101}), json!({"app": "photocraft", "limit": "5"})] {
            assert!(s.execute("app.releases", bad).is_err());
        }
    }
}
