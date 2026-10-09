//! Catalog, plural and language-tag tests (PhotoCraft's, adapted): every string the UI shows is
//! translated in every language, placeholders survive, plural forms are complete, and every
//! character outside Chinese, Japanese and Korean exists in the built-in fonts.

use super::catalog::{parse_entries, placeholders};
use super::*;

fn lang(code: &str) -> Lang {
    Lang::from_code(code).expect("registered language")
}

/// The UI's English sources: `tl!` literals, plural pairs, from the source files (test modules
/// aside), plus the engine messages and the catalog's taglines.
fn sources() -> (std::collections::BTreeSet<String>, std::collections::BTreeSet<(String, String)>) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut plain = std::collections::BTreeSet::new();
    let mut plurals = std::collections::BTreeSet::new();
    let lit = |s: &str| s.replace("\\\"", "\"");
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            // Test files aside (this one quotes the patterns it looks for).
            if path.extension().is_none_or(|e| e != "rs") || path.file_name().is_some_and(|n| n == "tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            let code = text.split("#[cfg(test)]\nmod ").next().unwrap_or("");
            // tl! with a string literal
            for tail in code.split("tl!(\"").skip(1) {
                let mut end = 0;
                let bytes = tail.as_bytes();
                while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
                    end += 1;
                }
                if tail.get(end + 1..end + 2) == Some(")") {
                    plain.insert(lit(&tail[..end]));
                }
            }
            // tn and trn calls: the first two string literals separated by a comma
            for marker in ["tn(", "trn("] {
                for tail in code.split(marker).skip(1) {
                    let call = tail.split(';').next().unwrap_or("");
                    let args: Vec<&str> = call.split('"').collect();
                    if let [_, one, comma, other, ..] = args.as_slice()
                        && comma.trim() == ","
                    {
                        plurals.insert((lit(one), lit(other)));
                    }
                }
            }
        }
    }
    plain.extend(crate::wording::ENGINE_STRINGS.iter().map(|s| s.to_string()));
    plain.extend(artcraft_toolbox_engine::Catalog::builtin().unwrap().apps.iter().map(|a| a.tagline.clone()));
    (plain, plurals)
}

#[test]
fn the_scan_finds_the_ui() {
    let (plain, plurals) = sources();
    assert!(plain.len() > 120, "only {} strings found", plain.len());
    assert!(plain.contains("Check for updates") && plain.contains("Word processing") && plain.contains("the server took too long to answer"));
    assert!(plurals.contains(&("{n} app".to_string(), "{n} apps".to_string())), "{plurals:?}");
}

/// A new label can't ship untranslated: every language has every string.
#[test]
fn every_string_is_translated_in_every_language() {
    let (plain, plurals) = sources();
    for l in Lang::all().filter(|l| *l != Lang::EN) {
        let missing: Vec<_> = plain.iter().filter(|s| !has(l, s)).collect();
        assert!(missing.is_empty(), "{}: untranslated: {missing:#?}", l.code());
        let missing: Vec<_> = plurals.iter().filter(|(one, other)| !l.catalog().has_plural(one, other)).collect();
        assert!(missing.is_empty(), "{}: untranslated plurals: {missing:#?}", l.code());
    }
}

/// Every bundled catalog parses cleanly, has no duplicates, keeps every placeholder and the
/// trailing ellipsis, and has as many plural forms as its language's rule.
#[test]
fn bundled_catalogs_are_consistent() {
    let (plain, plurals) = sources();
    for l in &LANGUAGES {
        assert!(l.code == l.code.to_ascii_lowercase() && !l.name.is_empty(), "{}", l.code);
        let (entries, errors) = parse_entries(l.source);
        assert!(errors.is_empty(), "{}: {errors:?}", l.code);
        let mut seen = std::collections::HashSet::new();
        let forms = (0..=1000).map(l.plural).max().unwrap_or(0) + 1;
        for (ctx, src, tr) in &entries {
            assert!(seen.insert((ctx.clone(), src.clone())), "{}: duplicate {ctx:?} {src:?}", l.code);
            if ctx == "@plural" {
                let (one, other) = src.split_once('|').unwrap_or_else(|| panic!("{}: plural source must be `one|other`: {src:?}", l.code));
                assert!(plurals.contains(&(one.to_string(), other.to_string())), "{}: unused plural {src:?}", l.code);
                assert_eq!(tr.split('|').count(), forms, "{}: {forms} plural forms expected in {src:?}", l.code);
                for form in tr.split('|') {
                    let (mut want, mut got) = (placeholders(other), placeholders(form));
                    want.sort_unstable();
                    got.sort_unstable();
                    assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
                }
                continue;
            }
            assert!(plain.contains(src), "{}: {src:?} is no longer shown anywhere: remove it", l.code);
            let (mut want, mut got) = (placeholders(src), placeholders(tr));
            want.sort_unstable();
            got.sort_unstable();
            assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
            assert_eq!(src.ends_with('…'), tr.ends_with('…'), "{}: ellipsis mismatch: {src:?}", l.code);
        }
    }
}

/// egui's built-in fonts draw every character of every catalog except Chinese, Japanese and
/// Korean (those come from the system's fonts, `cjk_fonts`): no missing-glyph boxes.
#[test]
fn translations_use_glyphs_the_built_in_fonts_have() {
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let font = egui::FontId::proportional(12.0);
    for l in Lang::all().filter(|l| *l != Lang::EN) {
        for (_, _, tr) in parse_entries(l.0.source).0 {
            let missing: String = tr
                .chars()
                .filter(|c| !c.is_whitespace() && crate::cjk::classify(*c).is_none())
                .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&font, *c)))
                .collect();
            assert!(missing.is_empty(), "{}: no glyph for {missing:?} in {tr:?}", l.code());
        }
    }
}

#[test]
fn tags_map_to_languages() {
    assert_eq!(lang_from_tag("ja_JP.UTF-8"), Some(lang("ja")));
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("cs-CZ"), Some(lang("cs")));
    assert_eq!(lang_from_tag("de-AT"), Some(lang("de")));
    assert_eq!(lang_from_tag("el_GR.UTF-8"), Some(lang("el")));
    for tag in ["zh_TW.UTF-8", "zh-HK", "zh_MO", "zh-Hant", "zh-Hant-HK"] {
        assert_eq!(lang_from_tag(tag), Some(lang("zh-hant")), "{tag}");
    }
    for tag in ["zh-CN", "zh_SG", "zh-Hans", "zh"] {
        assert_eq!(lang_from_tag(tag), Some(lang("zh-hans")), "{tag}");
    }
    for tag in ["pt", "pt-BR", "pt_PT.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(lang("pt-br")), "{tag}");
    }
    assert_eq!(lang_from_tag("sv-SE"), None);
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
    assert_eq!(candidates("zh_TW"), ["zh-tw", "zh-hant", "zh"]);
}

#[test]
fn preferences_resolve_with_fallback() {
    assert_eq!(Lang::from_pref("ja"), lang("ja"));
    assert_eq!(Lang::from_pref("JA"), lang("ja"));
    assert_eq!(Lang::from_pref("ZH-Hant"), lang("zh-hant"));
    // `auto` and unknown codes follow the system (English under test).
    assert_eq!(Lang::from_pref("auto"), Lang::EN);
    assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
}

#[test]
fn lookups_fall_back_to_english() {
    assert_eq!(tr(lang("ja"), "no such label"), "no such label");
    assert_eq!(tr(Lang::EN, "Check for updates"), "Check for updates");
    assert_eq!(tr(lang("de"), "Check for updates"), "Nach Updates suchen");
    assert_eq!(tr(lang("el"), "Uninstall"), "Απεγκατάσταση");
    assert_eq!(tr_ctx(lang("fr"), "no such context", "Cancel"), "Annuler");
    assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y")]), "y before x");
    assert_eq!(fmt("{missing}", &[]), "{missing}");
    assert_eq!(placeholders("a {x} b {y} {"), ["x", "y"]);
}

#[test]
fn plural_rules_and_messages() {
    let one_other = ("{n} app", "{n} apps");
    assert_eq!(trn(Lang::EN, 1, one_other.0, one_other.1), "1 app");
    assert_eq!(trn(Lang::EN, 12, one_other.0, one_other.1), "12 apps");
    for (n, want) in [(1, "1 приложение"), (3, "3 приложения"), (5, "5 приложений"), (11, "11 приложений"), (21, "21 приложение"), (22, "22 приложения")]
    {
        assert_eq!(trn(lang("ru"), n, one_other.0, one_other.1), want);
    }
    for (n, want) in [(1, "1 aplikacja"), (3, "3 aplikacje"), (5, "5 aplikacji"), (12, "12 aplikacji"), (22, "22 aplikacje")] {
        assert_eq!(trn(lang("pl"), n, one_other.0, one_other.1), want);
    }
    for (n, want) in [(1, "1 aplikace"), (3, "3 aplikace"), (5, "5 aplikací")] {
        assert_eq!(trn(lang("cs"), n, one_other.0, one_other.1), want);
    }
    assert_eq!(trn(lang("fr"), 0, "{n} day ago", "{n} days ago"), "il y a 0 jour");
    assert_eq!(trn(lang("fr"), 2, "{n} day ago", "{n} days ago"), "il y a 2 jours");
    assert_eq!(trn(lang("ja"), 7, one_other.0, one_other.1), "7 個のアプリ");
    let forms: Vec<usize> = [0, 1, 2, 4, 5, 12, 14, 21, 22, 25, 112, 122].into_iter().map(plural_polish).collect();
    assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2, 2, 1, 2, 2, 1]);
    let forms: Vec<usize> = [0, 1, 2, 3, 4, 5, 11, 21, u64::MAX].into_iter().map(plural_cs).collect();
    assert_eq!(forms, [2, 0, 1, 1, 1, 2, 2, 2, 2]);
}

#[test]
fn the_current_language_is_scoped_and_restored() {
    with_language(Lang::EN, || {
        with_language(lang("de"), || {
            assert_eq!(t("Cancel"), "Abbrechen");
            with_language(lang("ko"), || assert_eq!(t("Cancel"), "취소"));
            assert_eq!(t("Cancel"), "Abbrechen");
        });
        assert_eq!(t("Cancel"), "Cancel");
        let restored = std::panic::catch_unwind(|| with_language(lang("it"), || panic!("unwinding")));
        assert!(restored.is_err());
        assert_eq!(current(), Lang::EN, "restored after a panic");
        std::thread::spawn(|| assert_eq!(current(), Lang::EN, "per thread")).join().unwrap();
    });
}

#[test]
fn malformed_lines_are_reported_not_fatal() {
    let (entries, errors) = parse_entries("\tok\tはい\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n");
    assert_eq!(entries.len(), 1);
    assert_eq!(errors.len(), 4, "{errors:?}");
    assert_eq!(parse_entries("\ta\\tb\tx\\ny\\\\z\n").0[0], (String::new(), "a\tb".into(), "x\ny\\z".into()));
}
