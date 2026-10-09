//! Installing and uninstalling from the UI, against a fake GitHub and temp folders.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Layout, Session, Target};
use artcraft_toolbox_ui_egui::ToolboxApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sha2::Digest;

const URL: &str = "https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-linux-x86_64.AppImage";
const SUMS: &str = "https://github.com/storytold/photocraft/releases/download/v0.5.0/SHA256SUMS.txt";

fn appimage() -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend((0..100_000u32).map(|i| (i % 253) as u8));
    b
}

struct Fake;

impl Transport for Fake {
    fn get(&self, _: &Request<'_>) -> Result<Response, NetError> {
        let hex: String = sha2::Sha256::digest(appimage()).iter().map(|b| format!("{b:02x}")).collect();
        Ok(Response::Ok { body: format!("{hex}  photocraft-0.5.0-linux-x86_64.AppImage\n").into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, _: &str, from: u64) -> Result<Download, NetError> {
        let rest = appimage().get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(appimage().len() as u64) })
    }
}

fn temp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-ui-m2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn session(root: &Path) -> Session {
    let (mut s, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Fake)));
    s.set_host(Target::from_consts("linux", "x86_64"));
    s.set_layout(Some(Layout {
        apps: root.join("apps"),
        desktop_entries: None,
        icons: None,
        start_menu: None,
        downloads: root.join("downloads"),
        kept: root.join("kept"),
    }));
    let feed = serde_json::json!([{
        "tag_name": "v0.5.0",
        "assets": [
            {"name": "photocraft-0.5.0-linux-x86_64.AppImage", "size": appimage().len(), "browser_download_url": URL},
            {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": SUMS},
        ]
    }]);
    s.ingest_releases("photocraft", &feed.to_string(), artcraft_toolbox_engine::time::now_unix()).unwrap();
    s
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
    h.run();
}

#[test]
fn install_from_the_list_then_uninstall_from_the_page() {
    let root = temp();
    let mut h = harness(ToolboxApp::new(session(&root)));
    // PhotoCraft's row comes first; the others' Install buttons are disabled (not checked).
    h.get_all_by_label("Install").next().unwrap().click();
    wait_for_jobs(&mut h);
    assert_eq!(h.state().notice, None);
    let path = root.join("apps/photocraft/0.5.0/photocraft.AppImage");
    assert!(path.is_file());
    h.get_by_label("0.5.0 · up to date");
    h.get_by_label("Open");
    assert_eq!(h.state().ui.selected, None, "clicking the action doesn't open the page");

    h.get_by_label("PhotoCraft").click();
    h.run();
    h.get_by_label("Uninstall").click();
    h.run();
    h.get_by_label("Uninstall PhotoCraft 0.5.0?");
    // Cancel first: nothing happens.
    h.get_by_label("Cancel").click();
    h.run();
    assert!(path.is_file());
    // Escape is a Cancel too.
    h.get_by_label("Uninstall").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().ui.confirm_uninstall, None);
    assert!(h.query_by_label("Uninstall PhotoCraft 0.5.0?").is_none());
    assert!(path.is_file());
    h.get_by_label("Uninstall").click();
    h.run();
    h.get_all_by_label("Uninstall").last().unwrap().click();
    h.run();
    assert!(!path.exists());
    assert!(h.state().session.inventory().current("photocraft").is_none());
    assert_eq!(h.state().notice, None);
    let _ = std::fs::remove_dir_all(&root);
}
