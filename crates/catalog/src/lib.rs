//! The Crafting Apps the toolbox manages, as data.
//!
//! The built-in list is `catalog.toml`, embedded at compile time. The same format comes from the
//! signed remote catalog ([`Remote`], docs/architecture.md § 12), so [`Catalog::parse`] validates
//! everything and never panics on any input.
//!
//! Standalone (L0): no workspace dependencies and no I/O.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The catalog format version this build reads.
pub const SCHEMA: u32 = 1;
/// Largest catalog accepted.
pub const MAX_BYTES: usize = 256 * 1024;
/// Most apps a catalog may list.
pub const MAX_APPS: usize = 256;
/// Longest asset-name pattern of an app outside the contract.
pub const MAX_PATTERN_LEN: usize = 128;
/// What a pattern writes where the version goes.
pub const VERSION_PLACEHOLDER: &str = "{version}";

/// The catalog compiled into this build.
pub const BUILTIN: &str = include_str!("../catalog.toml");

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("catalog is not valid TOML: {0}")]
    Parse(String),
    #[error("catalog schema {found} is not supported (this build reads schema {SCHEMA})")]
    Schema { found: u32 },
    #[error("catalog: {0}")]
    Invalid(String),
    #[error("catalog is too large ({0} bytes, limit {MAX_BYTES})")]
    TooLarge(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One Crafting App.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct App {
    /// Slug used in asset names, binaries and the bundle id: `photocraft`.
    pub id: String,
    /// User-facing name: `PhotoCraft`.
    pub name: String,
    pub tagline: String,
    /// GitHub `owner/name` that publishes the releases.
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// Earlier asset-name slugs (an app that was renamed keeps its old releases).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub former_slugs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// The app icon (a PNG), when it isn't at the Crafting Apps' usual path (see [`App::icon_url`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// For an app outside the release contract: its build per `<os>-<arch>` target, as a file
    /// name with `{version}` (`"macos-universal" = "ArtCraft_{version}_universal.dmg"`). Such an
    /// app has no `SHA256SUMS.txt`; its digests come from the signed aggregated feed, and
    /// without one nothing is installed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, String>,
}

impl App {
    /// Does this app publish the contract's asset set (no [`App::assets`] patterns)?
    pub fn follows_contract(&self) -> bool {
        self.assets.is_empty()
    }

    /// Every slug its assets may start with: the id first, then former names.
    pub fn slugs(&self) -> Vec<&str> {
        std::iter::once(self.id.as_str()).chain(self.former_slugs.iter().map(String::as_str)).collect()
    }

    /// The platform id (`ai.storyteller.photocraft`): macOS bundle id, Linux desktop file and
    /// icon name, Windows AppUserModelID.
    pub fn bundle_id(&self) -> String {
        self.bundle_id.clone().unwrap_or_else(|| format!("ai.storyteller.{}", self.id))
    }

    /// `https://github.com/<repo>`.
    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }

    /// Its 128 px icon. Every craft keeps it at the same path in its repository
    /// (`assets/app-icon/hicolor/128x128/apps/<bundle id>.png`, docs/release-contract.md › Icons),
    /// served from `raw.githubusercontent.com`, which doesn't count against the API's rate limit.
    pub fn icon_url(&self) -> String {
        self.icon
            .clone()
            .unwrap_or_else(|| format!("https://raw.githubusercontent.com/{}/HEAD/assets/app-icon/hicolor/128x128/apps/{}.png", self.repo, self.bundle_id()))
    }

    /// The GitHub REST endpoint listing its releases, newest first.
    pub fn releases_api_url(&self) -> String {
        format!("https://api.github.com/repos/{}/releases", self.repo)
    }
}

/// Where the publisher puts a newer catalog and the suite's aggregated release feed, and the
/// key both are signed with (`release::signing`). The key is pinned by the built-in catalog: a
/// remote catalog may move the URLs but never change the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    /// The signed catalog envelope (`artcraft-catalog.json`).
    pub catalog: String,
    /// The signed aggregated feed envelope (`artcraft-feed.json`).
    pub feed: String,
    /// `ed25519:<base64>`.
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    /// Bumped by the publisher on every change. A remote catalog replaces the current one only
    /// when its revision is not older ([`Catalog::accepts`]).
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Remote>,
    /// The toolbox itself, as the app whose releases it updates itself from (the same contract
    /// as the apps; docs/architecture.md § 7). Not listed among [`Catalog::apps`]. A catalog
    /// without it leaves self-update off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolbox: Option<App>,
    #[serde(rename = "app", default)]
    pub apps: Vec<App>,
}

impl Catalog {
    /// The catalog compiled into this build ([`BUILTIN`]).
    pub fn builtin() -> Result<Catalog> {
        Catalog::parse(BUILTIN)
    }

    /// Parse and validate a catalog.
    pub fn parse(text: &str) -> Result<Catalog> {
        if text.len() > MAX_BYTES {
            return Err(Error::TooLarge(text.len()));
        }
        let catalog: Catalog = toml::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn get(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// An app of the list, or the toolbox itself: everything with a release feed.
    pub fn feed_app(&self, id: &str) -> Option<&App> {
        self.get(id).or_else(|| self.toolbox.as_ref().filter(|t| t.id == id))
    }

    /// May `other`, a verified remote catalog, replace this one? It must not be older, and it
    /// must keep a `[remote]` section with the same key, or the toolbox would stop following
    /// the publisher (or start following another).
    pub fn accepts(&self, other: &Catalog) -> std::result::Result<(), String> {
        if other.revision < self.revision {
            return Err(format!("revision {} is older than the current {}", other.revision, self.revision));
        }
        match (&self.remote, &other.remote) {
            (Some(mine), Some(theirs)) if mine.public_key != theirs.public_key => Err("it names another signing key; the key is pinned by this build".into()),
            (Some(_), None) => Err("it has no [remote] section".into()),
            _ => Ok(()),
        }
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::Invalid(m));
        if self.schema != SCHEMA {
            return Err(Error::Schema { found: self.schema });
        }
        if self.apps.is_empty() {
            return bad("no apps".into());
        }
        if self.apps.len() > MAX_APPS {
            return bad(format!("{} apps (limit {MAX_APPS})", self.apps.len()));
        }
        let mut slugs: Vec<&str> = Vec::new();
        for app in self.apps.iter().chain(&self.toolbox) {
            for slug in app.slugs() {
                if !valid_slug(slug) {
                    return bad(format!("`{}` is not a valid slug (a-z, 0-9 and -, starting with a letter, at most 32)", short(slug)));
                }
                if slugs.contains(&slug) {
                    return bad(format!("slug `{slug}` is used twice"));
                }
                slugs.push(slug);
            }
            let id = &app.id;
            if app.name.trim().is_empty() || app.name.len() > 64 {
                return bad(format!("{id}: name must be 1 to 64 bytes"));
            }
            if app.tagline.len() > 200 {
                return bad(format!("{id}: tagline is longer than 200 bytes"));
            }
            if !valid_repo(&app.repo) {
                return bad(format!("{id}: repo `{}` is not `owner/name`", short(&app.repo)));
            }
            if let Some(w) = &app.website
                && (!w.starts_with("https://") || w.len() > 512 || w.contains(char::is_whitespace))
            {
                return bad(format!("{id}: website must be an https:// URL"));
            }
            if let Some(i) = &app.icon
                && (!i.starts_with("https://") || i.len() > 512 || i.contains(char::is_whitespace))
            {
                return bad(format!("{id}: icon must be an https:// URL"));
            }
            if let Some(b) = &app.bundle_id
                && (b.is_empty() || b.len() > 128 || !b.split('.').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')))
            {
                return bad(format!("{id}: bundle_id `{}` is not a reverse-DNS id", short(b)));
            }
            for (target, pattern) in &app.assets {
                if !valid_target(target) {
                    return bad(format!("{id}: assets key `{}` is not `<os>-<arch>`", short(target)));
                }
                if let Err(why) = valid_pattern(pattern) {
                    return bad(format!("{id}: assets pattern `{}` {why}", short(pattern)));
                }
            }
        }
        if let Some(remote) = &self.remote {
            for (what, url) in [("catalog", &remote.catalog), ("feed", &remote.feed)] {
                if !valid_url(url) {
                    return bad(format!("remote.{what} must be an https:// URL"));
                }
            }
            if !valid_key(&remote.public_key) {
                return bad("remote.public_key must be `ed25519:<base64 of 32 bytes>`".into());
            }
        }
        Ok(())
    }
}

fn valid_url(s: &str) -> bool {
    s.starts_with("https://") && s.len() > 8 && s.len() <= 512 && !s.contains(char::is_whitespace) && !s.contains(char::is_control)
}

/// `ed25519:` and 44 base64 characters (32 bytes, padded). The key itself is parsed above L0.
fn valid_key(s: &str) -> bool {
    s.strip_prefix("ed25519:")
        .is_some_and(|b| b.len() == 44 && b.ends_with('=') && b.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=')))
}

/// `<os>-<arch>`: lowercase tokens, as in asset names (`macos-universal`, `linux-x86_64`).
fn valid_target(s: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.len() <= 16 && p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    matches!(s.split_once('-'), Some((os, arch)) if part(os) && part(arch))
}

/// A file name with `{version}` once, an extension, and nothing that could leave a folder.
fn valid_pattern(p: &str) -> std::result::Result<(), String> {
    if p.is_empty() || p.len() > MAX_PATTERN_LEN {
        return Err(format!("must be 1 to {MAX_PATTERN_LEN} bytes"));
    }
    if p.matches(VERSION_PLACEHOLDER).count() != 1 {
        return Err(format!("must contain `{VERSION_PLACEHOLDER}` exactly once"));
    }
    if p.contains(['/', '\\']) || p.contains(char::is_whitespace) || p.contains(char::is_control) || p.starts_with('.') {
        return Err("must be a plain file name".into());
    }
    let Some((_, after)) = p.rsplit_once(VERSION_PLACEHOLDER) else { return Err("must contain `{version}`".into()) };
    if !after.contains('.') || after.ends_with('.') {
        return Err("must end with a file extension".into());
    }
    Ok(())
}

fn valid_slug(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn valid_repo(s: &str) -> bool {
    let part =
        |p: &str| (1..=100).contains(&p.len()) && p != "." && p != ".." && p.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'));
    matches!(s.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

/// Echo at most 40 characters of hostile input.
fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid() {
        let c = Catalog::builtin().unwrap();
        assert_eq!(c.apps.len(), 13, "the twelve Crafting Apps and ArtCraft itself");
        assert!(c.revision >= 1);
        let remote = c.remote.as_ref().expect("a [remote] section");
        assert!(remote.catalog.ends_with("/artcraft-catalog.json") && remote.feed.ends_with("/artcraft-feed.json"));
        assert!(remote.public_key.starts_with("ed25519:"));
        // The toolbox itself follows the contract under its own slug and app id.
        let toolbox = c.toolbox.as_ref().unwrap();
        assert_eq!(
            (toolbox.id.as_str(), toolbox.name.as_str(), toolbox.bundle_id().as_str()),
            ("artcraft-toolbox", "ArtCraft Toolbox", "ai.storyteller.toolbox")
        );
        assert!(c.get("artcraft-toolbox").is_none(), "not an app of the list");
        assert_eq!(c.feed_app("artcraft-toolbox").map(|a| a.name.as_str()), Some("ArtCraft Toolbox"));
        assert_eq!(c.feed_app("photocraft").map(|a| a.name.as_str()), Some("PhotoCraft"));
        assert!(c.feed_app("nope").is_none());
        let ids: Vec<&str> = c.apps.iter().map(|a| a.id.as_str()).collect();
        for id in ["photocraft", "vectorcraft", "filmcraft", "lightcraft", "pdfcraft", "effectcraft", "designcraft"] {
            assert!(ids.contains(&id), "{id}");
        }
        for app in &c.apps {
            assert!(app.name.ends_with("Craft"), "{}: product names are {{Function}}Craft", app.name);
            assert_eq!(app.repo, format!("storytold/{}", app.id));
        }
        // ArtCraft itself is the one app outside the contract: matched by patterns.
        let artcraft = c.get("artcraft").unwrap();
        assert!(!artcraft.follows_contract() && artcraft.assets.contains_key("macos-universal"));
        assert_eq!(artcraft.bundle_id(), "ai.artcraft.app");
        assert!(c.apps.iter().filter(|a| a.id != "artcraft").all(App::follows_contract));
    }

    #[test]
    fn a_remote_catalog_is_accepted_only_when_not_older_and_with_the_same_key() {
        let current = Catalog::builtin().unwrap();
        let mut newer = current.clone();
        newer.revision += 1;
        assert_eq!(current.accepts(&newer), Ok(()));
        assert_eq!(current.accepts(&current), Ok(()), "the same revision again is harmless");
        let mut older = current.clone();
        older.revision = 0;
        assert!(current.accepts(&older).unwrap_err().contains("older"));
        let mut other_key = newer.clone();
        other_key.remote.as_mut().unwrap().public_key = "ed25519:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=".into();
        assert!(current.accepts(&other_key).unwrap_err().contains("pinned"));
        let mut no_remote = newer.clone();
        no_remote.remote = None;
        assert!(current.accepts(&no_remote).unwrap_err().contains("[remote]"));
        // Moving the URLs is fine.
        let mut moved = newer.clone();
        moved.remote.as_mut().unwrap().feed = "https://example.invalid/feed.json".into();
        assert_eq!(current.accepts(&moved), Ok(()));
    }

    #[test]
    fn patterns_and_remote_are_validated() {
        let ok = "id = \"x\"\nname = \"XCraft\"\ntagline = \"\"\nrepo = \"o/x\"";
        let with = |extra: &str| format!("schema = 1\n[[app]]\n{ok}\n{extra}\n");
        assert!(Catalog::parse(&with("[app.assets]\n\"macos-universal\" = \"X_{version}_universal.dmg\"")).is_ok());
        for (extra, want) in [
            ("[app.assets]\n\"macos\" = \"X_{version}.dmg\"", "<os>-<arch>"),
            ("[app.assets]\n\"macos-universal\" = \"X.dmg\"", "exactly once"),
            ("[app.assets]\n\"macos-universal\" = \"X_{version}_{version}.dmg\"", "exactly once"),
            ("[app.assets]\n\"macos-universal\" = \"../X_{version}.dmg\"", "plain file name"),
            ("[app.assets]\n\"macos-universal\" = \"X_{version}\"", "extension"),
            ("[app.assets]\n\"macos-universal\" = \"X {version}.dmg\"", "plain file name"),
        ] {
            let e = Catalog::parse(&with(extra)).unwrap_err().to_string();
            assert!(e.contains(want), "{extra}: {e}");
        }
        let remote = |catalog: &str, feed: &str, key: &str| {
            format!("schema = 1\n[remote]\ncatalog = \"{catalog}\"\nfeed = \"{feed}\"\npublic_key = \"{key}\"\n[[app]]\n{ok}\n")
        };
        let key = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        assert!(Catalog::parse(&remote("https://x/c.json", "https://x/f.json", key)).is_ok());
        assert!(Catalog::parse(&remote("http://x/c.json", "https://x/f.json", key)).unwrap_err().to_string().contains("remote.catalog"));
        assert!(Catalog::parse(&remote("https://x/c.json", "https://x/f json", key)).unwrap_err().to_string().contains("remote.feed"));
        assert!(Catalog::parse(&remote("https://x/c.json", "https://x/f.json", "ed25519:short")).unwrap_err().to_string().contains("public_key"));
        assert!(Catalog::parse(&remote("https://x/c.json", "https://x/f.json", "rsa:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")).is_err());
    }

    #[test]
    fn derived_fields() {
        let c = Catalog::builtin().unwrap();
        let pdf = c.get("pdfcraft").unwrap();
        assert_eq!(pdf.slugs(), ["pdfcraft", "printcraft"]);
        assert_eq!(pdf.bundle_id(), "ai.storyteller.pdfcraft");
        assert_eq!(pdf.releases_api_url(), "https://api.github.com/repos/storytold/pdfcraft/releases");
        assert_eq!(pdf.repo_url(), "https://github.com/storytold/pdfcraft");
        assert_eq!(
            pdf.icon_url(),
            "https://raw.githubusercontent.com/storytold/pdfcraft/HEAD/assets/app-icon/hicolor/128x128/apps/ai.storyteller.pdfcraft.png"
        );
        assert!(c.get("nope").is_none());
    }

    fn one(app: &str) -> String {
        format!("schema = 1\n[[app]]\n{app}\n")
    }

    #[test]
    fn rejects_invalid_catalogs() {
        let ok = "id = \"x\"\nname = \"XCraft\"\ntagline = \"\"\nrepo = \"o/x\"";
        assert!(Catalog::parse(&one(ok)).is_ok());
        let cases = [
            ("not toml [[[".to_string(), "TOML"),
            ("schema = 2\n[[app]]\nid=\"x\"\nname=\"X\"\ntagline=\"\"\nrepo=\"o/x\"".to_string(), "schema"),
            ("schema = 1".to_string(), "no apps"),
            (one(&ok.replace("id = \"x\"", "id = \"X\"")), "slug"),
            (one(&ok.replace("id = \"x\"", "id = \"../x\"")), "slug"),
            (one(&ok.replace("repo = \"o/x\"", "repo = \"o\"")), "repo"),
            (one(&ok.replace("repo = \"o/x\"", "repo = \"o/..\"")), "repo"),
            (one(&ok.replace("name = \"XCraft\"", "name = \" \"")), "name"),
            (one(&format!("{ok}\nwebsite = \"http://x\"")), "https"),
            (one(&format!("{ok}\nformer_slugs = [\"x\"]")), "twice"),
            (one(&format!("{ok}\nbundle_id = \"a..b\"")), "reverse-DNS"),
            (one(&format!("{ok}\nicon = \"http://x/i.png\"")), "icon"),
            (format!("{}{}", one(ok), "[[app]]\nid=\"x\"\nname=\"X2\"\ntagline=\"\"\nrepo=\"o/y\""), "twice"),
            // The toolbox entry is validated like an app, and can't reuse an app's slug.
            (format!("schema = 1\n[toolbox]\nid = \"x\"\nname = \"T\"\ntagline = \"\"\nrepo = \"o/t\"\n[[app]]\n{ok}\n"), "twice"),
            (format!("schema = 1\n[toolbox]\nid = \"T\"\nname = \"T\"\ntagline = \"\"\nrepo = \"o/t\"\n[[app]]\n{ok}\n"), "slug"),
            (format!("schema = 1\n[toolbox]\nid = \"t\"\nname = \"T\"\ntagline = \"\"\nrepo = \"o\"\n[[app]]\n{ok}\n"), "repo"),
        ];
        for (text, want) in cases {
            let e = Catalog::parse(&text).unwrap_err().to_string();
            assert!(e.contains(want), "{text:?}: {e}");
        }
        assert!(matches!(Catalog::parse(&" ".repeat(MAX_BYTES + 1)), Err(Error::TooLarge(_))));
        // A catalog without a toolbox entry is fine (self-update is then off).
        assert_eq!(Catalog::parse(&one(ok)).unwrap().toolbox, None);
    }
}
