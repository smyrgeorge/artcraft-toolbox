//! The window's own controls, driven like a user would (accessibility tree, keys) without a GPU:
//! the search button and its keys, folding the available apps, an installed app's menu, the
//! error banner, the settings switches.

use std::path::PathBuf;

use artcraft_toolbox_engine::{Layout, Session, Target};
use artcraft_toolbox_model::{Installation, Inventory};
use artcraft_toolbox_release::{PackageKind, Version};
use artcraft_toolbox_ui_egui::ToolboxApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-ui-look-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// PhotoCraft's releases known; `installed`: its 0.3.0 recorded as installed under `root`.
fn app(root: &std::path::Path, installed: bool) -> ToolboxApp {
    let mut s = Session::new().unwrap();
    s.set_host(Target::from_consts("linux", "x86_64"));
    s.ingest_releases("photocraft", PHOTOCRAFT, artcraft_toolbox_engine::time::now_unix()).unwrap();
    s.set_layout(Some(Layout {
        apps: root.join("apps"),
        desktop_entries: None,
        icons: None,
        start_menu: None,
        downloads: root.join("downloads"),
        kept: root.join("kept"),
    }));
    if installed {
        let mut inventory = Inventory::default();
        inventory
            .record(Installation {
                app: "photocraft".into(),
                version: Version::new(0, 3, 0),
                kind: PackageKind::AppImage,
                path: root.join("apps/photocraft/0.3.0/photocraft.AppImage").display().to_string(),
                installed_at: 1,
                active: true,
                trust: None,
            })
            .unwrap();
        s.set_inventory(inventory);
    }
    s.execute("settings.set", json!({ "language": "en" })).unwrap();
    ToolboxApp::new(s)
}

fn harness(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    let mut h = Harness::builder().with_size(egui::vec2(440.0, 1600.0)).build_ui_state(
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
fn search_opens_from_its_button_and_its_keys() {
    let root = temp("search");
    let mut h = harness(app(&root, false));
    h.get_by_label("Search apps").click();
    h.run();
    assert!(h.state().ui.search_open);
    h.get_by_label("Close search");
    h.state_mut().ui.search = "vector".into();
    h.run();
    assert!(h.query_by_label("PhotoCraft").is_none());
    h.get_by_label("VectorCraft");
    // Escape closes it and shows every app again; Cmd/Ctrl+F opens it from anywhere.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().ui.search_open && h.state().ui.search.is_empty());
    h.get_by_label("PhotoCraft");
    h.get_by_label("Settings").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run();
    assert!(h.state().ui.search_open);
    assert_eq!(h.state().ui.tab, artcraft_toolbox_ui_egui::state::Tab::Apps);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_available_apps_fold_away() {
    let root = temp("fold");
    let mut h = harness(app(&root, true));
    h.get_by_label("VectorCraft");
    h.get_by_label("Available apps").click();
    h.run();
    assert!(h.state().ui.available_folded);
    assert!(h.query_by_label("VectorCraft").is_none());
    h.get_by_label("PhotoCraft");
    // A search shows its matches even while folded.
    h.state_mut().ui.search = "vector".into();
    h.run();
    h.get_by_label("VectorCraft");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_installed_app_menu_offers_details_and_uninstall() {
    let root = temp("menu");
    let mut h = harness(app(&root, true));
    h.get_by_label("More actions for PhotoCraft").click();
    h.run();
    h.get_by_label("Details").click();
    h.run();
    assert_eq!(h.state().ui.selected.as_deref(), Some("photocraft"));
    h.get_by_label("Back").click();
    h.run();
    h.get_by_label("More actions for PhotoCraft").click();
    h.run();
    h.get_by_label("Uninstall…").click();
    h.run();
    // The question comes from the list too; Escape keeps the app.
    h.get_by_label("Uninstall PhotoCraft 0.3.0?");
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().ui.confirm_uninstall, None);
    assert!(h.state().session.inventory().current("photocraft").is_some());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_banner_shows_an_error_until_dismissed() {
    let root = temp("banner");
    let mut a = app(&root, false);
    a.notice = Some("Something went wrong".into());
    let mut h = harness(a);
    h.get_by_label("Something went wrong");
    h.get_by_label("Dismiss").click();
    h.run();
    assert_eq!(h.state().notice, None);
    assert!(h.query_by_label("Something went wrong").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn settings_switches_change_the_settings() {
    let root = temp("switches");
    let mut h = harness(app(&root, false));
    h.get_by_label("Settings").click();
    h.run();
    assert!(!h.state().session.settings().auto_update);
    h.get_by_label("Install updates automatically").click();
    h.run();
    assert!(h.state().session.settings().auto_update);
    // Without a notification service, its switch does nothing.
    let before = h.state().session.settings().notifications;
    h.get_by_label("Notify me when updates are found").click();
    h.run();
    assert_eq!(h.state().session.settings().notifications, before);
    let _ = std::fs::remove_dir_all(&root);
}
