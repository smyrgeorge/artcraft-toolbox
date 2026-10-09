//! Just enough URL handling to enforce a [`Policy`]: scheme, host and port of absolute
//! `http(s)://` URLs, and resolving a redirect's `Location`. Anything unusual is refused rather
//! than interpreted: userinfo (`user@host`), IPv6 literals, other schemes, control characters.

use crate::{NetError, Policy};

/// Longest URL accepted.
pub const MAX_URL_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    /// Lowercase.
    pub host: String,
    pub port: Option<u16>,
    /// From the first `/` (or `/` when absent).
    pub path: String,
}

impl Url {
    /// Parse an absolute `http://` or `https://` URL. `None` for anything else.
    pub fn parse(s: &str) -> Option<Url> {
        if s.len() > MAX_URL_LEN || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        let (https, rest) = match strip_prefix_ci(s, "https://") {
            Some(r) => (true, r),
            None => (false, strip_prefix_ci(s, "http://")?),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.is_empty() || authority.contains(['@', '[', ']', '\\']) {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p.parse::<u16>().ok().filter(|p| *p != 0)?)),
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        if host.is_empty()
            || host.starts_with(['.', '-'])
            || host.ends_with(['.', '-'])
            || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return None;
        }
        let path = if tail.starts_with('/') { tail.to_string() } else { format!("/{tail}") };
        Some(Url { https, host, port, path })
    }

    /// `https://host[:port]`.
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        match self.port {
            Some(p) => format!("{scheme}://{}:{p}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) { s.get(prefix.len()..) } else { None }
}

/// Parse `url` and check it against `policy`: scheme, host. The error names the URL (cut short).
pub fn check(policy: &Policy, url: &str) -> Result<Url, NetError> {
    let forbidden = || NetError::Forbidden(url.chars().take(120).collect());
    let u = Url::parse(url).ok_or_else(forbidden)?;
    if !u.https && !policy.allow_http {
        return Err(forbidden());
    }
    if !policy.hosts.iter().any(|h| h.eq_ignore_ascii_case(&u.host)) {
        return Err(forbidden());
    }
    Ok(u)
}

/// The absolute URL a redirect's `Location` points to: absolute `http(s)` URLs as they are,
/// `/path` against `base`'s origin. Scheme-relative (`//host`) and relative paths are refused.
pub fn resolve_location(base: &Url, location: &str) -> Option<String> {
    let loc = location.trim();
    if loc.starts_with("//") {
        return None;
    }
    if loc.starts_with('/') {
        return Some(format!("{}{loc}", base.origin()));
    }
    Url::parse(loc).map(|_| loc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy::github_api()
    }

    #[test]
    fn parses_what_the_toolbox_requests() {
        let u = Url::parse("https://api.github.com/repos/storytold/photocraft/releases?per_page=20").unwrap();
        assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "api.github.com", None, "/repos/storytold/photocraft/releases?per_page=20"));
        assert_eq!(Url::parse("HTTPS://API.GitHub.com").unwrap().host, "api.github.com");
        assert_eq!(Url::parse("http://127.0.0.1:8080/x").unwrap().origin(), "http://127.0.0.1:8080");
        assert_eq!(Url::parse("https://api.github.com?x").unwrap().path, "/?x");
    }

    #[test]
    fn the_policy_admits_only_its_hosts_over_https() {
        assert!(check(&policy(), "https://api.github.com/repos/o/r/releases").is_ok());
        for bad in [
            "http://api.github.com/repos",
            "https://api.github.com.evil.example/repos",
            "https://evil.example/https://api.github.com/",
            "https://api.github.com@evil.example/",
            "https://evil.example\\@api.github.com/",
            "https://[::1]/",
            "ftp://api.github.com/",
            "file:///etc/passwd",
            "api.github.com/repos",
            "https://api.github.com/a b",
            "https://api.github.com/\r\nX-Injected: 1",
            "https://api.github.com:0/",
            "https://api.github.com:99999/",
            "https://.api.github.com/",
            "https://",
            "",
        ] {
            assert!(matches!(check(&policy(), bad), Err(NetError::Forbidden(_))), "{bad:?}");
        }
        let long = format!("https://api.github.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(check(&policy(), &long).is_err());
    }

    #[test]
    fn locations() {
        let base = Url::parse("https://api.github.com/repos/o/r/releases").unwrap();
        assert_eq!(resolve_location(&base, "/repositories/1/releases").as_deref(), Some("https://api.github.com/repositories/1/releases"));
        assert_eq!(resolve_location(&base, "https://evil.example/x").as_deref(), Some("https://evil.example/x"), "resolved here, refused by check()");
        assert_eq!(resolve_location(&base, "//evil.example/x"), None);
        assert_eq!(resolve_location(&base, "relative/path"), None);
        assert_eq!(resolve_location(&base, "javascript:alert(1)"), None);
    }
}
