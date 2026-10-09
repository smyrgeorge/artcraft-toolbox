//! GitHub's REST "list releases" response (`GET /repos/{owner}/{repo}/releases`) to [`Release`]s.

use artcraft_toolbox_release::{Version, asset};
use serde::Deserialize;

use crate::{Asset, Error, Release, Result};

/// Largest response accepted. 30 releases with long notes are a few hundred KB.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Most releases read from one response.
pub const MAX_RELEASES: usize = 1000;
/// Most assets read per release.
pub const MAX_ASSETS: usize = 500;
/// Release notes beyond this are cut.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;
/// Every download must come from here: a feed can't send the toolbox to another host.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/";

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

/// Parse a "list releases" response for an app published under `slugs` (its id first, then
/// former names). Drafts and releases whose tag is not a version are skipped. The result is
/// newest first.
pub fn parse_releases(json: &str, slugs: &[&str]) -> Result<Vec<Release>> {
    if json.len() > MAX_RESPONSE_BYTES {
        return Err(Error::TooLarge(json.len()));
    }
    let raw: Vec<GhRelease> = serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))?;
    let mut out: Vec<Release> = raw.into_iter().take(MAX_RELEASES).filter(|r| !r.draft).filter_map(|r| convert(r, slugs)).collect();
    out.sort_by(|a, b| b.version.cmp(&a.version));
    out.dedup_by(|a, b| a.version == b.version);
    Ok(out)
}

fn convert(r: GhRelease, slugs: &[&str]) -> Option<Release> {
    let version = Version::from_tag(&r.tag_name).ok()?;
    let mut assets = Vec::new();
    let mut checksums_url = None;
    let mut unrecognized = Vec::new();
    for a in r.assets.into_iter().take(MAX_ASSETS) {
        if !a.browser_download_url.starts_with(DOWNLOAD_PREFIX) || a.browser_download_url.len() > 2048 {
            unrecognized.push(a.name);
            continue;
        }
        if a.name == "SHA256SUMS.txt" {
            checksums_url = Some(a.browser_download_url);
            continue;
        }
        match asset::parse(&a.name, slugs) {
            Some(name) if name.version == version => assets.push(Asset { name, file: a.name, size: a.size, url: a.browser_download_url }),
            _ => unrecognized.push(a.name),
        }
    }
    let mut notes = r.body.unwrap_or_default();
    if notes.len() > MAX_NOTES_BYTES {
        let mut end = MAX_NOTES_BYTES;
        while end > 0 && !notes.is_char_boundary(end) {
            end -= 1;
        }
        notes.truncate(end);
    }
    Some(Release {
        version,
        tag: r.tag_name,
        prerelease: r.prerelease,
        published_at: r.published_at,
        page_url: r.html_url,
        notes,
        assets,
        checksums_url,
        unrecognized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use artcraft_toolbox_release::{Os, PackageKind};
    use serde_json::json;

    fn release(tag: &str, assets: &[&str]) -> serde_json::Value {
        json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": assets.iter().map(|n| json!({"name": n, "size": 1, "browser_download_url": format!("https://github.com/storytold/photocraft/releases/download/{tag}/{n}")})).collect::<Vec<_>>()
        })
    }

    #[test]
    fn skips_drafts_and_non_version_tags_and_sorts_newest_first() {
        let mut draft = release("v9.0.0", &[]);
        draft["draft"] = json!(true);
        let feed = json!([release("v0.3.0", &[]), draft, release("nightly", &[]), release("v0.5.0", &[]), release("v0.4.0", &[])]).to_string();
        let r = parse_releases(&feed, &["photocraft"]).unwrap();
        let tags: Vec<&str> = r.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, ["v0.5.0", "v0.4.0", "v0.3.0"]);
    }

    #[test]
    fn sorts_assets_into_known_checksums_and_unrecognized() {
        let mut r = release("v0.5.0", &["photocraft-0.5.0-macos-universal.dmg", "SHA256SUMS.txt", "notes.pdf", "photocraft-0.4.0-linux-x86_64.AppImage"]);
        r["assets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "photocraft-0.5.0-linux-x86_64.AppImage", "size": 5, "browser_download_url": "https://evil.example/photocraft.AppImage"}));
        let out = parse_releases(&json!([r]).to_string(), &["photocraft"]).unwrap();
        let rel = &out[0];
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(rel.assets[0].name.target.map(|t| t.os), Some(Os::Macos));
        assert!(rel.checksums_url.as_deref().is_some_and(|u| u.ends_with("/SHA256SUMS.txt")));
        // A foreign file, an asset whose version disagrees with the tag, and an off-GitHub URL.
        assert_eq!(rel.unrecognized, ["notes.pdf", "photocraft-0.4.0-linux-x86_64.AppImage", "photocraft-0.5.0-linux-x86_64.AppImage"]);
    }

    #[test]
    fn caps_notes_at_a_char_boundary() {
        let mut r = release("v1.0.0", &[]);
        r["body"] = json!("é".repeat(MAX_NOTES_BYTES));
        let out = parse_releases(&json!([r]).to_string(), &["photocraft"]).unwrap();
        assert!(out[0].notes.len() <= MAX_NOTES_BYTES);
        assert!(out[0].notes.chars().all(|c| c == 'é'));
    }

    #[test]
    fn hostile_responses_are_errors_not_panics() {
        for bad in ["", "{}", "[1]", "null", r#"[{"tag_name": 5}]"#, r#"[{"tag_name":"v1.0.0","assets":[{"name":"x"}]}]"#, "[[[[[[[[[[[[[[[[[[[["] {
            assert!(matches!(parse_releases(bad, &["photocraft"]), Err(Error::Json(_))), "{bad}");
        }
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(parse_releases(&deep, &["photocraft"]).is_err());
        assert!(matches!(parse_releases(&" ".repeat(MAX_RESPONSE_BYTES + 1), &["x"]), Err(Error::TooLarge(_))));
        assert!(parse_releases("[]", &["photocraft"]).unwrap().is_empty());
    }

    #[test]
    fn duplicate_versions_keep_one() {
        let feed = json!([release("v1.0.0", &[]), release("1.0.0", &[])]).to_string();
        assert_eq!(parse_releases(&feed, &["photocraft"]).unwrap().len(), 1);
    }

    #[test]
    fn kinds_survive_parsing() {
        let r = parse_releases(&json!([release("v0.5.0", &["photocraft-0.5.0-windows-x64-portable.zip"])]).to_string(), &["photocraft"]).unwrap();
        assert_eq!(r[0].assets[0].name.kind, PackageKind::PortableZip);
    }
}
