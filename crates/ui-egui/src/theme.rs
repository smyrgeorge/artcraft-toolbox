//! Design tokens. Colours and radii come from here, never hard-coded in a widget
//! (`Tokens::get(ctx)`). Two themes, dark and light, picked by the `theme` setting (`system`
//! follows the OS); docs/ui-design.md has the rules. Every text colour meets WCAG AA (4.5:1) on
//! the surfaces it is drawn on, in both themes (`text_meets_wcag_aa`).

use artcraft_toolbox_model::ThemePref;
use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Theme, ThemePreference, Visuals};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Header and status bar.
    pub chrome: Color32,
    /// Behind the app list.
    pub bg: Color32,
    /// App rows and cards.
    pub card: Color32,
    /// Buttons, and a hovered card.
    pub card_hover: Color32,
    pub border: Color32,
    /// Inputs.
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Least important text (the version in the status bar); only on `chrome`.
    pub text_faint: Color32,
    /// Fills: the selected tab, progress bars. Text on it is `accent_text`.
    pub accent: Color32,
    /// The accent as text and links on `bg` and `card`.
    pub accent_fg: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub radius_sm: u8,
    pub radius: u8,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark: true,
        chrome: Color32::from_rgb(0x1b, 0x1c, 0x1f),
        bg: Color32::from_rgb(0x23, 0x24, 0x28),
        card: Color32::from_rgb(0x2b, 0x2d, 0x31),
        card_hover: Color32::from_rgb(0x33, 0x35, 0x3a),
        border: Color32::from_rgb(0x3a, 0x3d, 0x43),
        field: Color32::from_rgb(0x1e, 0x1f, 0x22),
        text: Color32::from_rgb(0xe3, 0xe5, 0xe8),
        text_dim: Color32::from_rgb(0xa0, 0xa4, 0xab),
        text_faint: Color32::from_rgb(0x8b, 0x90, 0x98),
        accent: Color32::from_rgb(0x2f, 0x74, 0xd6),
        accent_fg: Color32::from_rgb(0x5a, 0xa3, 0xf5),
        accent_text: Color32::WHITE,
        success: Color32::from_rgb(0x5f, 0xb8, 0x65),
        warning: Color32::from_rgb(0xe5, 0xb5, 0x5b),
        danger: Color32::from_rgb(0xef, 0x81, 0x89),
        radius_sm: 4,
        radius: 8,
    };

    pub const LIGHT: Tokens = Tokens {
        dark: false,
        chrome: Color32::from_rgb(0xec, 0xee, 0xf2),
        bg: Color32::from_rgb(0xf4, 0xf5, 0xf7),
        card: Color32::WHITE,
        card_hover: Color32::from_rgb(0xf0, 0xf2, 0xf5),
        border: Color32::from_rgb(0xd3, 0xd7, 0xde),
        field: Color32::WHITE,
        text: Color32::from_rgb(0x1c, 0x1e, 0x22),
        text_dim: Color32::from_rgb(0x52, 0x58, 0x62),
        text_faint: Color32::from_rgb(0x62, 0x68, 0x72),
        accent: Color32::from_rgb(0x1c, 0x64, 0xc9),
        accent_fg: Color32::from_rgb(0x1c, 0x64, 0xc9),
        accent_text: Color32::WHITE,
        success: Color32::from_rgb(0x2a, 0x76, 0x32),
        warning: Color32::from_rgb(0x8a, 0x56, 0x00),
        danger: Color32::from_rgb(0xbf, 0x30, 0x3b),
        radius_sm: 4,
        radius: 8,
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

fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.card;
    v.hyperlink_color = t.accent_fg;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.danger;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.accent_text);
    let r = CornerRadius::same(t.radius_sm);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    // Default label colour. (Not `override_text_color`: that would also force link text, in
    // release notes, to the label colour.)
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    for (wv, bg) in [(&mut w.inactive, t.card_hover), (&mut w.hovered, t.border), (&mut w.active, t.border), (&mut w.open, t.border)] {
        wv.bg_fill = bg;
        wv.weak_bg_fill = bg;
        // Light surfaces need an outline to show where a button is.
        wv.bg_stroke = if t.dark { Stroke::NONE } else { Stroke::new(1.0, t.border) };
        wv.fg_stroke = Stroke::new(1.0, t.text);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    v
}

/// Install both themes' visuals and the shared text styles and spacing. Call once.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&Tokens::DARK));
    ctx.set_visuals_of(Theme::Light, visuals(&Tokens::LIGHT));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(16.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 4.0);
        s.spacing.interact_size = egui::vec2(24.0, 26.0);
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
            let mut pairs = vec![("text_faint on chrome", t.text_faint, t.chrome), ("accent_text on accent", t.accent_text, t.accent)];
            for (fg_name, fg) in
                [("text", t.text), ("text_dim", t.text_dim), ("accent_fg", t.accent_fg), ("success", t.success), ("warning", t.warning), ("danger", t.danger)]
            {
                for (bg_name, bg) in [("chrome", t.chrome), ("bg", t.bg), ("card", t.card), ("field", t.field)] {
                    pairs.push((Box::leak(format!("{fg_name} on {bg_name}").into_boxed_str()), fg, bg));
                }
            }
            pairs.push(("text on card_hover (buttons)", t.text, t.card_hover));
            pairs.push(("text on border (hovered buttons)", t.text, t.border));
            for (what, fg, bg) in pairs {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{} theme: {what} is {c:.2}:1", if t.dark { "dark" } else { "light" });
            }
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
