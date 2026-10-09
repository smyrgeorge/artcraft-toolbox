//! Languages, themes, text size and accessibility, driven like a user would (accessibility tree,
//! keys) without a GPU.

use artcraft_toolbox_engine::{Session, Target};
use artcraft_toolbox_ui_egui::i18n::{self, Lang};
use artcraft_toolbox_ui_egui::theme::Tokens;
use artcraft_toolbox_ui_egui::{ToolboxApp, wording};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

/// PhotoCraft's releases known, nothing installed, drawn in `language`.
fn app(language: &str) -> ToolboxApp {
    let mut s = Session::new().unwrap();
    s.set_host(Target::from_consts("linux", "x86_64"));
    s.ingest_releases("photocraft", PHOTOCRAFT, artcraft_toolbox_engine::time::now_unix()).unwrap();
    s.execute("settings.set", json!({ "language": language })).unwrap();
    ToolboxApp::new(s)
}

fn harness(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
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

fn set(h: &mut Harness<'static, ToolboxApp>, key: &str, value: serde_json::Value) {
    let mut params = json!({});
    params[key] = value;
    h.state_mut().run("settings.set", params).expect("settings.set");
    h.run();
}

#[test]
fn the_language_setting_relabels_the_ui_while_it_runs() {
    let mut h = harness(app("de"));
    h.get_by_label("Nach Updates suchen");
    h.get_by_label("Einstellungen");
    h.get_by_label("PhotoCraft, 0.5.0 verfügbar");
    set(&mut h, "language", json!("ja"));
    h.get_by_label("アップデートを確認");
    h.get_by_label("PhotoCraft, 0.5.0 入手可能");
    set(&mut h, "language", json!("el"));
    h.get_by_label("Έλεγχος για ενημερώσεις");
    set(&mut h, "language", json!("en"));
    h.get_by_label("Check for updates");
    assert!(h.query_by_label("Nach Updates suchen").is_none());
}

#[test]
fn the_tray_and_notifications_follow_the_language() {
    let el = Lang::from_code("el").unwrap();
    assert_eq!(i18n::with_language(el, wording::tray_labels), ["Άνοιγμα του ArtCraft Toolbox", "Έλεγχος για ενημερώσεις", "Έξοδος από το ArtCraft Toolbox"]);
    assert_eq!(i18n::with_language(Lang::EN, wording::tray_labels), ["Open ArtCraft Toolbox", "Check for Updates", "Quit ArtCraft Toolbox"]);
    assert_eq!(i18n::tr(Lang::from_code("ru").unwrap(), "Update available"), "Доступно обновление");
}

#[test]
fn the_theme_follows_the_setting() {
    let mut h = harness(app("en"));
    assert_eq!(h.ctx.options(|o| o.theme_preference), egui::ThemePreference::System, "the system's by default");
    set(&mut h, "theme", json!("light"));
    h.run();
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
    assert_eq!(Tokens::get(&h.ctx), Tokens::LIGHT);
    set(&mut h, "theme", json!("dark"));
    h.run();
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
    assert_eq!(Tokens::get(&h.ctx), Tokens::DARK);
}

#[test]
fn text_size_follows_the_setting_and_its_shortcuts() {
    let mut h = harness(app("en"));
    set(&mut h, "textSize", json!(125));
    h.run();
    assert_eq!(h.ctx.zoom_factor(), 1.25);
    let step = |h: &mut Harness<'static, ToolboxApp>, key| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, key);
        h.run();
        h.run();
        (h.state().session.settings().text_size, h.ctx.zoom_factor())
    };
    assert_eq!(step(&mut h, egui::Key::Plus), (150, 1.5));
    assert_eq!(step(&mut h, egui::Key::Plus), (150, 1.5), "the largest size stays");
    assert_eq!(step(&mut h, egui::Key::Minus), (125, 1.25));
    assert_eq!(step(&mut h, egui::Key::Num0), (100, 1.0));
}

#[test]
fn a_row_speaks_its_status_and_opens_from_the_keyboard() {
    let mut h = harness(app("en"));
    let row = "PhotoCraft, 0.5.0 available";
    h.get_by_label(row);
    // Tab through the controls until the row has the focus, then Enter opens its page.
    let mut focused = false;
    for _ in 0..40 {
        h.key_press(egui::Key::Tab);
        h.run();
        if h.get_by_label(row).is_focused() {
            focused = true;
            break;
        }
    }
    assert!(focused, "Tab never reached the PhotoCraft row");
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().ui.selected.as_deref(), Some("photocraft"));
}
