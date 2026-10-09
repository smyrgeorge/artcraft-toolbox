//! `app.status` and `apps.status`: installed versions against the release feeds.

use serde_json::Value;

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session, params};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "app.status", label: "App Status", params: r#"{"app":"<id>"}"#, enabled: always, run: one, start: None },
        CommandSpec { id: "apps.status", label: "All Apps Status", params: "{}", enabled: always, run: all, start: None },
    ]
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| EngineError::BadParams(e.to_string()))
}

fn one(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let id = params::app(s, p)?.id.clone();
    to_json(&s.app_status(&id)?)
}

fn all(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    to_json(&s.statuses())
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use artcraft_toolbox_model::{Installation, Inventory};
    use artcraft_toolbox_release::{Arch, Os, PackageKind, Target, Version};
    use serde_json::json;

    const FEED: &str = r#"[{"tag_name":"v0.5.0","assets":[
        {"name":"photocraft-0.5.0-linux-x86_64.AppImage","size":1,"browser_download_url":"https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-linux-x86_64.AppImage"}]}]"#;

    #[test]
    fn status_follows_feed_and_inventory() {
        let mut s = Session::new().unwrap();
        s.set_host(Some(Target::new(Os::Linux, Arch::X86_64)));
        assert_eq!(s.execute("app.status", json!({"app": "photocraft"})).unwrap()["status"], json!({"state": "unknown", "installed": null}));

        assert_eq!(s.ingest_releases("photocraft", FEED, 1).unwrap(), 1);
        assert_eq!(s.execute("app.status", json!({"app": "photocraft"})).unwrap()["status"], json!({"state": "notInstalled", "latest": "0.5.0"}));

        let mut inv = Inventory::default();
        inv.record(Installation {
            app: "photocraft".into(),
            version: Version::new(0, 3, 0),
            kind: PackageKind::AppImage,
            path: "/x".into(),
            installed_at: 0,
            active: true,
        })
        .unwrap();
        s.set_inventory(inv);
        assert_eq!(s.execute("app.status", json!({"app": "photocraft"})).unwrap()["status"]["state"], "updateAvailable");

        let all = s.execute("apps.status", json!(null)).unwrap();
        assert_eq!(all.as_array().map(Vec::len), Some(12));
        assert!(s.ingest_releases("nope", FEED, 1).is_err());
        assert!(s.ingest_releases("photocraft", "garbage", 1).is_err());
    }
}
