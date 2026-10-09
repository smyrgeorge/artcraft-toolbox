//! HTTPS for ArtCraft Toolbox (L3, native only).
//!
//! Everything the toolbox fetches goes through one [`Transport`]: [`Client`] (ureq + rustls,
//! trusting the OS certificate store) in the apps, fakes in tests. The [`Policy`] is the security
//! boundary (AGENTS.md, "Never install what you can't verify"):
//!
//! - only the policy's hosts, over HTTPS; every redirect hop is checked against the same list
//!   before it is followed (ureq's own redirect handling is off);
//! - the `Authorization` header goes to the policy's token hosts only, never along a redirect to
//!   another host;
//! - bodies are capped while they are read, and timeouts bound every request;
//! - GitHub's rate-limit headers come back with every response, and a rate-limit refusal is its
//!   own error ([`NetError::RateLimited`]) so callers can back off.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod client;
pub mod url;

use std::time::Duration;

pub use client::Client;

/// The GitHub REST API host: release feeds come from here.
pub const GITHUB_API_HOST: &str = "api.github.com";
/// Raw repository files (app icons). Not rate-limited like the API; never sent a token.
pub const GITHUB_RAW_HOST: &str = "raw.githubusercontent.com";
/// The REST API version the toolbox is written against (`X-GitHub-Api-Version`).
pub const GITHUB_API_VERSION: &str = "2022-11-28";
/// `Accept` for GitHub REST JSON.
pub const ACCEPT_GITHUB_JSON: &str = "application/vnd.github+json";

/// Where requests may go and how long they may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Hosts requests and redirect targets may go to (exact, lowercase).
    pub hosts: Vec<String>,
    /// Hosts that may receive the `Authorization` header.
    pub token_hosts: Vec<String>,
    /// Plain `http://` allowed. Only tests against a local server set this.
    pub allow_http: bool,
    pub max_redirects: u8,
    /// Whole request, connect to last body byte.
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl Policy {
    /// What the toolbox reaches: the GitHub REST API (feeds) and raw repository files (icons).
    /// Only the API may receive a token.
    pub fn github() -> Policy {
        Policy {
            hosts: vec![GITHUB_API_HOST.into(), GITHUB_RAW_HOST.into()],
            token_hosts: vec![GITHUB_API_HOST.into()],
            allow_http: false,
            max_redirects: 3,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// `ArtCraft-Toolbox/0.1.0`: generic, never a person's name or other details (AGENTS.md rule 9).
pub fn user_agent(version: &str) -> String {
    format!("ArtCraft-Toolbox/{version}")
}

/// One GET.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub url: &'a str,
    pub accept: &'a str,
    /// Sent as `If-None-Match`; a match comes back as [`Response::NotModified`].
    pub etag: Option<&'a str>,
    /// The body is read up to this many bytes; more is [`NetError::TooLarge`].
    pub max_bytes: usize,
}

/// GitHub's rate-limit headers (`x-ratelimit-*`), when the response had them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimit {
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    /// Unix seconds when the window resets.
    pub reset_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Ok {
        body: Vec<u8>,
        etag: Option<String>,
        rate: RateLimit,
    },
    /// `304`: the `etag` sent still matches. No body.
    NotModified {
        rate: RateLimit,
    },
}

impl Response {
    pub fn rate(&self) -> RateLimit {
        match self {
            Response::Ok { rate, .. } | Response::NotModified { rate } => *rate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    #[error("`{0}` is not an address the toolbox may contact")]
    Forbidden(String),
    #[error("no connection ({0})")]
    Connect(String),
    #[error("the server took too long to answer")]
    Timeout,
    #[error("GitHub's request limit is used up")]
    RateLimited {
        /// Unix seconds when the window resets (`x-ratelimit-reset`).
        reset_at: Option<u64>,
        /// Seconds to wait (`retry-after`, secondary limits).
        retry_after: Option<u64>,
    },
    #[error("HTTP {status}: {message}")]
    Status { status: u16, message: String },
    #[error("the response is larger than {0} bytes")]
    TooLarge(usize),
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("secure connection failed ({0})")]
    Tls(String),
    #[error("{0}")]
    Other(String),
}

/// Something that can GET under a [`Policy`]. `Send + Sync` so jobs can share one across worker
/// threads.
pub trait Transport: Send + Sync {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError>;
}

/// The error for a non-success status: a rate-limit refusal (`429`, or `403` with no requests
/// left, a `retry-after`, or GitHub's rate-limit message) or a plain status with GitHub's
/// `message` (or the start of the body).
pub fn classify(status: u16, rate: &RateLimit, retry_after: Option<u64>, body: &[u8]) -> NetError {
    let message = error_message(body);
    let limited = status == 429 || (status == 403 && (rate.remaining == Some(0) || retry_after.is_some() || message.to_lowercase().contains("rate limit")));
    if limited { NetError::RateLimited { reset_at: rate.reset_at, retry_after } } else { NetError::Status { status, message } }
}

/// GitHub's `{"message": "..."}`, else the first line of the body; at most 200 characters.
fn error_message(body: &[u8]) -> String {
    let from_json = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string));
    let text = from_json.unwrap_or_else(|| String::from_utf8_lossy(body).lines().next().unwrap_or_default().to_string());
    let text: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    if text.trim().is_empty() { "no details".into() } else { text }
}

/// Rate-limit headers, from a header lookup (`name` is lowercase).
pub fn parse_rate(header: impl Fn(&str) -> Option<String>) -> RateLimit {
    let num = |name: &str| header(name).and_then(|v| v.trim().parse::<u64>().ok());
    RateLimit {
        limit: num("x-ratelimit-limit").and_then(|n| u32::try_from(n).ok()),
        remaining: num("x-ratelimit-remaining").and_then(|n| u32::try_from(n).ok()),
        reset_at: num("x-ratelimit-reset"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(remaining: Option<u32>) -> RateLimit {
        RateLimit { limit: Some(60), remaining, reset_at: Some(1_791_528_397) }
    }

    #[test]
    fn rate_limit_refusals_are_recognised() {
        let limited = NetError::RateLimited { reset_at: Some(1_791_528_397), retry_after: None };
        assert_eq!(classify(403, &rate(Some(0)), None, b"{}"), limited);
        assert_eq!(classify(429, &rate(Some(5)), None, b""), limited);
        assert_eq!(
            classify(403, &rate(None), None, br#"{"message":"API rate limit exceeded for 1.2.3.4."}"#),
            NetError::RateLimited { reset_at: Some(1_791_528_397), retry_after: None }
        );
        assert_eq!(classify(403, &rate(Some(10)), Some(60), b""), NetError::RateLimited { reset_at: Some(1_791_528_397), retry_after: Some(60) });
        // A 403 with requests left and no rate-limit wording is a plain refusal.
        assert_eq!(classify(403, &rate(Some(10)), None, br#"{"message":"Forbidden"}"#), NetError::Status { status: 403, message: "Forbidden".into() });
    }

    #[test]
    fn error_messages_are_short_and_clean() {
        assert_eq!(classify(404, &RateLimit::default(), None, br#"{"message":"Not Found","documentation_url":"x"}"#).to_string(), "HTTP 404: Not Found");
        assert_eq!(classify(500, &RateLimit::default(), None, b"").to_string(), "HTTP 500: no details");
        assert_eq!(classify(502, &RateLimit::default(), None, b"bad gateway\nmore").to_string(), "HTTP 502: bad gateway");
        let long = "x".repeat(10_000);
        assert!(classify(500, &RateLimit::default(), None, long.as_bytes()).to_string().len() < 220);
        assert_eq!(classify(500, &RateLimit::default(), None, b"a\x1b[31mb").to_string(), "HTTP 500: a[31mb");
        assert_eq!(classify(500, &RateLimit::default(), None, &[0xff, 0xfe]).to_string(), "HTTP 500: \u{fffd}\u{fffd}");
    }

    #[test]
    fn rate_headers() {
        let h = |name: &str| match name {
            "x-ratelimit-limit" => Some("60".to_string()),
            "x-ratelimit-remaining" => Some(" 44 ".to_string()),
            "x-ratelimit-reset" => Some("1791528397".to_string()),
            _ => None,
        };
        assert_eq!(parse_rate(h), RateLimit { limit: Some(60), remaining: Some(44), reset_at: Some(1_791_528_397) });
        assert_eq!(parse_rate(|_| Some("-1".into())), RateLimit::default());
        assert_eq!(parse_rate(|_| Some("99999999999999999999".into())), RateLimit::default());
    }

    #[test]
    fn user_agent_is_generic() {
        assert_eq!(user_agent("0.1.0"), "ArtCraft-Toolbox/0.1.0");
    }
}
