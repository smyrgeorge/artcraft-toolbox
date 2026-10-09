//! Design tokens, fonts and egui visuals. Colours and radii come from here, never hard-coded in
//! a widget (`Tokens::get(ctx)`). The look is PhotoCraft's Studio theme, adapted: near-black (or
//! light grey) surfaces, rounded panels, Inter, a soft violet accent. Two themes, dark and light,
//! picked by the `theme` setting (`system` follows the OS); docs/ui-design.md has the rules. Every
//! text colour meets WCAG AA (4.5:1) on the surfaces it is drawn on, in both themes
//! (`text_meets_wcag_aa`).

use std::sync::Arc;

use artcraft_toolbox_model::ThemePref;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Theme, ThemePreference, Visuals};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Behind everything: the window (or popover), the header.
    pub bg: Color32,
    /// Panels: the tab strip, each section of the list, cards on the details and settings pages.
    pub card: Color32,
    /// A hovered row or button.
    pub card_hover: Color32,
    /// Button outlines, the progress bar's track.
    pub border: Color32,
    /// A panel's edge: barely there on dark surfaces, a hairline on light ones.
    pub outline: Color32,
    /// Inputs.
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Least important text (the list's footer).
    pub text_faint: Color32,
    /// Fills: the primary button, the selected tab's underline, progress, focus rings. Text on it
    /// is `accent_text`.
    pub accent: Color32,
    /// The accent as text: "update available", progress phases, links.
    pub accent_fg: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub radius_sm: u8,
    pub radius: u8,
    /// Panels and the popover's corners.
    pub radius_lg: u8,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark: true,
        bg: Color32::from_rgb(19, 19, 22),
        card: Color32::from_rgb(28, 28, 32),
        card_hover: Color32::from_rgb(39, 39, 45),
        border: Color32::from_rgb(48, 48, 55),
        outline: Color32::from_rgb(34, 34, 39),
        field: Color32::from_rgb(21, 21, 25),
        text: Color32::from_rgb(236, 236, 241),
        text_dim: Color32::from_rgb(160, 160, 172),
        text_faint: Color32::from_rgb(142, 142, 153),
        accent: Color32::from_rgb(108, 92, 231),
        accent_fg: Color32::from_rgb(174, 162, 255),
        accent_text: Color32::WHITE,
        success: Color32::from_rgb(99, 201, 125),
        warning: Color32::from_rgb(238, 190, 92),
        danger: Color32::from_rgb(242, 118, 118),
        radius_sm: 6,
        radius: 8,
        radius_lg: 12,
    };

    pub const LIGHT: Tokens = Tokens {
        dark: false,
        bg: Color32::from_rgb(243, 243, 246),
        card: Color32::WHITE,
        card_hover: Color32::from_rgb(238, 238, 243),
        border: Color32::from_rgb(222, 222, 229),
        outline: Color32::from_rgb(228, 228, 234),
        field: Color32::from_rgb(246, 246, 249),
        text: Color32::from_rgb(24, 24, 28),
        text_dim: Color32::from_rgb(88, 88, 100),
        text_faint: Color32::from_rgb(104, 104, 116),
        accent: Color32::from_rgb(108, 92, 231),
        accent_fg: Color32::from_rgb(84, 66, 204),
        accent_text: Color32::WHITE,
        success: Color32::from_rgb(28, 122, 52),
        warning: Color32::from_rgb(146, 92, 0),
        danger: Color32::from_rgb(196, 48, 58),
        radius_sm: 6,
        radius: 8,
        radius_lg: 12,
    };

    pub fn for_theme(theme: Theme) -> Tokens {
        match theme {
            Theme::Dark => Tokens::DARK,
            Theme::Light => Tokens::LIGHT,
        }
    }

    /// The tokens of the theme egui is drawing in (the setting, or the system's).
    pub fn get(ctx: &egui::Context) -> Tokens {
        Tokens::for_theme(ctx.theme())
    }
}

/// Inter at the weights the UI uses (PhotoCraft's UI font, SIL OFL 1.1, `assets/fonts/`).
static INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// The `medium` font family: Inter Medium (buttons, tabs).
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

/// The `semibold` font family: Inter SemiBold (app names, headings).
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

/// The UI's fonts: Inter first, egui's built-in fonts behind it for anything Inter lacks, and the
/// named weights. The system's CJK fonts are added on demand ([`crate::cjk_fonts`]).
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [("Inter", INTER_REGULAR), ("Inter-Medium", INTER_MEDIUM), ("Inter-SemiBold", INTER_SEMIBOLD)] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".to_owned());
    for (family, primary) in [("medium", "Inter-Medium"), ("semibold", "Inter-SemiBold")] {
        let mut stack = vec![primary.to_owned()];
        stack.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), stack);
    }
    fonts
}

/// Have [`font_definitions`] reached the active fonts? (`set_fonts` takes effect next pass;
/// until then the named weights the text styles use don't exist.)
pub fn fonts_active(ctx: &egui::Context) -> bool {
    ctx.fonts(|f| f.definitions().families.contains_key(&FontFamily::Name("semibold".into())))
}

fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(t.radius_lg);
    v.menu_corner_radius = CornerRadius::same(t.radius);
    v.extreme_bg_color = t.field;
    v.text_edit_bg_color = Some(t.field);
    v.faint_bg_color = t.card;
    v.hyperlink_color = t.accent_fg;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.danger;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.accent_text);
    v.window_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(if t.dark { 120 } else { 40 }) };
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 36 }) };
    let r = CornerRadius::same(t.radius_sm);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    // Default label colour. (Not `override_text_color`: that would also force link text, in
    // release notes, to the label colour.)
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.corner_radius = r;
    // Buttons and combo boxes are outlined; hovering fills them.
    for (wv, bg) in [(&mut w.inactive, egui::Color32::TRANSPARENT), (&mut w.hovered, t.card_hover), (&mut w.active, t.border), (&mut w.open, t.card_hover)] {
        wv.bg_fill = if bg == Color32::TRANSPARENT { t.field } else { bg };
        wv.weak_bg_fill = bg;
        wv.bg_stroke = Stroke::new(1.0, t.border);
        wv.fg_stroke = Stroke::new(1.0, t.text);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    v
}

/// Install both themes' visuals and the shared text styles and spacing, and the SVG loader for
/// the icons. Call once. (The fonts are installed with the UI language, [`crate::cjk_fonts`].)
pub fn apply(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    ctx.set_visuals_of(Theme::Dark, visuals(&Tokens::DARK));
    ctx.set_visuals_of(Theme::Light, visuals(&Tokens::LIGHT));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, medium(12.5)),
            (TextStyle::Heading, semibold(17.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 3.0);
        s.spacing.interact_size = egui::vec2(24.0, 24.0);
        s.spacing.menu_margin = egui::Margin::same(6);
    });
}

/// Follow the `theme` setting (`system`: the OS's light or dark mode, as egui reports it).
pub fn set_preference(ctx: &egui::Context, pref: ThemePref) {
    let want = match pref {
        ThemePref::System => ThemePreference::System,
        ThemePref::Dark => ThemePreference::Dark,
        ThemePref::Light => ThemePreference::Light,
    };
    if ctx.options(|o| o.theme_preference) != want {
        ctx.set_theme(want);
    }
}

/// WCAG 2 contrast ratio of two colours (1.0 to 21.0).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let c = f32::from(v) / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every text colour against every surface it is drawn on, in both themes: WCAG AA for
    /// normal-size text is 4.5:1.
    #[test]
    fn text_meets_wcag_aa() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            let mut pairs = vec![("accent_text on accent", t.accent_text, t.accent)];
            for (fg_name, fg) in [
                ("text", t.text),
                ("text_dim", t.text_dim),
                ("text_faint", t.text_faint),
                ("accent_fg", t.accent_fg),
                ("success", t.success),
                ("warning", t.warning),
                ("danger", t.danger),
            ] {
                for (bg_name, bg) in [("bg", t.bg), ("card", t.card), ("card_hover", t.card_hover), ("field", t.field)] {
                    pairs.push((Box::leak(format!("{fg_name} on {bg_name}").into_boxed_str()), fg, bg));
                }
            }
            pairs.push(("text on border (pressed buttons)", t.text, t.border));
            for (what, fg, bg) in pairs {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{} theme: {what} is {c:.2}:1", if t.dark { "dark" } else { "light" });
            }
        }
    }

    #[test]
    fn inter_is_the_ui_font_with_named_weights() {
        let fonts = font_definitions();
        assert_eq!(fonts.families[&FontFamily::Proportional].first().map(String::as_str), Some("Inter"));
        for family in ["medium", "semibold"] {
            let stack = &fonts.families[&FontFamily::Name(family.into())];
            assert!(stack.len() > 1, "{family} falls back to the built-in fonts");
        }
    }

    #[test]
    fn contrast_of_known_pairs() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
    }

    #[test]
    fn tokens_follow_the_theme() {
        let ctx = egui::Context::default();
        apply(&ctx);
        set_preference(&ctx, ThemePref::Light);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        assert_eq!(Tokens::get(&ctx), Tokens::LIGHT);
        set_preference(&ctx, ThemePref::Dark);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        assert_eq!(Tokens::get(&ctx), Tokens::DARK);
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::DARK.bg);
    }
}
