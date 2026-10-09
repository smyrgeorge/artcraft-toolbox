//! `catalog.*` and `app.info`: what the suite contains.

use artcraft_toolbox_catalog::App;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{Result, Session, params};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "catalog.list", label: "List Apps", params: "{}", enabled: always, run: list, start: None },
        CommandSpec { id: "app.info", label: "App Info", params: r#"{"app":"<id>"}"#, enabled: always, run: info, start: None },
    ]
}

fn describe(a: &App) -> Value {
    json!({
        "id": a.id, "name": a.name, "tagline": a.tagline, "repo": a.repo, "repoUrl": a.repo_url(),
        "website": a.website, "bundleId": a.bundle_id(), "formerSlugs": a.former_slugs,
    })
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    Ok(Value::Array(s.catalog().apps.iter().map(describe).collect()))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?;
    let mut v = describe(app);
    let installed: Vec<Value> = s.inventory().versions(&app.id).iter().map(|i| json!({"version": i.version, "path": i.path, "active": i.active})).collect();
    let releases: Vec<Value> = s
        .feed(&app.id)
        .map(|f| {
            f.releases
                .iter()
                .map(|r| json!({"version": r.version, "prerelease": r.is_prerelease(), "publishedAt": r.published_at, "assets": r.assets.len()}))
                .collect()
        })
        .unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("installed".into(), Value::Array(installed));
        o.insert("releases".into(), Value::Array(releases));
        o.insert("feedFetchedAt".into(), json!(s.feed(&app.id).map(|f| f.fetched_at)));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::json;

    #[test]
    fn list_and_info() {
        let mut s = Session::new().unwrap();
        let list = s.execute("catalog.list", json!({})).unwrap();
        assert_eq!(list.as_array().map(Vec::len), Some(12));
        assert_eq!(list[0]["bundleId"], "ai.storyteller.photocraft");
        let info = s.execute("app.info", json!({"app": "pdfcraft"})).unwrap();
        assert_eq!(info["formerSlugs"], json!(["printcraft"]));
        assert_eq!(info["releases"], json!([]));
        assert!(s.execute("app.info", json!({"app": "nope"})).unwrap_err().to_string().contains("unknown app"));
        assert!(s.execute("app.info", json!({})).unwrap_err().to_string().contains("missing `app`"));
        assert!(s.execute("catalog.list", json!({"x": 1})).unwrap_err().to_string().contains("unknown param"));
    }
}
