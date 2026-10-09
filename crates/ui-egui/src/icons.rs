//! Icons: Lucide SVGs (ISC, `assets/icons/`), embedded, recoloured to white, rasterized by the
//! egui_extras SVG loader at their on-screen pixel size (crisp at any scale) and tinted per use.
//! PhotoCraft's `icons.rs`, ported. Never a Unicode symbol: egui's fonts lack most of them
//! (docs/ui-design.md › Text).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::{Color32, Rect, Vec2};

/// Every icon the UI uses, by its Lucide name.
pub static ICONS: &[(&str, &[u8])] = &[
    ("arrow-left", include_bytes!("../../../assets/icons/arrow-left.svg")),
    ("check", include_bytes!("../../../assets/icons/check.svg")),
    ("chevron-down", include_bytes!("../../../assets/icons/chevron-down.svg")),
    ("chevron-right", include_bytes!("../../../assets/icons/chevron-right.svg")),
    ("download", include_bytes!("../../../assets/icons/download.svg")),
    ("ellipsis-vertical", include_bytes!("../../../assets/icons/ellipsis-vertical.svg")),
    ("external-link", include_bytes!("../../../assets/icons/external-link.svg")),
    ("folder-open", include_bytes!("../../../assets/icons/folder-open.svg")),
    ("info", include_bytes!("../../../assets/icons/info.svg")),
    ("refresh-cw", include_bytes!("../../../assets/icons/refresh-cw.svg")),
    ("search", include_bytes!("../../../assets/icons/search.svg")),
    ("settings", include_bytes!("../../../assets/icons/settings.svg")),
    ("trash", include_bytes!("../../../assets/icons/trash.svg")),
    ("triangle-alert", include_bytes!("../../../assets/icons/triangle-alert.svg")),
    ("x", include_bytes!("../../../assets/icons/x.svg")),
];

/// The toolbox's own mark (the placeholder app icon until M5), for the header.
pub static LOGO: &[u8] = include_bytes!("../../../assets/app-icon/artcraft-toolbox.svg");

fn white_icons() -> &'static HashMap<&'static str, Arc<[u8]>> {
    static MAP: OnceLock<HashMap<&'static str, Arc<[u8]>>> = OnceLock::new();
    MAP.get_or_init(|| {
        ICONS
            .iter()
            .map(|(name, bytes)| {
                let svg = String::from_utf8_lossy(bytes).replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
                (*name, Arc::from(svg.into_bytes().into_boxed_slice()))
            })
            .collect()
    })
}

/// Is `name` one of [`ICONS`]?
pub fn exists(name: &str) -> bool {
    white_icons().contains_key(name)
}

/// An egui image of an icon, `size` points square, tinted. An unknown name draws nothing.
pub fn image(name: &str, size: f32, tint: Color32) -> egui::Image<'static> {
    let bytes = white_icons().get(name).cloned().unwrap_or_else(|| Arc::from(&b""[..]));
    egui::Image::from_bytes(format!("bytes://icons/{name}.svg"), egui::load::Bytes::Shared(bytes)).fit_to_exact_size(Vec2::splat(size)).tint(tint)
}

/// Paint an icon centred in `rect`.
pub fn paint(ui: &egui::Ui, rect: Rect, name: &str, size: f32, tint: Color32) {
    image(name, size, tint).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(size)));
}

/// The toolbox's mark, `size` points square, in its own colours.
pub fn logo(size: f32) -> egui::Image<'static> {
    egui::Image::from_bytes("bytes://artcraft-toolbox-logo.svg", egui::load::Bytes::Static(LOGO)).fit_to_exact_size(Vec2::splat(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_white_svg() {
        for (name, _) in ICONS {
            let svg = white_icons().get(name).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            assert!(svg.starts_with("<svg") && svg.contains("#ffffff") && !svg.contains("currentColor"), "{name}");
        }
        assert!(exists("search") && !exists("no-such-icon"));
        assert!(String::from_utf8_lossy(LOGO).contains("<svg"));
    }
}
