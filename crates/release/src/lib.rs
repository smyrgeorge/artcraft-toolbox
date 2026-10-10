//! The release contract every Crafting App follows, as pure data: versions, release asset names,
//! platform targets and `SHA256SUMS.txt`. See `docs/release-contract.md`.
//!
//! Standalone (L0): no workspace dependencies and no I/O. Everything here parses untrusted text
//! (GitHub API responses, file names, checksum files), so nothing panics on any input.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod asset;
pub mod checksums;
pub mod signing;
pub mod target;
pub mod version;

pub use asset::{AssetName, Component, PackageKind};
pub use checksums::{Checksums, Sha256};
pub use target::{Arch, Os, Target};
pub use version::Version;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("`{0}` is not a version like 1.2.3 or 1.2.3-rc.1")]
    BadVersion(String),
    #[error("checksum file, line {line}: {reason}")]
    BadChecksums { line: usize, reason: String },
    #[error("{what} is too large ({len} bytes, limit {limit})")]
    TooLarge { what: &'static str, len: usize, limit: usize },
    #[error("key: {0}")]
    BadKey(String),
    #[error("signed document: {0}")]
    BadEnvelope(String),
    #[error("signature: {0}")]
    BadSignature(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// At most `max` bytes of `s`, cut at a char boundary: error messages echo hostile input.
pub(crate) fn excerpt(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", s.get(..end).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::excerpt;

    #[test]
    fn excerpt_cuts_at_char_boundaries() {
        assert_eq!(excerpt("short", 10), "short");
        assert_eq!(excerpt("abcdef", 3), "abc…");
        // 'é' is two bytes: cutting at 2 would split it.
        assert_eq!(excerpt("aéb", 2), "a…");
    }
}
