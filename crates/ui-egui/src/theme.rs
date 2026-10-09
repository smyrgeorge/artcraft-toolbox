//! Design tokens. Colours and radii come from here, never hard-coded in a widget
//! (`Tokens::get(ctx)`). One dark theme for now; docs/ui-design.md has the rules.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Visuals};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Header and status bar.
    pub chrome: Color32,
    /// Behind the app list.
    pub bg: Color32,
    /// App rows and cards.
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    /// Inputs.
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub radius_sm: u8,
    pub radius: u8,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        chrome: Color32::from_rgb(0x1b, 0x1c, 0x1f),
        bg: Color32::from_rgb(0x23, 0x24, 0x28),
        card: Color32::from_rgb(0x2b, 0x2d, 0x31),
        card_hover: Color32::from_rgb(0x33, 0x35, 0x3a),
        border: Color32::from_rgb(0x3a, 0x3d, 0x43),
        field: Color32::from_rgb(0x1e, 0x1f, 0x22),
        text: Color32::from_rgb(0xe3, 0xe5, 0xe8),
        text_dim: Color32::from_rgb(0xa0, 0xa4, 0xab),
        text_faint: Color32::from_rgb(0x6f, 0x73, 0x7a),
        accent: Color32::from_rgb(0x37, 0x8e, 0xf0),
        accent_text: Color32::WHITE,
        success: Color32::from_rgb(0x5f, 0xb8, 0x65),
        warning: Color32::from_rgb(0xe5, 0xb5, 0x5b),
        danger: Color32::from_rgb(0xe0, 0x6c, 0x75),
        radius_sm: 4,
        radius: 8,
    };

    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::new("artcraft-toolbox-theme"))).unwrap_or(Tokens::DARK)
    }
}

pub fn apply(ctx: &egui::Context) {
    let t = Tokens::DARK;
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("artcraft-toolbox-theme"), t));
    let mut v = Visuals::dark();
    v.panel_fill = t.bg;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.card;
    v.hyperlink_color = t.accent;
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
        wv.bg_stroke = Stroke::NONE;
        wv.fg_stroke = Stroke::new(1.0, t.text);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
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
