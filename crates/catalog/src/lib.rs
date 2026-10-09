//! The Crafting Apps the toolbox manages, as data.
//!
//! The built-in list is `catalog.toml`, embedded at compile time. The same format can later come
//! from a signed remote catalog (docs/roadmap.md), so [`Catalog::parse`] validates everything and
//! never panics on any input.
//!
//! Standalone (L0): no workspace dependencies and no I/O.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};

/// The catalog format version this build reads.
pub const SCHEMA: u32 = 1;
/// Largest catalog accepted.
pub const MAX_BYTES: usize = 256 * 1024;
/// Most apps a catalog may list.
pub const MAX_APPS: usize = 256;

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
}

impl App {
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

    /// The GitHub REST endpoint listing its releases, newest first.
    pub fn releases_api_url(&self) -> String {
        format!("https://api.github.com/repos/{}/releases", self.repo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
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
        for app in &self.apps {
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
            if let Some(b) = &app.bundle_id
                && (b.is_empty() || b.len() > 128 || !b.split('.').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')))
            {
                return bad(format!("{id}: bundle_id `{}` is not a reverse-DNS id", short(b)));
            }
        }
        Ok(())
    }
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
        assert_eq!(c.apps.len(), 12);
        let ids: Vec<&str> = c.apps.iter().map(|a| a.id.as_str()).collect();
        for id in ["photocraft", "vectorcraft", "filmcraft", "lightcraft", "pdfcraft", "effectcraft", "designcraft"] {
            assert!(ids.contains(&id), "{id}");
        }
        for app in &c.apps {
            assert!(app.name.ends_with("Craft"), "{}: product names are {{Function}}Craft", app.name);
            assert_eq!(app.repo, format!("storytold/{}", app.id));
        }
    }

    #[test]
    fn derived_fields() {
        let c = Catalog::builtin().unwrap();
        let pdf = c.get("pdfcraft").unwrap();
        assert_eq!(pdf.slugs(), ["pdfcraft", "printcraft"]);
        assert_eq!(pdf.bundle_id(), "ai.storyteller.pdfcraft");
        assert_eq!(pdf.releases_api_url(), "https://api.github.com/repos/storytold/pdfcraft/releases");
        assert_eq!(pdf.repo_url(), "https://github.com/storytold/pdfcraft");
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
            (format!("{}{}", one(ok), "[[app]]\nid=\"x\"\nname=\"X2\"\ntagline=\"\"\nrepo=\"o/y\""), "twice"),
        ];
        for (text, want) in cases {
            let e = Catalog::parse(&text).unwrap_err().to_string();
            assert!(e.contains(want), "{text:?}: {e}");
        }
        assert!(matches!(Catalog::parse(&" ".repeat(MAX_BYTES + 1)), Err(Error::TooLarge(_))));
    }
}
