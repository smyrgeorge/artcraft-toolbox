//! The UI drives the engine: rows come from the session, and controls dispatch commands.
//! Runs without a GPU (no rendering, accessibility tree only).

use artcraft_toolbox_engine::Session;
use artcraft_toolbox_model::Channel;
use artcraft_toolbox_ui_egui::ToolboxApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn harness() -> Harness<'static, ToolboxApp> {
    let app = ToolboxApp::new(Session::new().unwrap());
    let mut h = Harness::builder().with_size(egui::vec2(440.0, 1400.0)).build_ui_state(
        |ui, app: &mut ToolboxApp| {
            ToolboxApp::setup_context(ui.ctx());
            app.show(ui);
        },
        app,
    );
    h.run();
    h
}

#[test]
fn lists_every_catalog_app() {
    let h = harness();
    for name in ["PhotoCraft", "VectorCraft", "FilmCraft", "LightCraft", "PdfCraft", "EffectCraft", "DesignCraft", "WordCraft"] {
        h.get_by_label(name);
    }
}

#[test]
fn search_filters_the_list() {
    let mut h = harness();
    h.state_mut().ui.search = "vector".into();
    h.run();
    h.get_by_label("VectorCraft");
    assert!(h.query_by_label("PhotoCraft").is_none());
}

#[test]
fn settings_changes_go_through_the_engine() {
    let mut h = harness();
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Pre-release").click();
    h.run();
    assert_eq!(h.state().session.settings().channel, Channel::Prerelease);
    assert!(h.state().notice.is_none());
}

/// Every status line must render with the bundled fonts: a missing glyph draws as a box (the
/// `→` in "0.3.0 → 0.5.0" once did). Add new user-facing symbols here.
#[test]
fn status_text_has_no_missing_glyphs() {
    use artcraft_toolbox_engine::{Status, Version};
    let h = harness();
    let (a, b) = (Version::new(0, 3, 0), Version::new(0, 5, 0));
    let mut texts: Vec<String> = [
        Status::Unknown { installed: None },
        Status::Unknown { installed: Some(a.clone()) },
        Status::NotInstalled { latest: b.clone() },
        Status::UpToDate { installed: a.clone() },
        Status::UpdateAvailable { installed: a, latest: b },
        Status::Unsupported { installed: None },
    ]
    .iter()
    .map(Status::label)
    .collect();
    texts.extend(["INSTALLED · 1", "No app matches “x”", "0.1.0 (dev build)"].map(String::from));
    for t in &texts {
        let missing: Vec<char> = t.chars().filter(|c| !h.ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(13.0), *c))).collect();
        assert!(missing.is_empty(), "{t:?} has no glyph for {missing:?}");
    }
}
