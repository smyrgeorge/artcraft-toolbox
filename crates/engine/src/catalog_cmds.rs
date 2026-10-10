//! `catalog.*` and `app.info`: what the suite contains.

use artcraft_toolbox_catalog::App;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{CatalogSource, Result, Session, params};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "catalog.list", label: "List Apps", params: "{}", enabled: always, run: list, start: None },
        CommandSpec { id: "catalog.status", label: "Catalog Source", params: "{}", enabled: always, run: status, start: None },
        CommandSpec { id: "app.info", label: "App Info", params: r#"{"app":"<id>"}"#, enabled: always, run: info, start: None },
    ]
}

fn describe(a: &App) -> Value {
    json!({
        "id": a.id, "name": a.name, "tagline": a.tagline, "repo": a.repo, "repoUrl": a.repo_url(),
        "website": a.website, "bundleId": a.bundle_id(), "formerSlugs": a.former_slugs,
        "followsContract": a.follows_contract(), "assets": a.assets,
    })
}

/// Where the catalog came from and whether the publisher's documents are followed
/// (docs/architecture.md § 12).
fn status(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    let c = s.catalog();
    let (source, fetched_at) = match s.catalog_source() {
        CatalogSource::Builtin => ("builtin", None),
        CatalogSource::Remote { fetched_at } => ("remote", Some(fetched_at)),
    };
    Ok(json!({
        "source": source,
        "revision": c.revision,
        "apps": c.apps.len(),
        "fetchedAt": fetched_at,
        "feedFetchedAt": s.feed_fetched_at(),
        "enabled": s.remote_enabled(),
        "remote": c.remote.as_ref().map(|r| json!({"catalog": r.catalog, "feed": r.feed, "publicKey": r.public_key})),
    }))
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
        assert_eq!(list.as_array().map(Vec::len), Some(13));
        assert_eq!(list[0]["bundleId"], "ai.storyteller.photocraft");
        assert_eq!(list[0]["followsContract"], true);
        let artcraft = list.as_array().unwrap().iter().find(|a| a["id"] == "artcraft").unwrap();
        assert_eq!(
            (artcraft["followsContract"].as_bool(), artcraft["assets"]["macos-universal"].as_str()),
            (Some(false), Some("ArtCraft_{version}_universal.dmg"))
        );
        let status = s.execute("catalog.status", json!({})).unwrap();
        assert_eq!((status["source"].as_str(), status["revision"].as_u64(), status["apps"].as_u64()), (Some("builtin"), Some(1), Some(13)));
        assert_eq!(status["enabled"], true);
        assert!(status["remote"]["feed"].as_str().unwrap().ends_with("/artcraft-feed.json"));
        assert!(s.execute("catalog.status", json!({"x": 1})).is_err());
        let info = s.execute("app.info", json!({"app": "pdfcraft"})).unwrap();
        assert_eq!(info["formerSlugs"], json!(["printcraft"]));
        assert_eq!(info["releases"], json!([]));
        assert!(s.execute("app.info", json!({"app": "nope"})).unwrap_err().to_string().contains("unknown app"));
        assert!(s.execute("app.info", json!({})).unwrap_err().to_string().contains("missing `app`"));
        assert!(s.execute("catalog.list", json!({"x": 1})).unwrap_err().to_string().contains("unknown param"));
    }
}
