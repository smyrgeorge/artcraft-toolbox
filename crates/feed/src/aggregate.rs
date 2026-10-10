//! The suite's aggregated release feed: every app's GitHub "list releases" response (trimmed to
//! what the parser reads) in one document, published signed by `cargo xtask feed` and fetched
//! with one request per check instead of one per app (docs/architecture.md § 12).
//!
//! ```json
//! {"schema": 1, "generated": 1760000000,
//!  "apps": {"photocraft": [ <GitHub release objects> ], "artcraft": [ ... ]}}
//! ```
//!
//! Each app's list is exactly what GitHub's API returns for it (minus unused fields), so
//! [`crate::parse_releases_for`] reads it unchanged and the toolbox caches it as it caches a
//! direct response. For apps outside the contract the generator adds `sha256` to each asset it
//! matched, which is how those get verified at install.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// The format this build reads.
pub const SCHEMA: u32 = 1;
/// Largest aggregated feed accepted (13 apps' trimmed feeds are a few hundred KB).
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Most apps in one feed.
pub const MAX_APPS: usize = 256;

#[derive(Serialize, Deserialize)]
struct Raw {
    schema: u32,
    #[serde(default)]
    generated: u64,
    apps: BTreeMap<String, Value>,
}

/// A parsed aggregated feed: each app's release list, as JSON text the GitHub parser reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    /// Unix seconds when the publisher built it.
    pub generated: u64,
    /// By app id: the GitHub-format release list, serialized.
    pub apps: BTreeMap<String, String>,
}

/// Parse and shape-check an aggregated feed (the payload of a verified envelope).
pub fn parse_aggregate(text: &str) -> Result<Aggregate> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge(text.len()));
    }
    let raw: Raw = serde_json::from_str(text).map_err(|e| Error::Json(e.to_string()))?;
    if raw.schema != SCHEMA {
        return Err(Error::Aggregate(format!("schema {} is not supported (this build reads schema {SCHEMA})", raw.schema)));
    }
    if raw.apps.len() > MAX_APPS {
        return Err(Error::Aggregate(format!("{} apps (limit {MAX_APPS})", raw.apps.len())));
    }
    let mut apps = BTreeMap::new();
    for (id, list) in raw.apps {
        if !list.is_array() {
            return Err(Error::Aggregate(format!("`{}`: not a release list", id.chars().take(40).collect::<String>())));
        }
        let text = serde_json::to_string(&list).map_err(|e| Error::Json(e.to_string()))?;
        apps.insert(id, text);
    }
    Ok(Aggregate { generated: raw.generated, apps })
}

/// The document the publisher writes: `apps` maps app ids to GitHub-format release lists.
pub fn build(generated: u64, apps: BTreeMap<String, Value>) -> String {
    // Strings, numbers and JSON values always serialize.
    serde_json::to_string(&Raw { schema: SCHEMA, generated, apps }).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PHOTOCRAFT: &str = include_str!("../tests/fixtures/photocraft-releases.json");

    #[test]
    fn round_trips_and_feeds_the_release_parser() {
        let mut apps = BTreeMap::new();
        apps.insert("photocraft".to_string(), serde_json::from_str::<Value>(PHOTOCRAFT).unwrap());
        apps.insert("vectorcraft".to_string(), json!([]));
        let text = build(1_760_000_000, apps);
        let a = parse_aggregate(&text).unwrap();
        assert_eq!(a.generated, 1_760_000_000);
        assert_eq!(a.apps.keys().collect::<Vec<_>>(), ["photocraft", "vectorcraft"]);
        let releases = crate::parse_releases(&a.apps["photocraft"], &["photocraft"]).unwrap();
        assert_eq!(releases.len(), crate::parse_releases(PHOTOCRAFT, &["photocraft"]).unwrap().len());
        assert_eq!(crate::parse_releases(&a.apps["vectorcraft"], &["vectorcraft"]).unwrap(), []);
    }

    #[test]
    fn hostile_feeds_are_errors_not_panics() {
        for bad in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"schema":1}"#,
            r#"{"schema":2,"apps":{}}"#,
            r#"{"schema":1,"apps":[]}"#,
            r#"{"schema":1,"apps":{"x":{}}}"#,
            r#"{"schema":1,"apps":{"x":"[]"}}"#,
        ] {
            assert!(parse_aggregate(bad).is_err(), "{bad}");
        }
        assert!(matches!(parse_aggregate(&" ".repeat(MAX_BYTES + 1)), Err(Error::TooLarge(_))));
        let many: BTreeMap<String, Value> = (0..=MAX_APPS).map(|i| (format!("a{i}"), json!([]))).collect();
        assert!(parse_aggregate(&build(0, many)).unwrap_err().to_string().contains("limit"));
        assert!(parse_aggregate(r#"{"schema":1,"apps":{}}"#).unwrap().apps.is_empty());
    }
}
