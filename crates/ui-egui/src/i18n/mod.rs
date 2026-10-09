//! UI localisation, PhotoCraft's pattern (its `ui-egui/src/i18n`). Strings in code stay English
//! and are the lookup keys: `tl!("Check for updates")`. A per-language catalog (`*.tsv`, format
//! in `de.tsv`) maps them to display text at render time; a string without a translation shows
//! in English. Command ids, the CLI, logs and the engine's JSON never see translated text, so
//! scripts and agents are unaffected.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `de.tsv` (copy its header; translate from the *meaning* of the English
//!    text, clean-room).
//! 2. Add one row to [`LANGUAGES`] (code, native name, catalog, plural rule).
//!
//! The Settings dropdown, the system-language match and the tests (every `tl!` string translated,
//! placeholders kept, plural forms complete) pick it up from the registry.
//!
//! # Looking strings up
//! - The `tl!` macro (a string literal) and [`t`]: a plain string in the current language. [`tr_ctx`]: when one English word
//!   needs different translations.
//! - [`trn`]: plural-aware (`{n}` is filled in). [`fmt`]: fill `{name}` placeholders after a
//!   lookup; translators may reorder them freely.

mod catalog;
mod system;

pub use system::{system_lang, system_tags};

use std::cell::Cell;
use std::sync::OnceLock;

use catalog::Catalog;

/// One supported UI language.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hans`, `pt-br`); also the `language` setting's value.
    pub code: &'static str,
    /// The language's name in itself, shown in the Settings dropdown.
    pub name: &'static str,
    /// Catalog file contents (empty for the built-in English).
    pub source: &'static str,
    /// Plural form index for a count (English: 0 = one, 1 = other; Japanese and Chinese: always 0;
    /// Czech: 0 = one, 1 = few (2–4), 2 = other; French: 0 = one (0 and 1), 1 = other). A catalog's
    /// `@plural` entries list one form per index.
    pub plural: fn(u64) -> usize,
    catalog: OnceLock<Catalog>,
}

fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

fn plural_none(_: u64) -> usize {
    0
}

/// Russian: 1, 21, 31… → one; 2–4, 22–24… → few; everything else (11–19 included) → many.
fn plural_russian(n: u64) -> usize {
    match (n % 10, n % 100) {
        (1, 11..=19) => 2,
        (1, _) => 0,
        (2..=4, 11..=19) => 2,
        (2..=4, _) => 1,
        _ => 2,
    }
}

/// Czech: 1 → one, 2–4 → few, everything else (0, 5+) → other.
fn plural_cs(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

/// French and Brazilian Portuguese: 0 and 1 take the singular, everything else the plural.
fn plural_zero_one(n: u64) -> usize {
    usize::from(n > 1)
}

/// Polish: 1 → one; 2–4, except 12–14 → few; everything else → many.
fn plural_polish(n: u64) -> usize {
    let (last, last_two) = (n % 10, n % 100);
    if n == 1 {
        0
    } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
        1
    } else {
        2
    }
}

/// The registry. English first: it is the fallback and the source language. The same languages
/// as PhotoCraft.
pub static LANGUAGES: [LangInfo; 15] = [
    LangInfo { code: "en", name: "English", source: "", plural: plural_one_other, catalog: OnceLock::new() },
    LangInfo { code: "ja", name: "日本語", source: include_str!("ja.tsv"), plural: plural_none, catalog: OnceLock::new() },
    LangInfo { code: "zh-hans", name: "简体中文", source: include_str!("zh-hans.tsv"), plural: plural_none, catalog: OnceLock::new() },
    // Traditional Chinese as used in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` resolve here.
    LangInfo { code: "zh-hant", name: "繁體中文", source: include_str!("zh-hant.tsv"), plural: plural_none, catalog: OnceLock::new() },
    LangInfo { code: "es", name: "Español", source: include_str!("es.tsv"), plural: plural_one_other, catalog: OnceLock::new() },
    LangInfo { code: "ru", name: "Русский", source: include_str!("ru.tsv"), plural: plural_russian, catalog: OnceLock::new() },
    LangInfo { code: "cs", name: "Čeština", source: include_str!("cs.tsv"), plural: plural_cs, catalog: OnceLock::new() },
    LangInfo { code: "fr", name: "Français", source: include_str!("fr.tsv"), plural: plural_zero_one, catalog: OnceLock::new() },
    LangInfo { code: "id", name: "Bahasa Indonesia", source: include_str!("id.tsv"), plural: plural_none, catalog: OnceLock::new() },
    LangInfo { code: "ko", name: "한국어", source: include_str!("ko.tsv"), plural: plural_none, catalog: OnceLock::new() },
    LangInfo { code: "pl", name: "Polski", source: include_str!("pl.tsv"), plural: plural_polish, catalog: OnceLock::new() },
    LangInfo { code: "de", name: "Deutsch", source: include_str!("de.tsv"), plural: plural_one_other, catalog: OnceLock::new() },
    // Brazilian Portuguese; `pt`, `pt-BR` and `pt-PT` resolve here.
    LangInfo { code: "pt-br", name: "Português (Brasil)", source: include_str!("pt-br.tsv"), plural: plural_zero_one, catalog: OnceLock::new() },
    LangInfo { code: "el", name: "Ελληνικά", source: include_str!("el.tsv"), plural: plural_one_other, catalog: OnceLock::new() },
    LangInfo { code: "it", name: "Italiano", source: include_str!("it.tsv"), plural: plural_one_other, catalog: OnceLock::new() },
];

impl LangInfo {
    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| Catalog::parse(self.source))
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// A language by its exact code (any case).
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// The `language` setting: a code, or `auto` (and anything unknown, a newer version's code
    /// for instance) for the system's language.
    pub fn from_pref(pref: &str) -> Lang {
        if pref.eq_ignore_ascii_case("auto") {
            return system_lang();
        }
        Lang::from_code(pref).or_else(|| lang_from_tag(pref)).unwrap_or_else(system_lang)
    }

    /// Every registered language, English first.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

/// Candidate language codes for a locale tag, most specific first: `zh_TW.UTF-8` → `zh-tw`,
/// `zh-hant`, `zh`.
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else { return Vec::new() };
    let mut out = Vec::new();
    for n in (1..=parts.len()).rev() {
        out.push(parts.get(..n).unwrap_or_default().join("-"));
    }
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        // Chinese by region when no script is given.
        let script = if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-hant" } else { "zh-hans" };
        out.insert(out.len() - 1, script.to_string());
    }
    if primary == "pt" && !out.iter().any(|c| c == "pt-br") {
        // The only Portuguese catalog is Brazilian; other regions use it rather than English.
        out.insert(out.len() - 1, "pt-br".to_string());
    }
    out
}

/// The registered language for a locale tag such as `ja_JP.UTF-8`, `ja-JP`, `zh-TW`; `None` if
/// it isn't supported. `C` and `POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let cands = candidates(tag);
    if matches!(cands.first().map(String::as_str), Some("c" | "posix")) {
        return Some(Lang::EN);
    }
    cands.iter().find_map(|c| Lang::from_code(c))
}

thread_local! {
    // Independent app and test threads must not change each other's drawing language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// Set the language for drawing (the shell does it each frame from the setting), so widgets can
/// translate without every call site carrying a language around.
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    CURRENT.get()
}

/// Draw in another language for the duration of `draw`, restoring the previous one even when it
/// unwinds.
pub fn with_language<R>(lang: Lang, draw: impl FnOnce() -> R) -> R {
    struct Restore(Lang);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_current(self.0);
        }
    }
    let _restore = Restore(current());
    set_current(lang);
    draw()
}

/// Apply the `language` setting to this context: the current language, and on a change the
/// fallback fonts for Japanese, Chinese and Korean (which script's glyph forms win follows the
/// language).
pub fn sync_context(ctx: &egui::Context, language: &str) {
    let lang = Lang::from_pref(language);
    set_current(lang);
    let id = egui::Id::new("artcraft-toolbox-ui-language");
    if ctx.data(|d| d.get_temp::<Lang>(id)) != Some(lang) {
        ctx.data_mut(|d| d.insert_temp(id, lang));
        crate::cjk_fonts::install(ctx);
        ctx.request_repaint();
    }
}

/// Does `lang` have a catalog entry for this plain string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    lang.catalog().contextual(context, s).unwrap_or_else(|| tr(lang, s))
}

/// Fill `{name}` placeholders. Unknown placeholders are left as written.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// A plural-aware message in `lang`: `one`/`other` are the English forms (with `{n}` where the
/// count goes).
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    let idx = (lang.0.plural)(n);
    let text = lang.catalog().plural(one, other, idx).unwrap_or(if n == 1 { one } else { other });
    fmt(text, &[("n", &n.to_string())])
}

/// [`trn`] in the current language.
pub fn tn(n: u64, one: &str, other: &str) -> String {
    trn(current(), n, one, other)
}

#[cfg(test)]
mod tests;
