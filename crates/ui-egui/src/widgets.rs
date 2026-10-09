//! Shared widgets. Colours come from [`Tokens`], icons from [`crate::icons`].

use egui::{Align2, Color32, CornerRadius, FontId, Response, Sense, Stroke, Vec2, WidgetInfo, WidgetType};

use crate::icons;
use crate::theme::{self, Tokens};

/// Side of an app tile in the list, in points.
pub const TILE: f32 = 44.0;

/// An app's tile, `side` points square: its own icon once fetched (docs/release-contract.md ›
/// Icons), else a monogram on a colour derived from its id, so every craft is recognisable before
/// its icon arrives or when it can't be fetched. `badge` puts a dot on its corner (an update is
/// waiting), ringed in `ring`, the colour behind the tile.
pub fn app_tile(ui: &mut egui::Ui, id: &str, name: &str, icon: Option<&egui::TextureHandle>, side: f32, badge: Option<(Color32, Color32)>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    // Rounded like an app icon on a desktop: about a fifth of the side.
    let radius = CornerRadius::same((side * 0.22).round().clamp(0.0, 255.0) as u8);
    match icon {
        Some(tex) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), rect.size())).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, radius, tile_color(id));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, monogram(name), theme::semibold(side * 0.4), Color32::WHITE);
        }
    }
    if let Some((dot, ring)) = badge {
        let center = rect.right_bottom() - Vec2::splat(side * 0.08);
        ui.painter().circle(center, side * 0.13, dot, Stroke::new(2.0, ring));
    }
}

/// `PhotoCraft` → `Ph`, `CADCraft` → `CA`: the first two letters of the name before `Craft`.
pub fn monogram(name: &str) -> String {
    let base = name.strip_suffix("Craft").filter(|b| !b.is_empty()).unwrap_or(name);
    let mut chars = base.chars().filter(|c| c.is_alphanumeric());
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    let second = chars.next().map(|c| c.to_string()).unwrap_or_default();
    format!("{first}{second}")
}

/// A stable, readable colour per id (FNV-1a hash to a hue; fixed saturation and value).
pub fn tile_color(id: &str) -> Color32 {
    let hash = id.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    let hue = (hash % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.55, 0.72, 1.0).into()
}

/// A rounded panel around `add`: the tab strip, each section of the list.
pub fn panel<R>(ui: &mut egui::Ui, t: &Tokens, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::NONE.fill(t.card).stroke(Stroke::new(1.0, t.outline)).corner_radius(CornerRadius::same(t.radius_lg)).inner_margin(egui::Margin::same(6)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        },
    )
}

/// [`panel`] with roomier margins, for the details and settings pages.
pub fn card<R>(ui: &mut egui::Ui, t: &Tokens, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    panel(ui, t, |ui| {
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui)
            })
            .inner
    })
}

/// A panel's title, in the panel: `Installed`, `Settings for PhotoCraft`.
pub fn panel_title(ui: &mut egui::Ui, title: &str, t: &Tokens) {
    ui.add(egui::Label::new(egui::RichText::new(title).font(theme::medium(12.5)).color(t.text_dim)).truncate());
}

/// [`panel_title`] with something on its right (a button), on one line.
pub fn panel_title_with(ui: &mut egui::Ui, title: &str, t: &Tokens, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| panel_title(ui, title, t));
        });
    });
}

/// A heading above a panel or a list: `Versions`.
pub fn subhead(ui: &mut egui::Ui, title: &str, t: &Tokens) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        panel_title(ui, title, t);
    });
}

/// A 2-point ring in the accent colour around a widget that has the keyboard focus.
pub fn focus_ring(ui: &egui::Ui, response: &Response, radius: u8, t: &Tokens) {
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect, CornerRadius::same(radius), Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
    }
}

/// An icon-only button, 28 points square: the icon dim, brighter on hover with a rounded fill
/// (also while `active`). `name` is what screen readers say and the tooltip.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, name: &str, active: bool, t: &Tokens) -> Response {
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    if ui.is_rect_visible(rect) {
        let lit = enabled && (response.hovered() || active);
        if lit {
            ui.painter().rect_filled(rect, CornerRadius::same(t.radius_sm), t.card_hover);
        }
        focus_ring(ui, &response, t.radius_sm, t);
        let tint = match (enabled, lit) {
            (false, _) => t.text_faint,
            (true, true) => t.text,
            (true, false) => t.text_dim,
        };
        icons::paint(ui, rect, icon, 17.0, tint);
    }
    let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    response.on_hover_text(name)
}

/// A tab with an underline when selected (`Apps`, `Settings`).
pub fn tab(ui: &mut egui::Ui, label: &str, selected: bool, t: &Tokens) -> Response {
    let font = theme::medium(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 16.0, 30.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    if ui.is_rect_visible(rect) {
        let color = if selected || response.hovered() { t.text } else { t.text_dim };
        let pos = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, color);
        if selected {
            let line = egui::Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 8.0, rect.bottom()));
            ui.painter().rect_filled(line, CornerRadius::same(1), t.accent);
        }
        focus_ring(ui, &response, t.radius_sm, t);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The button for the action that matters most on a row or page (Update): filled with the accent.
pub fn primary_button(label: &str, t: &Tokens) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.to_string()).color(t.accent_text)).fill(t.accent).stroke(Stroke::NONE)
}

/// A setting that is on or off: its label on the left (wrapping), a switch on the right. Clicking
/// either flips it; screen readers hear a checkbox. Returns whether it changed.
pub fn toggle_row(ui: &mut egui::Ui, on: &mut bool, label: &str, t: &Tokens) -> Response {
    let enabled = ui.is_enabled();
    let switch = egui::vec2(34.0, 20.0);
    let width = ui.available_width();
    let galley = ui.painter().layout(label.to_string(), FontId::proportional(13.0), Color32::PLACEHOLDER, (width - switch.x - 12.0).max(40.0));
    let height = galley.size().y.max(switch.y) + 8.0;
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, label));
    if ui.is_rect_visible(rect) {
        let text = if enabled { t.text } else { t.text_faint };
        ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, text);
        let track = egui::Rect::from_min_size(egui::pos2(rect.right() - switch.x, rect.center().y - switch.y / 2.0), switch);
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let fill = if !enabled {
            t.border
        } else if *on {
            t.accent
        } else {
            t.text_faint
        };
        ui.painter().rect_filled(track, CornerRadius::same(10), fill);
        let knob_x = egui::lerp((track.left() + 10.0)..=(track.right() - 10.0), how_on);
        let knob = if enabled { Color32::WHITE } else { t.card_hover };
        ui.painter().circle_filled(egui::pos2(knob_x, track.center().y), 7.0, knob);
        if response.has_focus() {
            ui.painter().rect_stroke(track.expand(2.0), CornerRadius::same(12), Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
        }
    }
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// The font of an app's name in the list.
pub fn name_font() -> FontId {
    theme::semibold(14.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monograms() {
        assert_eq!(monogram("PhotoCraft"), "Ph");
        assert_eq!(monogram("CADCraft"), "CA");
        assert_eq!(monogram("PdfCraft"), "Pd");
        assert_eq!(monogram("Craft"), "Cr");
        assert_eq!(monogram(""), "");
        assert_eq!(monogram("élan"), "Él");
    }

    #[test]
    fn tile_colors_are_stable_and_distinct() {
        assert_eq!(tile_color("photocraft"), tile_color("photocraft"));
        assert_ne!(tile_color("photocraft"), tile_color("vectorcraft"));
    }
}
