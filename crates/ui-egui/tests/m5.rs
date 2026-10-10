//! The toolbox updating itself (M5): the offer at the top of the app list, the download, the
//! restart request, and the automatic download after a check; driven like a user would
//! (accessibility tree, clicks) without a GPU.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, SelfInstall, Session, Target, Version};
use artcraft_toolbox_release::PackageKind;
use artcraft_toolbox_store::Store;
use artcraft_toolbox_ui_egui::state::Tab;
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sha2::Digest;

const FILE: &str = "artcraft-toolbox-0.2.0-linux-x86_64.AppImage";
const URL: &str = "https://github.com/smyrgeorge/artcraft-toolbox/releases/download/v0.2.0/artcraft-toolbox-0.2.0-linux-x86_64.AppImage";
const SUMS: &str = "https://github.com/smyrgeorge/artcraft-toolbox/releases/download/v0.2.0/SHA256SUMS.txt";

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-ui-m5-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn appimage(tag: &[u8]) -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend((0..60_000u32).map(|i| (i % 251) as u8));
    b.extend_from_slice(tag);
    b
}

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A GitHub with ArtCraft Toolbox 0.2.0 for Linux (its feed, checksums and AppImage) and nothing
/// for the apps.
struct Fake;

fn feed() -> String {
    serde_json::json!([{
        "tag_name": "v0.2.0",
        "assets": [
            {"name": FILE, "size": appimage(b"0.2.0").len(), "browser_download_url": URL},
            {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": SUMS},
        ]
    }])
    .to_string()
}

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = if req.url == SUMS {
            format!("{}  {FILE}\n", sha(&appimage(b"0.2.0")))
        } else if req.url.contains("/artcraft-toolbox/releases") {
            feed()
        } else {
            "[]".into()
        };
        Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, url: &str, from: u64) -> Result<Download, NetError> {
        assert_eq!(url, URL);
        let body = appimage(b"0.2.0");
        let rest = body.get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(body.len() as u64) })
    }
}

/// A session running "ArtCraft Toolbox `running`" from an AppImage in `root`, with 0.2.0
/// published; `fetched`: its feed already known.
fn session(root: &std::path::Path, running: Version, fetched: bool) -> Session {
    let store = Store::open(root.join("data")).unwrap();
    let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(store), Some(Arc::new(Fake)));
    s.set_host(Target::from_consts("linux", "x86_64"));
    let current = root.join("bin").join("artcraft-toolbox.AppImage");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    std::fs::write(&current, appimage(b"0.1.0")).unwrap();
    s.set_self_install(Some(SelfInstall { kind: PackageKind::AppImage, path: current, version: running }));
    if fetched {
        s.ingest_releases("artcraft-toolbox", &feed(), 1).unwrap();
    }
    // No automatic check: the tests say when the network is used.
    s.execute("settings.set", serde_json::json!({ "language": "en", "checkIntervalHours": 0 })).unwrap();
    s
}

fn harness(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    Harness::builder().with_size(egui::vec2(440.0, 1400.0)).build_ui_state(
        |ui, app: &mut ToolboxApp| {
            ToolboxApp::setup_context(ui.ctx());
            app.tick(ui.ctx());
            app.show(ui);
        },
        app,
    )
}

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
fn the_offer_downloads_the_update_and_asks_for_a_restart() {
    let root = temp("offer");
    let mut h = harness(ToolboxApp::new(session(&root, Version::new(0, 1, 0), true)));
    h.run();
    h.get_by_label("0.2.0 is available");
    h.get_by_label("Update").click();
    wait_for_jobs(&mut h);
    h.get_by_label("0.2.0 is downloaded · restart to update");
    assert_eq!(h.state().session.staged_update().map(|u| u.version.clone()), Some(Version::new(0, 2, 0)));
    assert!(root.join("data/self-update/artcraft-toolbox/0.2.0/artcraft-toolbox.AppImage").is_file());
    // The About card says the same.
    h.state_mut().ui.tab = Tab::Settings;
    h.run();
    h.get_by_label("0.2.0 is downloaded · restart to update");
    assert!(!h.state().restart_wanted);
    h.get_by_label("Restart to update").click();
    h.run();
    assert!(h.state().restart_wanted, "the desktop app swaps the version in and restarts");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn up_to_date_and_not_packaged_copies_say_so() {
    let root = temp("uptodate");
    let mut app = ToolboxApp::new(session(&root, Version::new(0, 2, 0), true));
    app.ui.tab = Tab::Settings;
    let mut h = harness(app);
    h.run();
    h.get_by_label("Up to date");
    assert!(h.query_by_label("Restart to update").is_none() && h.query_by_label("Update").is_none());
    // A development build: the offer explains instead of offering a button.
    let mut s = session(&root, Version::new(0, 1, 0), true);
    s.set_self_install(None);
    let mut h = harness(ToolboxApp::new(s));
    h.run();
    let _ = h.get_by_label("0.2.0 is available");
    h.get(egui_kittest::kittest::by().label_contains("This copy can't update itself"));
    assert!(h.query_by_label("Update").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn automatic_updates_download_the_toolbox_too_and_announce_it_ready() {
    let root = temp("auto");
    let mut s = session(&root, Version::new(0, 1, 0), false);
    s.execute("settings.set", serde_json::json!({ "autoUpdate": true, "checkIntervalHours": 6 })).unwrap();
    let sent: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let log = Arc::clone(&sent);
    let services = Services {
        notify: Some(Box::new(move |title, body| log.lock().unwrap().push((title.into(), body.into())))),
        tray: false,
        popover: false,
        transparent: false,
    };
    // The first tick starts the due check by itself; the check finds the toolbox's update and,
    // with automatic updates on, downloads it quietly; the notification comes when it is ready.
    let mut h = harness(ToolboxApp::with_services(s, services));
    wait_for_jobs(&mut h);
    wait_for_jobs(&mut h);
    assert_eq!(*sent.lock().unwrap(), [("Update ready".to_string(), "ArtCraft Toolbox 0.2.0 will be used after a restart".to_string())]);
    assert_eq!(h.state().session.staged_update().map(|u| u.version.clone()), Some(Version::new(0, 2, 0)));
    h.get_by_label("Restart to update");
    let _ = std::fs::remove_dir_all(&root);
}
