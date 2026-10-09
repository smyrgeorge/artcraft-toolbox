//! The system's preferred UI languages, queried once without subprocesses or registry parsing
//! (PhotoCraft's `i18n/system.rs`). `ARTCRAFT_TOOLBOX_LOCALE` overrides them.

use super::{Lang, lang_from_tag};

const MAX_SYSTEM_TAGS: usize = 64;
const MAX_SYSTEM_TAG_BYTES: usize = 128;

/// The language `auto` means: the first of the system's preferred languages the toolbox has, else
/// English. The OS's tags are cached, not a language, so matching stays separate from detection.
pub fn system_lang() -> Lang {
    #[cfg(test)]
    {
        TEST_SYSTEM_TAGS.with(|tags| resolve(&tags.borrow()))
    }
    #[cfg(not(test))]
    {
        resolve(system_tags())
    }
}

/// The system's preferred language tags, most preferred first (cached).
#[cfg(not(test))]
pub fn system_tags() -> &'static [String] {
    static SYSTEM: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    SYSTEM.get_or_init(detect_system_tags)
}

/// The system's preferred language tags (tests: none, so `auto` is English).
#[cfg(test)]
pub fn system_tags() -> &'static [String] {
    &[]
}

fn resolve(tags: &[String]) -> Lang {
    tags.iter().find_map(|tag| lang_from_tag(tag)).unwrap_or(Lang::EN)
}

fn bounded_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    tags.into_iter()
        .take(MAX_SYSTEM_TAGS)
        .filter(|tag| {
            !tag.trim().is_empty()
                && tag.len() <= MAX_SYSTEM_TAG_BYTES
                && tag.trim().bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'@'))
        })
        .map(|tag| tag.trim().to_owned())
        .collect()
}

#[cfg_attr(test, allow(dead_code))]
fn detect_system_tags() -> Vec<String> {
    if let Ok(tag) = std::env::var("ARTCRAFT_TOOLBOX_LOCALE")
        && !tag.trim().is_empty()
    {
        return bounded_tags([tag]);
    }
    // sys-locale wraps GetUserPreferredUILanguages (Windows) and CFLocaleCopyPreferredLanguages
    // (macOS) safely; Unix reads the standard locale variables. A failure in platform interop
    // leaves the toolbox in English.
    std::panic::catch_unwind(|| bounded_tags(sys_locale::get_locales())).unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    // Tests don't depend on the developer's locale, and don't change process-wide state.
    static TEST_SYSTEM_TAGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn with_system_tags<R>(tags: &[&str], run: impl FnOnce() -> R) -> R {
    struct Restore(Vec<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_SYSTEM_TAGS.set(std::mem::take(&mut self.0));
        }
    }
    let _restore = Restore(TEST_SYSTEM_TAGS.replace(tags.iter().map(|tag| (*tag).to_owned()).collect()));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_preferences_are_negotiated_in_order() {
        for (tags, expected) in [
            (vec!["ja-JP", "en-US"], "ja"),
            (vec!["fr-CA", "en-US"], "fr"),
            (vec!["sv-SE", "ko-KR", "fr-FR"], "ko"),
            (vec!["zh-Hant-HK", "zh-CN"], "zh-hant"),
            (vec!["zh-Hans-CN", "zh-TW"], "zh-hans"),
            (vec!["en-US", "ru-RU"], "en"),
            (vec!["sv-SE", "ar-SA"], "en"),
            (vec!["de-AT", "en-US"], "de"),
            (vec!["el-GR"], "el"),
            (vec!["pt-PT"], "pt-br"),
            (vec!["it-IT", "en-US"], "it"),
            (vec![], "en"),
        ] {
            with_system_tags(&tags, || assert_eq!(system_lang().code(), expected));
        }
    }

    #[test]
    fn native_locale_results_are_bounded_and_trimmed() {
        assert_eq!(bounded_tags(["  fr-CA  ".into(), "".into(), " ".into(), "fr-\0".into(), "x".repeat(129)]), ["fr-CA"]);
        assert_eq!(bounded_tags(std::iter::repeat_n("en-US".into(), 100)).len(), MAX_SYSTEM_TAGS);
        let detected = detect_system_tags();
        assert!(detected.len() <= MAX_SYSTEM_TAGS);
        assert!(detected.iter().all(|tag| !tag.is_empty() && tag.len() <= MAX_SYSTEM_TAG_BYTES));
    }
}
