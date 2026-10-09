//! The details page, per-app settings, notifications and close-to-tray, driven like a user would
//! (accessibility tree, clicks) without a GPU.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use artcraft_toolbox_engine::net::{NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Session, Target, Version};
use artcraft_toolbox_model::{Installation, Inventory, PackageKind};
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

fn session() -> Session {
    let mut s = Session::new().unwrap();
    s.set_host(Target::from_consts("linux", "x86_64"));
    s.ingest_releases("photocraft", PHOTOCRAFT, 1).unwrap();
    s
}

/// The UI in English whatever this computer's language (the tests read English labels).
fn english(mut app: ToolboxApp) -> ToolboxApp {
    app.session.execute("settings.set", serde_json::json!({ "language": "en" })).unwrap();
    app
}

/// Draws with `tick` too, as eframe's `logic` does in the app.
fn harness(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    let mut h = unsettled(app);
    h.run();
    h
}

/// The harness before its first frames: for a test whose first tick starts background work,
/// which keeps the UI repainting until it ends (`Harness::run` gives up after a few frames, and
/// how many the work takes depends on the machine).
fn unsettled(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    let app = english(app);
    Harness::builder().with_size(egui::vec2(440.0, 1400.0)).build_ui_state(
        |ui, app: &mut ToolboxApp| {
            ToolboxApp::setup_context(ui.ctx());
            app.tick(ui.ctx());
            app.show(ui);
        },
        app,
    )
}

#[test]
fn a_row_opens_the_details_page_with_release_notes() {
    let mut h = harness(ToolboxApp::new(session()));
    h.get_by_label("PhotoCraft").click();
    h.run();
    assert_eq!(h.state().ui.selected.as_deref(), Some("photocraft"));
    h.get_by_label("Settings for PhotoCraft");
    h.get_by_label("0.5.0 · 2026-10-08");
    // The newest version is open; its notes are rendered.
    h.get_by_label("Release notes trimmed for this test fixture.");
    h.get_by_label("Open on GitHub");
    h.get_by_label("Back").click();
    h.run();
    assert_eq!(h.state().ui.selected, None);
    h.get_by_label("VectorCraft");
}

#[test]
fn pinning_a_version_changes_what_is_offered() {
    let mut app = ToolboxApp::new(session());
    app.ui.selected = Some("photocraft".into());
    let mut h = harness(app);
    // A combo box shows its choice as its value, not its label.
    h.get(egui_kittest::kittest::by().value("Latest")).click();
    h.run();
    h.get_by_label("Pin to 0.3.0").click();
    h.run();
    assert_eq!(h.state().session.settings().pinned("photocraft"), Some(&Version::new(0, 3, 0)));
    h.get(egui_kittest::kittest::by().value("Pinned to 0.3.0"));
    h.get_by_label("0.3.0 · 2026-10-07 · pinned");
    // Back on the list, the row says so.
    h.get_by_label("Back").click();
    h.run();
    h.get_by_label("PhotoCraft, 0.3.0 available · pinned to 0.3.0");
    h.get_by_label("0.3.0 · pinned");
}

/// PhotoCraft's real feed for every request.
struct Feed;

impl Transport for Feed {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = if req.url.contains("/photocraft/") { PHOTOCRAFT } else { "[]" };
        Ok(Response::Ok { body: body.as_bytes().to_vec(), etag: None, rate: RateLimit::default() })
    }
}

/// Draw frames until the background jobs are done (by a deadline, not a frame count: CI machines
/// differ).
fn wait_for_jobs(h: &mut Harness<'static, ToolboxApp>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().session.has_jobs() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "background jobs still running after 20 s");
        std::thread::sleep(Duration::from_millis(5));
    }
    h.step();
}

#[test]
fn a_background_check_notifies_once_per_new_version() {
    let (mut s, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Feed)));
    s.set_host(Target::from_consts("linux", "x86_64"));
    let mut inv = Inventory::default();
    inv.record(Installation {
        app: "photocraft".into(),
        version: Version::new(0, 3, 0),
        kind: PackageKind::AppImage,
        path: "/x".into(),
        installed_at: 0,
        active: true,
        trust: None,
    })
    .unwrap();
    s.set_inventory(inv);
    let sent: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let log = Arc::clone(&sent);
    let services = Services {
        notify: Some(Box::new(move |title, body| log.lock().unwrap().push((title.into(), body.into())))),
        tray: false,
        popover: false,
        transparent: false,
    };
    // The first tick starts the due check by itself (never checked), as the app does at start.
    let mut h = unsettled(ToolboxApp::with_services(s, services));
    wait_for_jobs(&mut h);
    assert_eq!(*sent.lock().unwrap(), [("Update available".to_string(), "PhotoCraft 0.5.0".to_string())]);
    // A manual check finds the same version: no second notification.
    h.state_mut().session.set_clock(|| artcraft_toolbox_engine::time::now_unix() + 3600);
    h.get_by_label("Check for updates").click();
    wait_for_jobs(&mut h);
    assert_eq!(sent.lock().unwrap().len(), 1);
}

fn request_close(h: &mut Harness<'static, ToolboxApp>) -> Vec<egui::ViewportCommand> {
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
}

#[test]
fn closing_the_window_hides_it_while_there_is_a_tray() {
    let with_tray = || ToolboxApp::with_services(Session::new().unwrap(), Services { notify: None, tray: true, popover: false, transparent: false });
    let mut h = harness(with_tray());
    let cmds = request_close(&mut h);
    assert!(cmds.contains(&egui::ViewportCommand::CancelClose) && cmds.contains(&egui::ViewportCommand::Visible(false)), "{cmds:?}");

    // Quit from the tray menu really closes.
    let mut app = with_tray();
    app.quitting = true;
    let mut h = harness(app);
    assert!(!request_close(&mut h).contains(&egui::ViewportCommand::CancelClose));

    // So does turning the setting off, and having no tray at all.
    let mut h = harness(with_tray());
    h.state_mut().run("settings.set", serde_json::json!({"closeToTray": false}));
    assert!(!request_close(&mut h).contains(&egui::ViewportCommand::CancelClose));
    let mut h = harness(ToolboxApp::new(Session::new().unwrap()));
    assert!(!request_close(&mut h).contains(&egui::ViewportCommand::CancelClose));
}
