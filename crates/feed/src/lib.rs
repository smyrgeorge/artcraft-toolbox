//! Release feeds: what each app has published, and what that means for this machine.
//!
//! [`github::parse_releases`] turns a GitHub "list releases" response into typed [`Release`]s;
//! [`status::status`] compares them with what is installed. Pure: the bytes come from the network
//! crate (or a test fixture), never from here. Responses are untrusted input: sizes are capped,
//! download URLs must point at GitHub, and nothing panics.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod github;
pub mod status;

use artcraft_toolbox_release::{AssetName, Version};
use serde::{Deserialize, Serialize};

pub use github::parse_releases;
pub use status::{Status, latest, status};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("release feed is not valid JSON: {0}")]
    Json(String),
    #[error("release feed is too large ({0} bytes, limit {max})", max = github::MAX_RESPONSE_BYTES)]
    TooLarge(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One published release of one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: Version,
    pub tag: String,
    /// GitHub's pre-release flag (a `-rc.1` version is a pre-release whatever the flag says).
    pub prerelease: bool,
    /// RFC 3339, as GitHub reports it.
    pub published_at: Option<String>,
    /// The release page, for "What's new".
    pub page_url: Option<String>,
    /// Release notes (Markdown), capped at [`github::MAX_NOTES_BYTES`].
    pub notes: String,
    pub assets: Vec<Asset>,
    /// `SHA256SUMS.txt`, when the release has one.
    pub checksums_url: Option<String>,
    /// Asset names that don't follow the contract or point outside GitHub (kept for diagnostics).
    pub unrecognized: Vec<String>,
}

impl Release {
    /// Is this a pre-release, by flag or by version?
    pub fn is_prerelease(&self) -> bool {
        self.prerelease || self.version.is_prerelease()
    }
}

/// A downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: AssetName,
    /// The file name as published.
    pub file: String,
    pub size: u64,
    /// `https://github.com/<owner>/<repo>/releases/download/<tag>/<file>`.
    pub url: String,
}
