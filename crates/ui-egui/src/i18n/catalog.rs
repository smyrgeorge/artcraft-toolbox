//! Parser and lookup tables for translation catalogs (`*.tsv`, format documented in `de.tsv`).
//! Ported from PhotoCraft's `i18n/catalog.rs`, without the command-id entries the toolbox doesn't
//! have.

use std::collections::HashMap;

/// A parsed catalog. Values are owned for the life of the process (catalogs are built once).
#[derive(Debug, Default)]
pub struct Catalog {
    /// Context-free strings: English source → translation (the hot path, looked up every frame).
    plain: HashMap<String, String>,
    /// Strings with a disambiguating context, keyed `context \u{1} source`.
    contextual: HashMap<String, String>,
    /// Plural messages: `one|other` → forms.
    plurals: HashMap<String, Vec<String>>,
}

/// A catalog entry as read from the file: (context, source, translation).
pub type Entry = (String, String, String);

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Read the entries of a catalog file. Malformed lines are returned as errors (and skipped), so
/// a bad translation never breaks the UI; the tests insist the bundled catalogs have none.
pub fn parse_entries(text: &str) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        match (cols.next(), cols.next(), cols.next(), cols.next()) {
            (Some(ctx), Some(src), Some(tr), None) if !src.is_empty() && !tr.is_empty() => {
                entries.push((unescape(ctx), unescape(src), unescape(tr)));
            }
            _ => errors.push(format!("line {}: expected `context<TAB>source<TAB>translation`", n + 1)),
        }
    }
    (entries, errors)
}

impl Catalog {
    pub fn parse(text: &str) -> Catalog {
        let mut c = Catalog::default();
        for (ctx, src, tr) in parse_entries(text).0 {
            match ctx.as_str() {
                "" => {
                    c.plain.insert(src, tr);
                }
                "@plural" => {
                    c.plurals.insert(src, tr.split('|').map(str::to_string).collect());
                }
                _ => {
                    c.contextual.insert(format!("{ctx}\u{1}{src}"), tr);
                }
            }
        }
        c
    }

    pub fn plain(&self, s: &str) -> Option<&str> {
        self.plain.get(s).map(String::as_str)
    }

    pub fn contextual(&self, ctx: &str, s: &str) -> Option<&str> {
        self.contextual.get(&format!("{ctx}\u{1}{s}")).map(String::as_str)
    }

    /// The plural form `index` of the message whose English forms are `one|other`.
    pub fn plural(&self, one: &str, other: &str, index: usize) -> Option<&str> {
        let forms = self.plurals.get(&format!("{one}|{other}"))?;
        forms.get(index.min(forms.len().saturating_sub(1))).map(String::as_str)
    }

    /// Is there a plural entry for `one|other`?
    #[cfg(test)]
    pub fn has_plural(&self, one: &str, other: &str) -> bool {
        self.plurals.contains_key(&format!("{one}|{other}"))
    }
}

/// `{name}` placeholders of a template, in order of appearance.
#[cfg(test)]
pub fn placeholders(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        let Some(after) = rest.get(a + 1..) else { break };
        match after.find('}') {
            Some(b) => {
                out.push(after.get(..b).unwrap_or(""));
                rest = after.get(b + 1..).unwrap_or("");
            }
            None => break,
        }
    }
    out
}
