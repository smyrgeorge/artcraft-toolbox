//! Shared widgets. Colours come from [`Tokens`].

use egui::{Align2, Color32, CornerRadius, FontId, Sense, Stroke, Vec2};

use crate::theme::Tokens;

/// Side of an app tile, in points.
pub const TILE: f32 = 36.0;

/// An app's tile: a monogram on a colour derived from its id, so every craft is recognisable
/// without bundling its logo (docs/ui-design.md › App tiles).
pub fn app_tile(ui: &mut egui::Ui, id: &str, name: &str, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(t.radius), tile_color(id));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, monogram(name), FontId::proportional(15.0), Color32::WHITE);
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

/// A rounded, full-width card around `add`.
pub fn card<R>(ui: &mut egui::Ui, t: &Tokens, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(t.radius))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
}

/// A section heading: `INSTALLED · 2`.
pub fn section(ui: &mut egui::Ui, title: &str, count: usize, t: &Tokens) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(format!("{} · {count}", title.to_uppercase())).small().strong().color(t.text_dim));
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
