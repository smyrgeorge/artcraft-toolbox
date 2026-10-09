//! `SHA256SUMS.txt`, published with every release: one `<hex digest>  <file name>` line per asset
//! (GNU `sha256sum` format; a `*` before the name marks binary mode and is accepted too).
//!
//! A checksum from the same release proves the download is intact, not who published it: see
//! `SECURITY.md` and `docs/release-contract.md` › Integrity.

use std::fmt;

use crate::{Error, Result, excerpt};

/// Largest checksum file accepted. A real one is about 2 KB.
pub const MAX_BYTES: usize = 1 << 20;

/// A SHA-256 digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    /// From 64 hex digits (either case).
    pub fn from_hex(s: &str) -> Option<Sha256> {
        let b = s.as_bytes();
        if b.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        let (pairs, _) = b.as_chunks::<2>();
        for (o, [hi, lo]) in out.iter_mut().zip(pairs) {
            *o = (hex_val(*hi)? << 4) | hex_val(*lo)?;
        }
        Some(Sha256(out))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({})", self.to_hex())
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A parsed checksum file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checksums {
    entries: Vec<(String, Sha256)>,
}

impl Checksums {
    /// Parse a checksum file. Blank lines and `#` comments are skipped. Any other malformed line,
    /// a path instead of a bare file name, or two different digests for one name is an error:
    /// a checksum file we only half understand must not vouch for anything.
    pub fn parse(text: &str) -> Result<Checksums> {
        if text.len() > MAX_BYTES {
            return Err(Error::TooLarge { what: "checksum file", len: text.len(), limit: MAX_BYTES });
        }
        let mut entries: Vec<(String, Sha256)> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |reason: String| Error::BadChecksums { line: i + 1, reason };
            let (hex, rest) = line.split_once(' ').ok_or_else(|| err(format!("expected `<sha256>  <file>`, got `{}`", excerpt(line, 80))))?;
            let digest = Sha256::from_hex(hex).ok_or_else(|| err(format!("`{}` is not a SHA-256 digest", excerpt(hex, 80))))?;
            let name = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*')).unwrap_or(rest);
            if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                return Err(err(format!("`{}` is not a bare file name", excerpt(name, 80))));
            }
            match entries.iter().find(|(n, _)| n == name) {
                Some((_, d)) if *d != digest => return Err(err(format!("two different digests for `{}`", excerpt(name, 80)))),
                Some(_) => {}
                None => entries.push((name.to_string(), digest)),
            }
        }
        Ok(Checksums { entries })
    }

    /// The digest listed for `file`.
    pub fn get(&self, file: &str) -> Option<Sha256> {
        self.entries.iter().find(|(n, _)| n == file).map(|(_, d)| *d)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Sha256)> {
        self.entries.iter().map(|(n, d)| (n.as_str(), *d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const B: &str = "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824";

    #[test]
    fn hex_round_trip() {
        let d = Sha256::from_hex(A).unwrap();
        assert_eq!(d.to_hex(), A);
        assert_eq!(Sha256::from_hex(B).unwrap().to_hex(), B.to_lowercase());
        for bad in ["", "abc", &A[..63], &format!("{A}0"), &A.replace('e', "g"), &"é".repeat(32)] {
            assert!(Sha256::from_hex(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn parses_gnu_and_binary_mode_lines() {
        let text = format!("{A}  photocraft-0.5.0-macos-universal.dmg\r\n\n# comment\n{B} *photocraft-0.5.0-windows-x64.msi\n");
        let c = Checksums::parse(&text).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c.get("photocraft-0.5.0-macos-universal.dmg").map(|d| d.to_hex()).as_deref(), Some(A));
        assert!(c.get("photocraft-0.5.0-windows-x64.msi").is_some());
        assert!(c.get("missing").is_none());
        assert!(Checksums::parse("").unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_files() {
        for bad in [
            A.to_string(),
            format!("{A}  "),
            "nothex  file.dmg".to_string(),
            format!("{A}  ../evil.dmg"),
            format!("{A}  dir/file.dmg"),
            format!("{A}  dir\\file.dmg"),
            format!("{A}  file.dmg\n{B}  file.dmg"),
        ] {
            let e = Checksums::parse(&bad).unwrap_err();
            assert!(matches!(e, Error::BadChecksums { .. }), "{bad}: {e}");
        }
        // The same digest twice is harmless.
        assert_eq!(Checksums::parse(&format!("{A}  f\n{A}  f\n")).unwrap().len(), 1);
        let huge = "x".repeat(MAX_BYTES + 1);
        assert!(matches!(Checksums::parse(&huge), Err(Error::TooLarge { .. })));
    }
}
