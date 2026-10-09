//! Lazy Japanese, Chinese and Korean fallback fonts for the UI (PhotoCraft's `cjk_fonts.rs`,
//! ported without its bundled fonts).
//!
//! The UI fonts (Inter, then egui's built-in fonts: `theme::font_definitions`) cover Latin, Greek
//! and Cyrillic but no CJK, and the system fonts that do are large (Hiragino ~10 MB, Apple SD
//! Gothic Neo 28 MB, PingFang 78 MB, Noto Sans CJK ~20 MB). Instead of reading them at startup,
//! an egui plugin scans each frame's text for CJK characters no registered font covers, and
//! registers the next system font for that character's script (`ctx.add_font`, active from the
//! next frame, which it requests). Fonts are appended at the lowest priority to every family, so
//! Latin text keeps Inter.
//!
//! The script order follows the UI language ([`cjk::script_order`]): Kana prefers a Japanese
//! font, Hangul a Korean one, Bopomofo a Traditional Chinese one, and Han the language's script.
//! At most one font is read per frame, each file at most once, and once every script has been
//! tried the scan stops.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FontData, FontFamily, FontId, Shape};
use std::path::PathBuf;

use crate::cjk::{self, CjkChar, CjkScript, FontFile};

/// Skip absurdly large files (a corrupt or non-font path must not eat memory).
pub const MAX_FONT_BYTES: u64 = 128 << 20;
/// Name prefix of the registered fallback fonts.
pub const FONT_PREFIX: &str = "system-cjk";

/// Where fonts come from; swapped out in tests.
#[derive(Clone)]
pub struct Sources {
    /// The language whose script order applies.
    pub locale: fn() -> Option<String>,
    pub files: fn(CjkScript) -> Vec<FontFile>,
    pub last_resort: fn() -> Vec<FontFile>,
}

impl Sources {
    /// The system's fonts, in the order of the UI language (or, for a non-CJK UI language, the
    /// system's first preferred language).
    pub fn system() -> Self {
        Self {
            locale: || match crate::i18n::current().code() {
                code @ ("ja" | "ko" | "zh-hans" | "zh-hant") => Some(code.to_string()),
                _ => crate::i18n::system_tags().first().cloned(),
            },
            files: cjk::font_files,
            last_resort: cjk::last_resort_files,
        }
    }
}

/// The lazy loader's state: which scripts were tried and which files were read.
pub struct CjkFallback {
    sources: Sources,
    /// `set_fonts` takes effect next pass: skip one frame so the new definitions are in place.
    defer_after_reset: bool,
    order: Option<[CjkScript; 4]>,
    tried: Vec<CjkScript>,
    last_resort_tried: bool,
    loaded: Vec<PathBuf>,
    /// Fonts registered so far: (name, path, face index).
    pub registered: Vec<(String, PathBuf, u32)>,
}

impl CjkFallback {
    pub fn new(sources: Sources) -> Self {
        Self { sources, defer_after_reset: false, order: None, tried: Vec::new(), last_resort_tried: false, loaded: Vec::new(), registered: Vec::new() }
    }

    /// Script order for the UI language (resolved on first use, not at startup).
    pub fn order(&mut self) -> [CjkScript; 4] {
        *self.order.get_or_insert_with(|| cjk::script_order((self.sources.locale)().as_deref()))
    }

    /// Every script and the last-resort fonts have been tried: nothing more to load.
    pub fn exhausted(&self) -> bool {
        self.tried.len() >= 4 && self.last_resort_tried
    }

    /// The next font to register for a character of kind `kind` that no current font covers:
    /// the first readable file of the first untried script (the character's preferred script,
    /// then the language's order), then the last-resort fonts. `None` when nothing is left.
    pub fn next_font(&mut self, kind: CjkChar) -> Option<(String, FontData)> {
        let order = self.order();
        let scripts: Vec<CjkScript> = std::iter::once(kind.preferred(&order)).chain(order).collect();
        for s in scripts {
            if self.tried.contains(&s) {
                continue;
            }
            self.tried.push(s);
            let files = (self.sources.files)(s);
            if let Some(f) = self.load_first(&files) {
                return Some(f);
            }
        }
        if !self.last_resort_tried {
            self.last_resort_tried = true;
            let files = (self.sources.last_resort)();
            return self.load_first(&files);
        }
        None
    }

    fn load_first(&mut self, files: &[FontFile]) -> Option<(String, FontData)> {
        for f in files {
            if self.loaded.contains(&f.path) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&f.path) else { continue };
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&f.path) else { continue };
            let index = cjk::face_index_for_family(&bytes, f.family);
            self.loaded.push(f.path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) as {name}", f.path.display());
            self.registered.push((name.clone(), f.path.clone(), index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        None
    }
}

/// First CJK character in the frame's text that the UI fonts can't draw.
fn missing_char(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape]) -> Option<char> {
    fn collect(shape: &Shape, out: &mut Vec<char>) {
        match shape {
            Shape::Text(t) => {
                let text = &t.galley.job.text;
                if !text.is_ascii() {
                    for c in text.chars().filter(|c| cjk::classify(*c).is_some()) {
                        if out.len() >= 256 {
                            return;
                        }
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut chars = Vec::new();
    for s in shapes {
        collect(&s.shape, &mut chars);
    }
    if chars.is_empty() {
        return None;
    }
    let font = FontId::proportional(12.0);
    ctx.fonts_mut(|f| chars.into_iter().find(|c| !f.has_glyph(&font, *c)))
}

/// (ascent, descent, line gap) of a face in em, from its `hhea` table; `descent` is negative.
fn vertical_metrics(font: &[u8], index: u32) -> Option<(f32, f32, f32)> {
    let u16_at = |o: usize| font.get(o..o.checked_add(2)?).and_then(|b| Some(u16::from_be_bytes([*b.first()?, *b.get(1)?])));
    let u32_at = |o: usize| font.get(o..o.checked_add(4)?).and_then(|b| Some(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?])));
    let i16_at = |o: usize| u16_at(o).map(|v| f32::from(v as i16));
    let dir = if font.get(..4)? == b"ttcf" {
        // Collection: the offset table of face `index` follows a 12-byte header.
        u32_at(12usize.checked_add(usize::try_from(index).ok()?.checked_mul(4)?)?)? as usize
    } else {
        0
    };
    let tables = u16_at(dir.checked_add(4)?)? as usize;
    let (mut hhea, mut head) = (None, None);
    for i in 0..tables {
        let rec = dir.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        let tag = font.get(rec..rec.checked_add(4)?)?;
        let off = u32_at(rec.checked_add(8)?)? as usize;
        match tag {
            b"hhea" => hhea = Some(off),
            b"head" => head = Some(off),
            _ => {}
        }
    }
    let (hhea, head) = (hhea?, head?);
    let upm = f32::from(u16_at(head.checked_add(18)?)?);
    if upm <= 0.0 {
        return None;
    }
    Some((i16_at(hhea.checked_add(4)?)? / upm, i16_at(hhea.checked_add(6)?)? / upm, i16_at(hhea.checked_add(8)?)? / upm))
}

/// `y_offset_factor` that puts `face`'s baseline where `primary`'s would be. egui centres a
/// fallback face's row in the primary font's row, so a face with a big line gap (Hiragino: 0.5em)
/// otherwise rides high next to Latin text.
fn baseline_offset(face: (f32, f32, f32), primary: (f32, f32, f32)) -> f32 {
    let height = |m: (f32, f32, f32)| m.0 - m.1 + m.2;
    let off = primary.0 - face.0 - 0.5 * (height(primary) - height(face));
    if off.is_finite() { off.clamp(-0.5, 0.5) } else { 0.0 }
}

/// Registers `name` at the lowest priority in every font family.
fn add_to_all_families(ctx: &egui::Context, name: String, mut data: FontData) {
    // Align to the first font of the proportional family (egui's built-in UI font).
    let primary = ctx.fonts(|f| {
        let defs = f.definitions();
        let first = defs.families.get(&FontFamily::Proportional).and_then(|v| v.first())?;
        defs.font_data.get(first).and_then(|p| vertical_metrics(&p.font, p.index))
    });
    if let (Some(face), Some(primary)) = (vertical_metrics(&data.font, data.index), primary) {
        data.tweak.y_offset_factor = baseline_offset(face, primary);
    }
    let mut families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
    for f in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&f) {
            families.push(f);
        }
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    ctx.add_font(FontInsert { name, data, families });
}

/// The egui plugin that watches drawn text and loads fallback fonts on demand.
pub struct CjkFontPlugin(pub CjkFallback);

impl egui::Plugin for CjkFontPlugin {
    fn debug_name(&self) -> &'static str {
        "artcraft-toolbox-cjk-fonts"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if std::mem::take(&mut self.0.defer_after_reset) {
            ctx.request_repaint();
            return;
        }
        if self.0.exhausted() {
            return;
        }
        let Some(c) = missing_char(ctx, &output.shapes) else { return };
        let Some(kind) = cjk::classify(c) else { return };
        if let Some((name, data)) = self.0.next_font(kind) {
            add_to_all_families(ctx, name, data);
            ctx.request_repaint();
        } else if !self.0.exhausted() {
            // Nothing readable for this script; try the next one next frame.
            ctx.request_repaint();
        }
    }
}

/// Reset the fonts to the built-in ones and start the lazy loader over (the system's fonts).
/// Done when the UI language changes: which script's glyph forms win depends on it.
pub fn install(ctx: &egui::Context) {
    install_with(ctx, Sources::system());
}

pub fn install_with(ctx: &egui::Context, sources: Sources) {
    ctx.set_fonts(crate::theme::font_definitions());
    // egui keeps the first plugin of a type: reset it rather than adding another.
    let fresh = || {
        let mut f = CjkFallback::new(sources.clone());
        f.defer_after_reset = true;
        f
    };
    if ctx.with_plugin::<CjkFontPlugin, _>(|plugin| plugin.0 = fresh()).is_none() {
        ctx.add_plugin(CjkFontPlugin(fresh()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

    /// egui's built-in UI font stands in for a CJK font (it has no CJK glyphs, so the loader keeps
    /// going: what is checked is which files it reads, in which order).
    fn fake_dir() -> PathBuf {
        let mut g = DIR.lock().unwrap();
        g.get_or_insert_with(|| {
            let dir = std::env::temp_dir().join(format!("artcraft-toolbox-cjk-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let defs = egui::FontDefinitions::default();
            let first = &defs.families[&FontFamily::Proportional][0];
            std::fs::write(dir.join("font.ttf"), &*defs.font_data[first].font).unwrap();
            std::fs::write(dir.join("empty.ttf"), b"").unwrap();
            dir
        })
        .clone()
    }

    fn fake(name: &str) -> FontFile {
        FontFile { path: fake_dir().join(name), family: "" }
    }

    fn fake_sources(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            // Korean has a readable "font", Japanese only broken files, Chinese none.
            files: |s| match s {
                CjkScript::Korean => vec![fake("missing.ttc"), fake("empty.ttf"), fake("font.ttf")],
                CjkScript::Japanese => vec![fake("empty.ttf")],
                _ => vec![],
            },
            last_resort: || vec![fake("font.ttf")],
        }
    }

    #[test]
    fn picks_preferred_script_skips_unreadable_and_exhausts() {
        let mut fb = CjkFallback::new(fake_sources(|| Some("en_US".into())));
        assert!(!fb.exhausted());
        let (name, data) = fb.next_font(CjkChar::Hangul).expect("korean font");
        assert!(name.starts_with(FONT_PREFIX));
        assert_eq!(data.index, 0);
        assert_eq!(fb.tried, vec![CjkScript::Korean]);
        // Han for English: SC, TC, JA have nothing readable and the last-resort file was already
        // read: nothing new, and everything is tried.
        assert!(fb.next_font(CjkChar::Han).is_none());
        assert!(fb.exhausted());
        assert_eq!(fb.tried, vec![CjkScript::Korean, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Japanese]);
        assert_eq!(fb.registered.len(), 1, "a file is read at most once");
        assert!(fb.next_font(CjkChar::Kana).is_none());
    }

    #[test]
    fn han_follows_the_language() {
        for (loc, first) in
            [("ja", CjkScript::Japanese), ("zh-hans", CjkScript::SimplifiedChinese), ("zh-hant", CjkScript::TraditionalChinese), ("ko", CjkScript::Korean)]
        {
            let mut fb = CjkFallback::new(Sources { locale: || None, files: |_| vec![], last_resort: Vec::new });
            fb.order = Some(cjk::script_order(Some(loc)));
            assert!(fb.next_font(CjkChar::Han).is_none());
            assert_eq!(fb.tried[0], first, "{loc}");
        }
    }

    #[test]
    fn vertical_metrics_reject_garbage_and_offsets_stay_bounded() {
        for bad in [&b""[..], b"ttcf", b"ttcf\0\0\0\0\0\0\0\0\xff\xff\xff\xff", b"\0\x01\0\0\0\xff\0\0\0\0\0\0"] {
            assert!(vertical_metrics(bad, 0).is_none());
            assert!(vertical_metrics(bad, u32::MAX).is_none());
        }
        let defs = egui::FontDefinitions::default();
        let ui = &defs.font_data[&defs.families[&FontFamily::Proportional][0]];
        let m = vertical_metrics(&ui.font, 0).expect("the built-in font's metrics");
        assert_eq!(baseline_offset(m, m), 0.0);
        assert!((baseline_offset((0.88, -0.12, 0.5), m)).abs() <= 0.5);
        assert_eq!(baseline_offset((f32::NAN, 0.0, 0.0), m), 0.0);
    }

    /// Runs frames showing `text` until fonts settle; returns whether every glyph is covered.
    fn render(ctx: &egui::Context, text: &str) -> bool {
        for _ in 0..12 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                ui.label(text);
            });
            out.textures_delta.clear();
        }
        ctx.fonts_mut(|f| f.has_glyphs(&FontId::proportional(12.0), &text.replace(' ', "")))
    }

    #[test]
    fn latin_greek_and_cyrillic_load_nothing_and_cjk_registers_a_font_last() {
        let ctx = egui::Context::default();
        install_with(&ctx, fake_sources(|| Some("ko".into())));
        assert!(render(&ctx, "Check for updates · Ελέγχος · Проверка · Čeština"), "the built-in font covers these");
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 0, "no CJK text, no font read");
        let _ = render(&ctx, "업데이트 확인");
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let name = format!("{FONT_PREFIX}-0");
        assert!(fonts.font_data.contains_key(&name));
        for (fam, stack) in &fonts.families {
            assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps the built-in font");
        }
    }

    /// With the real system fonts, every CJK sample renders (skipped per script when the machine
    /// has no font for it, e.g. CI Linux without Noto CJK).
    #[test]
    fn system_fonts_cover_cjk_samples() {
        let available = |s: CjkScript| cjk::font_files(s).iter().any(|f| f.path.is_file());
        let samples = [
            ("检查更新", CjkScript::SimplifiedChinese),
            ("檢查更新", CjkScript::TraditionalChinese),
            ("업데이트 확인", CjkScript::Korean),
            ("アップデート", CjkScript::Japanese),
        ];
        for (text, script) in samples {
            if !available(script) {
                eprintln!("skipping {text}: no {script:?} system font");
                continue;
            }
            let ctx = egui::Context::default();
            install(&ctx);
            assert!(render(&ctx, text), "{text} still has missing glyphs");
        }
    }
}
