//! Release versions: `MAJOR.MINOR.PATCH` with an optional pre-release (`1.2.3-rc.1`), the form
//! every craft's `cargo xtask version` writes, ordered by semver precedence.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use crate::{Error, Result, excerpt};

/// Longest version string accepted. Real ones are a dozen characters.
pub const MAX_LEN: usize = 64;

/// A parsed version. Build metadata (`+…`) is accepted and dropped: it never affects ordering.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Pre-release identifiers (`rc.1` is `[Alpha("rc"), Num(1)]`); empty for a release.
    pub pre: Vec<Pre>,
}

/// One dot-separated pre-release identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pre {
    Num(u64),
    Alpha(String),
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version { major, minor, patch, pre: Vec::new() }
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Strict parse: `1.2.3`, `1.2.3-rc.1`, `1.2.3+build`. No `v` prefix, no leading zeros.
    pub fn parse(s: &str) -> Result<Version> {
        let bad = || Error::BadVersion(excerpt(s, MAX_LEN));
        if s.is_empty() || s.len() > MAX_LEN {
            return Err(bad());
        }
        let (rest, build) = match s.split_once('+') {
            Some((r, b)) => (r, Some(b)),
            None => (s, None),
        };
        if let Some(b) = build
            && !b.split('.').all(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
        {
            return Err(bad());
        }
        let (core, pre) = match rest.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (rest, None),
        };
        let mut nums = core.split('.').map(numeric);
        let (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) = (nums.next(), nums.next(), nums.next(), nums.next()) else {
            return Err(bad());
        };
        let mut ids = Vec::new();
        if let Some(p) = pre {
            for id in p.split('.') {
                if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
                    return Err(bad());
                }
                if id.bytes().all(|c| c.is_ascii_digit()) {
                    ids.push(Pre::Num(numeric(id).ok_or_else(bad)?));
                } else {
                    ids.push(Pre::Alpha(id.to_string()));
                }
            }
        }
        Ok(Version { major, minor, patch, pre: ids })
    }

    /// A release tag: `v1.2.3`, `1.2.3`, or `<name>-v1.2.3` (ArtCraft's own `artcraft-v0.41.0`).
    pub fn from_tag(tag: &str) -> Result<Version> {
        let t = tag.trim();
        if let Ok(v) = Version::parse(t) {
            return Ok(v);
        }
        if let Some(rest) = t.strip_prefix('v').or_else(|| t.strip_prefix('V'))
            && let Ok(v) = Version::parse(rest)
        {
            return Ok(v);
        }
        if let Some(i) = t.rfind("-v")
            && let Some(rest) = t.get(i + 2..)
            && let Ok(v) = Version::parse(rest)
        {
            return Ok(v);
        }
        Err(Error::BadVersion(excerpt(t, MAX_LEN)))
    }
}

/// A semver numeric identifier: digits only, no leading zero unless it is `0`, fits in u64.
fn numeric(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // A pre-release sorts before its release: 1.0.0-rc.1 < 1.0.0.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&other.pre),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pre {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Pre::Num(a), Pre::Num(b)) => a.cmp(b),
            (Pre::Alpha(a), Pre::Alpha(b)) => a.cmp(b),
            (Pre::Num(_), Pre::Alpha(_)) => Ordering::Less,
            (Pre::Alpha(_), Pre::Num(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Pre {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match id {
                Pre::Num(n) => write!(f, "{n}")?,
                Pre::Alpha(s) => f.write_str(s)?,
            }
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = Error;
    fn from_str(s: &str) -> Result<Version> {
        Version::parse(s)
    }
}

impl serde::Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Version, D::Error> {
        let s = String::deserialize(d)?;
        Version::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_and_displays() {
        for s in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2", "0.0.0"] {
            assert_eq!(v(s).to_string(), s);
        }
        assert_eq!(v("1.2.3+build.5").to_string(), "1.2.3");
        assert_eq!(v("1.0.0-rc.1").pre, vec![Pre::Alpha("rc".into()), Pre::Num(1)]);
    }

    #[test]
    fn rejects_malformed_versions() {
        let long = "1".repeat(MAX_LEN + 1);
        for bad in ["", "1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0-01", "1.a.0", "1.0.0+", "1.0.0+a..b", "-1.0.0", " 1.0.0"] {
            assert!(Version::parse(bad).is_err(), "{bad}");
        }
        assert!(Version::parse(&long).is_err());
        // u64 overflow is an error, not a panic.
        assert!(Version::parse("99999999999999999999.0.0").is_err());
        assert!(Version::parse("1.0.0-99999999999999999999").is_err());
    }

    #[test]
    fn semver_precedence() {
        let order = [
            "0.9.9",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
        ];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(v("1.0.0+a").cmp(&v("1.0.0+b")), Ordering::Equal);
    }

    #[test]
    fn tags() {
        assert_eq!(Version::from_tag("v0.5.0").unwrap(), v("0.5.0"));
        assert_eq!(Version::from_tag("0.5.0").unwrap(), v("0.5.0"));
        assert_eq!(Version::from_tag("v1.0.0-rc.2").unwrap(), v("1.0.0-rc.2"));
        assert_eq!(Version::from_tag("artcraft-v0.41.0").unwrap(), v("0.41.0"));
        for bad in ["", "v", "latest", "nightly-2026-10-09", "vv1.0.0"] {
            assert!(Version::from_tag(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn error_echo_is_bounded() {
        let hostile = "x".repeat(10_000);
        let msg = Version::from_tag(&hostile).unwrap_err().to_string();
        assert!(msg.len() < 200, "{}", msg.len());
    }

    #[test]
    fn serde_round_trip() {
        let json = serde_json::to_string(&v("1.2.3-rc.1")).unwrap();
        assert_eq!(json, "\"1.2.3-rc.1\"");
        assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), v("1.2.3-rc.1"));
        assert!(serde_json::from_str::<Version>("\"nope\"").is_err());
    }
}
