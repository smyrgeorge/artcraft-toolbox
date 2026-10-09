//! Which installed system fonts cover Japanese, Simplified Chinese, Traditional Chinese and
//! Korean, and in which order to try them (PhotoCraft's `text/src/cjk.rs`, ported). egui's
//! built-in fonts have none of these glyphs, and the toolbox doesn't bundle a multi-megabyte CJK
//! font: the system's are used, loaded only when such text is drawn (`cjk_fonts`).
//!
//! Han characters are shared between the four, but their preferred glyph forms differ, so the font
//! for the UI language's script goes first: Japanese forms only for Japanese, Chinese forms for
//! zh-Hans / zh-Hant, Korean fonts for Korean. Kana always prefers a Japanese font, Hangul a Korean
//! one and Bopomofo a Traditional Chinese one.

use std::path::PathBuf;

/// A CJK writing system with its own preferred fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkScript {
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
    Korean,
}

/// The kind of CJK character that needs a fallback font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkChar {
    /// Ideographs, CJK punctuation and full-width forms: the font follows the language.
    Han,
    Kana,
    Hangul,
    Bopomofo,
}

impl CjkChar {
    /// The script whose fonts are tried first for this character.
    pub fn preferred(self, order: &[CjkScript; 4]) -> CjkScript {
        match self {
            CjkChar::Kana => CjkScript::Japanese,
            CjkChar::Hangul => CjkScript::Korean,
            CjkChar::Bopomofo => CjkScript::TraditionalChinese,
            CjkChar::Han => order[0],
        }
    }
}

/// Classifies a character that needs a CJK font, or `None` for everything else.
pub fn classify(c: char) -> Option<CjkChar> {
    Some(match c as u32 {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF | 0xFFA0..=0xFFDC => CjkChar::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F | 0x1B000..=0x1B16F => CjkChar::Kana,
        0x3100..=0x312F | 0x31A0..=0x31BF => CjkChar::Bopomofo,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3190..=0x319F | 0x31C0..=0x31EF | 0x3200..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF => CjkChar::Han,
        0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF64 | 0xFFE0..=0xFFEF | 0x20000..=0x3FFFF => CjkChar::Han,
        _ => return None,
    })
}

/// Script order for a language or locale tag (`ja`, `zh-hant`, `zh_CN`, `ko-KR`). Others (or
/// none) get Simplified Chinese first, so Japanese forms only win shared Han for Japanese.
pub fn script_order(locale: Option<&str>) -> [CjkScript; 4] {
    use CjkScript::*;
    let tag = locale.unwrap_or("").split(['.', '@']).next().unwrap_or("").to_ascii_lowercase().replace('_', "-");
    let mut parts = tag.split('-');
    let lang = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    match lang {
        "ja" => [Japanese, SimplifiedChinese, TraditionalChinese, Korean],
        "ko" => [Korean, TraditionalChinese, SimplifiedChinese, Japanese],
        "zh" | "yue" => {
            let hant = rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
            let hans = rest.iter().any(|p| matches!(*p, "hans" | "cn" | "sg"));
            if hant && !hans || lang == "yue" && !hans {
                [TraditionalChinese, SimplifiedChinese, Japanese, Korean]
            } else {
                [SimplifiedChinese, TraditionalChinese, Japanese, Korean]
            }
        }
        _ => [SimplifiedChinese, TraditionalChinese, Japanese, Korean],
    }
}

/// An installed font file to try: its path and the family of the wanted face in a collection
/// (empty: face 0). Matching by family keeps working when a `.ttc` reorders its faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: &'static str,
}

/// System font files for `script`, most preferred first. Paths that don't exist are included;
/// callers skip them.
pub fn font_files(script: CjkScript) -> Vec<FontFile> {
    use CjkScript::*;
    let f = |p: &str, family: &'static str| FontFile { path: p.into(), family };
    if cfg!(target_os = "macos") {
        let pingfang = |family| mac_pingfang().map(|path| FontFile { path, family });
        match script {
            Japanese => vec![f("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", ""), f("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc", "")],
            SimplifiedChinese => pingfang("PingFang SC")
                .into_iter()
                .chain([f("/System/Library/Fonts/Hiragino Sans GB.ttc", ""), f("/System/Library/Fonts/STHeiti Light.ttc", "Heiti SC")])
                .collect(),
            TraditionalChinese => pingfang("PingFang TC").into_iter().chain([f("/System/Library/Fonts/STHeiti Light.ttc", "Heiti TC")]).collect(),
            Korean => vec![f("/System/Library/Fonts/AppleSDGothicNeo.ttc", "Apple SD Gothic Neo")],
        }
    } else if cfg!(windows) {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let w = |file: &str, family: &'static str| FontFile { path: std::path::Path::new(&dir).join("Fonts").join(file), family };
        match script {
            Japanese => vec![w("YuGothR.ttc", "Yu Gothic"), w("YuGothM.ttc", "Yu Gothic"), w("meiryo.ttc", "Meiryo"), w("msgothic.ttc", "")],
            SimplifiedChinese => vec![w("msyh.ttc", "Microsoft YaHei"), w("msyh.ttf", ""), w("simsun.ttc", "")],
            TraditionalChinese => vec![w("msjh.ttc", "Microsoft JhengHei"), w("msjh.ttf", ""), w("mingliu.ttc", "")],
            Korean => vec![w("malgun.ttf", ""), w("gulim.ttc", "")],
        }
    } else {
        // One Noto Sans CJK collection covers all four scripts; the face picks the glyph forms.
        let noto = match script {
            Japanese => "Noto Sans CJK JP",
            SimplifiedChinese => "Noto Sans CJK SC",
            TraditionalChinese => "Noto Sans CJK TC",
            Korean => "Noto Sans CJK KR",
        };
        let mut v: Vec<FontFile> = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/local/share/fonts/noto/NotoSansCJK-Regular.ttc",
            "/usr/local/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/local/share/fonts/noto/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-VF.ttc",
        ]
        .iter()
        .map(|p| f(p, noto))
        .collect();
        v.extend(match script {
            Japanese => vec![
                f("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf", ""),
                f("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", ""),
                f("/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf", ""),
            ],
            SimplifiedChinese | TraditionalChinese => vec![
                f("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", ""),
                f("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc", ""),
                f("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", ""),
                f("/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", ""),
            ],
            Korean => vec![
                f("/usr/share/fonts/truetype/nanum/NanumGothic.ttf", ""),
                f("/usr/share/fonts/nanum/NanumGothic.ttf", ""),
                f("/usr/share/fonts/truetype/unfonts-core/UnDotum.ttf", ""),
                f("/usr/share/fonts/un-core/UnDotum.ttf", ""),
            ],
        });
        with_flatpak_host_fonts(v)
    }
}

/// Flatpak mounts the host's font folder outside the runtime's `/usr/share/fonts`: try that
/// mirror right after each path, with the same collection face.
fn with_flatpak_host_fonts(files: Vec<FontFile>) -> Vec<FontFile> {
    let mut candidates = Vec::new();
    for file in files {
        let host = if cfg!(target_os = "linux") {
            file.path
                .strip_prefix("/usr/share/fonts")
                .ok()
                .map(|relative| FontFile { path: std::path::Path::new("/run/host/fonts").join(relative), family: file.family })
        } else {
            None
        };
        candidates.push(file);
        candidates.extend(host);
    }
    candidates
}

/// Broad-coverage fonts tried after every script's own (they cover Han, kana and Hangul but look
/// worse than the script fonts).
pub fn last_resort_files() -> Vec<FontFile> {
    let f = |p: &str| FontFile { path: p.into(), family: "" };
    if cfg!(target_os = "macos") {
        vec![f("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"), f("/Library/Fonts/Arial Unicode.ttf")]
    } else if cfg!(windows) {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        vec![FontFile { path: std::path::Path::new(&dir).join("Fonts").join("ARIALUNI.TTF"), family: "" }]
    } else {
        with_flatpak_host_fonts(vec![
            f("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
            f("/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf"),
        ])
    }
}

/// macOS ships PingFang as a pre-installed "mobile asset" outside `/System/Library/Fonts`
/// (`/System/Library/AssetsV2/com_apple_MobileAsset_Font*/<hash>.asset/AssetData/PingFang.ttc`).
/// Looks there (a few small folder listings) and in the pre-Catalina location.
pub fn mac_pingfang() -> Option<PathBuf> {
    use std::path::Path;
    let old = PathBuf::from("/System/Library/Fonts/PingFang.ttc");
    if old.is_file() {
        return Some(old);
    }
    let roots = [Path::new("/System/Library/AssetsV2"), Path::new("/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs")];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for e in rd.flatten().take(512) {
            if !e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                continue;
            }
            let Ok(assets) = std::fs::read_dir(e.path()) else { continue };
            for a in assets.flatten().take(4096) {
                let p = a.path().join("AssetData/PingFang.ttc");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Face index in a font file (or collection) whose family name is `family`; 0 when `family` is
/// empty or no face matches. Never panics on malformed data.
pub fn face_index_for_family(bytes: &[u8], family: &str) -> u32 {
    use skrifa::{MetadataProvider, raw::FileRef, string::StringId};
    if family.is_empty() {
        return 0;
    }
    let Ok(FileRef::Collection(c)) = FileRef::new(bytes) else { return 0 };
    for (i, f) in c.iter().enumerate() {
        let Ok(f) = f else { continue };
        let named = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME].iter().any(|id| f.localized_strings(*id).any(|s| s.chars().eq(family.chars())));
        if named {
            return u32::try_from(i).unwrap_or(0);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use CjkScript::*;

    #[test]
    fn locale_orders() {
        assert_eq!(script_order(Some("ja_JP.UTF-8"))[0], Japanese);
        assert_eq!(script_order(Some("ja"))[0], Japanese);
        assert_eq!(script_order(Some("zh_CN.UTF-8"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("zh-hans"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("zh-hant"))[0], TraditionalChinese);
        assert_eq!(script_order(Some("zh_TW"))[0], TraditionalChinese);
        assert_eq!(script_order(Some("zh-Hans-HK"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("ko-KR"))[0], Korean);
        for l in [None, Some(""), Some("en_US.UTF-8"), Some("C"), Some("de"), Some("!!@@")] {
            assert_eq!(script_order(l)[0], SimplifiedChinese, "{l:?}: Japanese forms are never first");
        }
        for l in ["ja", "ko", "zh-TW", "zh", "en"] {
            let o = script_order(Some(l));
            for s in [Japanese, SimplifiedChinese, TraditionalChinese, Korean] {
                assert!(o.contains(&s), "{l}: {o:?}");
            }
        }
    }

    #[test]
    fn classify_chars() {
        let order = script_order(Some("en"));
        assert_eq!(classify('a'), None);
        assert_eq!(classify('é'), None);
        assert_eq!(classify('Ω'), None, "Greek is in the built-in fonts");
        assert_eq!(classify('Ж'), None, "so is Cyrillic");
        assert_eq!(classify('图'), Some(CjkChar::Han));
        assert_eq!(classify('レ'), Some(CjkChar::Kana));
        assert_eq!(classify('카'), Some(CjkChar::Hangul));
        assert_eq!(classify('ㄅ'), Some(CjkChar::Bopomofo));
        assert_eq!(CjkChar::Kana.preferred(&order), Japanese);
        assert_eq!(CjkChar::Hangul.preferred(&order), Korean);
        assert_eq!(CjkChar::Han.preferred(&order), SimplifiedChinese);
        assert_eq!(CjkChar::Han.preferred(&script_order(Some("ja"))), Japanese);
    }

    #[test]
    fn face_index_handles_bad_data() {
        assert_eq!(face_index_for_family(b"", "PingFang SC"), 0);
        assert_eq!(face_index_for_family(b"ttcf\0\0\0\0garbage", "PingFang SC"), 0);
        assert_eq!(face_index_for_family(&[0xFF; 64], ""), 0);
    }

    /// On a Mac with PingFang, the SC and TC faces are found by name (not a hard-coded index).
    #[cfg(target_os = "macos")]
    #[test]
    fn pingfang_faces_by_name() {
        let Some(p) = mac_pingfang() else { return };
        let Ok(bytes) = std::fs::read(p) else { return };
        assert_ne!(face_index_for_family(&bytes, "PingFang SC"), face_index_for_family(&bytes, "PingFang TC"));
    }
}
