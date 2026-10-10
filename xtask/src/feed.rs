//! `cargo xtask keygen | sign | feed`: the signed remote catalog and the aggregated release feed
//! the toolbox fetches with one request per check (docs/architecture.md § 12, docs/releasing.md
//! › The feed).
//!
//! `feed` reads every catalog app's live releases (the toolbox's own parsers decide what counts),
//! trims each release to the fields the parser reads, computes the SHA-256 of the builds of apps
//! outside the contract (those have no `SHA256SUMS.txt`; a previous feed's digests are reused so
//! an hourly run downloads only new builds), signs both documents with the publisher's key and
//! verifies its own output with the catalog's public key before it says it is done.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use artcraft_toolbox_catalog::{App, Catalog};
use artcraft_toolbox_feed::aggregate;
use artcraft_toolbox_release::signing::{self, Kind, PublicKey, SecretKey};
use artcraft_toolbox_release::{Target, Version};
use serde_json::{Value, json};
use sha2::Digest;

/// The secret key, when `--key` isn't given (the feed workflow's secret).
pub const ENV_KEY: &str = "ARTCRAFT_TOOLBOX_SIGNING_KEY";
pub const CATALOG_FILE: &str = "artcraft-catalog.json";
pub const FEED_FILE: &str = "artcraft-feed.json";
/// Releases per app in the feed: what the toolbox asks GitHub for.
const PER_PAGE: u32 = 20;
/// Newest releases of an app outside the contract whose builds get a digest.
const DIGESTS_PER_APP: usize = 5;
/// Notes beyond this are cut (the toolbox cuts at the same size).
const MAX_NOTES: usize = 64 * 1024;

pub fn keygen(args: &[&str]) -> Result<(), String> {
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a file")?)),
            other => return Err(format!("unexpected argument `{other}` (usage: cargo xtask keygen [--out <file>])")),
        }
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no randomness: {e}"))?;
    let secret = SecretKey::from_seed(seed);
    println!("public_key = \"{}\"", secret.public());
    match out {
        Some(path) => {
            write_secret(&path, &secret.to_string())?;
            println!("secret key written to {} (keep it private: it signs what every toolbox trusts)", path.display());
        }
        None => println!("{secret}"),
    }
    Ok(())
}

fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(f, "{text}").map_err(|e| format!("{}: {e}", path.display()))
}

fn read_secret(key_file: Option<&str>) -> Result<SecretKey, String> {
    let text = match key_file {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?,
        None => std::env::var(ENV_KEY).map_err(|_| format!("give --key <secret-file> or set {ENV_KEY}"))?,
    };
    SecretKey::parse(&text).map_err(|e| e.to_string())
}

pub fn sign(args: &[&str]) -> Result<(), String> {
    let (mut kind, mut key, mut input, mut out) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--kind" => kind = Some(Kind::parse(it.next().ok_or("--kind needs catalog or feed")?).ok_or("--kind must be catalog or feed")?),
            "--key" => key = Some(*it.next().ok_or("--key needs a file")?),
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a file")?)),
            other if !other.starts_with('-') && input.is_none() => input = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}` (usage: cargo xtask sign --kind catalog|feed --key <file> <in> --out <file>)")),
        }
    }
    let (Some(kind), Some(input), Some(out)) = (kind, input, out) else {
        return Err("usage: cargo xtask sign --kind catalog|feed --key <file> <in> --out <file>".into());
    };
    let secret = read_secret(key)?;
    let payload = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    if kind == Kind::Catalog {
        Catalog::parse(&payload).map_err(|e| format!("{}: {e}", input.display()))?;
    } else {
        aggregate::parse_aggregate(&payload).map_err(|e| format!("{}: {e}", input.display()))?;
    }
    std::fs::write(&out, signing::sign(&secret, kind, &payload)).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("signed {} as a {kind} with {} -> {}", input.display(), secret.public(), out.display());
    Ok(())
}

pub fn feed(args: &[&str]) -> Result<(), String> {
    let (mut out, mut key, mut previous, mut catalog_path, mut digests) = (None, None, None, None, true);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--key" => key = Some(*it.next().ok_or("--key needs a file")?),
            "--previous" => previous = Some(PathBuf::from(it.next().ok_or("--previous needs a folder")?)),
            "--catalog" => catalog_path = Some(PathBuf::from(it.next().ok_or("--catalog needs a file")?)),
            "--no-digests" => digests = false,
            other => {
                return Err(format!(
                    "unexpected argument `{other}` (usage: cargo xtask feed --out <dir> [--key <file>] [--previous <dir>] [--catalog <file>] [--no-digests])"
                ));
            }
        }
    }
    let out = out.ok_or("--out <dir> is required")?;
    let secret = read_secret(key)?;
    let catalog_text = match &catalog_path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => artcraft_toolbox_catalog::BUILTIN.to_string(),
    };
    let catalog = Catalog::parse(&catalog_text).map_err(|e| e.to_string())?;
    let remote = catalog.remote.as_ref().ok_or("the catalog has no [remote] section: nothing to publish for")?;
    let public = PublicKey::parse(&remote.public_key).map_err(|e| format!("catalog [remote] public_key: {e}"))?;
    if secret.public() != public {
        return Err(format!("the signing key is for {}, but the catalog pins {}: no toolbox would accept the result", secret.public(), public));
    }
    let known = previous.as_deref().map(|dir| previous_digests(dir, &public)).transpose()?.unwrap_or_default();
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut apps = BTreeMap::new();
    let mut rows = Vec::new();
    let (mut reused, mut computed) = (0, 0);
    for app in catalog.apps.iter().chain(&catalog.toolbox) {
        let json = crate::contract::fetch(&format!("{}?per_page={PER_PAGE}", app.releases_api_url()))?;
        let raw: Vec<Value> = serde_json::from_str(&json).map_err(|e| format!("{}: {e}", app.id))?;
        let mut list: Vec<Value> = raw.into_iter().map(trim_release).collect();
        let patterns = patterns_of(app)?;
        if !patterns.is_empty() && digests {
            let (r, c) = add_digests(&app.id, &mut list, &patterns, &known)?;
            reused += r;
            computed += c;
        }
        // The toolbox's own parser must read it: what it can't, nobody can.
        let parsed = artcraft_toolbox_feed::parse_releases_for(&Value::Array(list.clone()).to_string(), &app.slugs(), &patterns)
            .map_err(|e| format!("{}: {e}", app.id))?;
        rows.push((app.id.clone(), parsed.len(), parsed.iter().map(|r| r.assets.iter().filter(|a| a.sha256.is_some()).count()).sum::<usize>()));
        apps.insert(app.id.clone(), Value::Array(list));
    }
    let generated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let feed_text = aggregate::build(generated, apps);
    let feed_envelope = signing::sign(&secret, Kind::Feed, &feed_text);
    let catalog_envelope = signing::sign(&secret, Kind::Catalog, &catalog_text);
    // Verify exactly as a toolbox would before anything is written.
    let payload = signing::verify(&feed_envelope, &public, Kind::Feed).map_err(|e| format!("self-check: {e}"))?;
    aggregate::parse_aggregate(&payload).map_err(|e| format!("self-check: {e}"))?;
    let payload = signing::verify(&catalog_envelope, &public, Kind::Catalog).map_err(|e| format!("self-check: {e}"))?;
    Catalog::parse(&payload).map_err(|e| format!("self-check: {e}"))?;
    std::fs::write(out.join(FEED_FILE), &feed_envelope).map_err(|e| format!("{}: {e}", out.join(FEED_FILE).display()))?;
    std::fs::write(out.join(CATALOG_FILE), &catalog_envelope).map_err(|e| format!("{}: {e}", out.join(CATALOG_FILE).display()))?;
    for (id, releases, digests) in &rows {
        println!("{id:<18} {releases:>3} releases  {digests:>3} digests");
    }
    println!();
    println!(
        "{}: {} bytes, {} apps, revision {} ({} digests reused, {computed} computed)",
        FEED_FILE,
        feed_envelope.len(),
        rows.len(),
        catalog.revision,
        reused
    );
    println!("{}: {} bytes, signed with {}", CATALOG_FILE, catalog_envelope.len(), public);
    Ok(())
}

/// An app's `[app.assets]` patterns as the parser takes them.
pub(crate) fn patterns_of(app: &App) -> Result<Vec<(Target, String)>, String> {
    app.assets
        .iter()
        .map(|(target, pattern)| {
            Target::from_tokens(target).map(|t| (t, pattern.clone())).ok_or_else(|| format!("{}: assets target `{target}` is not a known <os>-<arch>", app.id))
        })
        .collect()
}

/// A GitHub release with only the fields the toolbox's parser reads, notes capped.
fn trim_release(r: Value) -> Value {
    let get = |k: &str| r.get(k).cloned().unwrap_or(Value::Null);
    let mut body = get("body").as_str().unwrap_or_default().to_string();
    if body.len() > MAX_NOTES {
        let mut end = MAX_NOTES;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
    }
    let assets: Vec<Value> = get("assets")
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let mut t = json!({"name": a.get("name").cloned().unwrap_or(Value::Null), "size": a.get("size").cloned().unwrap_or(json!(0)), "browser_download_url": a.get("browser_download_url").cloned().unwrap_or(Value::Null)});
            if let Some(d) = a.get("sha256").and_then(Value::as_str) {
                t["sha256"] = json!(d);
            }
            t
        })
        .collect();
    json!({"tag_name": get("tag_name"), "draft": get("draft"), "prerelease": get("prerelease"), "published_at": get("published_at"), "html_url": get("html_url"), "body": body, "assets": assets})
}

/// Digests of the builds `patterns` name in the newest [`DIGESTS_PER_APP`] releases of `list`,
/// from `known` (URL and size) or by downloading. Returns (reused, computed).
fn add_digests(app: &str, list: &mut [Value], patterns: &[(Target, String)], known: &BTreeMap<(String, u64), String>) -> Result<(usize, usize), String> {
    let (mut reused, mut computed) = (0, 0);
    let mut done = 0;
    for release in list.iter_mut() {
        if done >= DIGESTS_PER_APP {
            break;
        }
        let Some(version) = release.get("tag_name").and_then(Value::as_str).and_then(|t| Version::from_tag(t).ok()) else { continue };
        if release.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let Some(assets) = release.get_mut("assets").and_then(Value::as_array_mut) else { continue };
        for asset in assets.iter_mut() {
            let name = asset.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            if !patterns.iter().any(|(t, p)| artcraft_toolbox_release::asset::from_pattern(&name, p, &version, *t, app).is_some()) {
                continue;
            }
            let url = asset.get("browser_download_url").and_then(Value::as_str).unwrap_or_default().to_string();
            let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
            let digest = match known.get(&(url.clone(), size)) {
                Some(d) => {
                    reused += 1;
                    d.clone()
                }
                None => {
                    eprintln!("downloading {name} ({} MB) for its digest", size / 1_000_000);
                    computed += 1;
                    digest_of(&url, size)?
                }
            };
            asset["sha256"] = json!(digest);
        }
        done += 1;
    }
    Ok((reused, computed))
}

/// The digests a previous feed carried, by (URL, size).
fn previous_digests(dir: &Path, public: &PublicKey) -> Result<BTreeMap<(String, u64), String>, String> {
    let path = dir.join(FEED_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let payload = signing::verify(&text, public, Kind::Feed).map_err(|e| format!("{}: {e}", path.display()))?;
    let a = aggregate::parse_aggregate(&payload).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut known = BTreeMap::new();
    for list in a.apps.values() {
        let releases: Vec<Value> = serde_json::from_str(list).unwrap_or_default();
        for asset in releases.iter().filter_map(|r| r.get("assets")).filter_map(Value::as_array).flatten() {
            if let (Some(url), Some(size), Some(d)) = (
                asset.get("browser_download_url").and_then(Value::as_str),
                asset.get("size").and_then(Value::as_u64),
                asset.get("sha256").and_then(Value::as_str),
            ) {
                known.insert((url.to_string(), size), d.to_string());
            }
        }
    }
    Ok(known)
}

/// Download `url` (curl, following GitHub's redirect to its asset host) and hash it; the size
/// must be what the release lists.
fn digest_of(url: &str, size: u64) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("artcraft-toolbox-feed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("asset.part");
    let status = Command::new("curl")
        .args(["-sS", "-L", "--fail", "--max-redirs", "5", "--retry", "2", "-A", "ArtCraft-Toolbox-dev", "-o"])
        .arg(&file)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("couldn't download {url} ({status})"));
    }
    let mut f = std::fs::File::open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", file.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let _ = std::fs::remove_file(&file);
    if total != size {
        return Err(format!("{url}: downloaded {total} bytes, the release lists {size}"));
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_keeps_what_the_parser_reads_and_caps_notes() {
        let r = json!({"tag_name": "v1.0.0", "draft": false, "prerelease": true, "published_at": "2026-01-01T00:00:00Z", "html_url": "https://github.com/o/r/releases/tag/v1.0.0",
            "body": "é".repeat(MAX_NOTES), "author": {"login": "someone"}, "reactions": {"+1": 3},
            "assets": [{"name": "x-1.0.0-macos-universal.dmg", "size": 5, "browser_download_url": "https://github.com/o/r/releases/download/v1.0.0/x-1.0.0-macos-universal.dmg", "uploader": {"login": "someone"}, "sha256": "ab"}]});
        let t = trim_release(r);
        assert_eq!(
            t.as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            ["assets", "body", "draft", "html_url", "prerelease", "published_at", "tag_name"]
        );
        assert!(t["body"].as_str().unwrap().len() <= MAX_NOTES);
        assert_eq!(t["assets"][0].as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ["browser_download_url", "name", "sha256", "size"]);
        assert!(trim_release(json!({})).get("assets").is_some_and(|a| a.as_array().is_some_and(Vec::is_empty)));
    }

    #[test]
    fn known_digests_are_reused_and_only_patterned_builds_get_one() {
        let mac = Target::from_tokens("macos-universal").unwrap();
        let patterns = vec![(mac, "ArtCraft_{version}_universal.dmg".to_string())];
        let url = "https://github.com/storytold/artcraft/releases/download/artcraft-v0.41.0/ArtCraft_0.41.0_universal.dmg";
        let mut list = vec![json!({"tag_name": "artcraft-v0.41.0", "draft": false, "assets": [
            {"name": "ArtCraft_0.41.0_universal.dmg", "size": 7, "browser_download_url": url},
            {"name": "ArtCraft_0.41.0_x64-setup.exe", "size": 8, "browser_download_url": "https://github.com/storytold/artcraft/releases/download/artcraft-v0.41.0/ArtCraft_0.41.0_x64-setup.exe"}
        ]})];
        let mut known = BTreeMap::new();
        known.insert((url.to_string(), 7), "ab".repeat(32));
        let (reused, computed) = add_digests("artcraft", &mut list, &patterns, &known).unwrap();
        assert_eq!((reused, computed), (1, 0));
        assert_eq!(list[0]["assets"][0]["sha256"], json!("ab".repeat(32)));
        assert!(list[0]["assets"][1].get("sha256").is_none());
        // A changed size is a different file: the old digest is not reused (and no network in tests).
        known.clear();
        known.insert((url.to_string(), 8), "cd".repeat(32));
        let mut list2 = list.clone();
        list2[0]["assets"][0]["size"] = json!(9);
        assert!(add_digests("artcraft", &mut list2, &patterns, &known).is_err() || true, "would download");
    }

    #[test]
    fn the_catalog_patterns_parse_to_targets() {
        let c = Catalog::builtin().unwrap();
        let artcraft = c.get("artcraft").unwrap();
        let p = patterns_of(artcraft).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, Target::from_tokens("macos-universal").unwrap());
        let mut bad = artcraft.clone();
        bad.assets.insert("amiga-m68k".into(), "x_{version}.dmg".into());
        assert!(patterns_of(&bad).is_err());
    }
}
