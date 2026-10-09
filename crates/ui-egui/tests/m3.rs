//! Updating, switching versions, adopting and automatic updates from the UI, against a fake
//! GitHub (PhotoCraft 0.4.0 and 0.5.0, VectorCraft 0.6.0 and 0.7.0 for Linux) and temp folders.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Layout, Session, Target};
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use sha2::Digest;

const APPS: [(&str, [&str; 2]); 2] = [("photocraft", ["0.5.0", "0.4.0"]), ("vectorcraft", ["0.7.0", "0.6.0"])];

fn appimage(app: &str, v: &str) -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend(format!("{app} {v}").bytes().cycle().take(20_000));
    b
}

fn url(app: &str, v: &str, file: &str) -> String {
    format!("https://github.com/storytold/{app}/releases/download/v{v}/{file}")
}

fn releases(app: &str) -> String {
    let Some((_, versions)) = APPS.iter().find(|(a, _)| *a == app) else { return "[]".into() };
    let rels: Vec<_> = versions
        .iter()
        .map(|v| {
            let file = format!("{app}-{v}-linux-x86_64.AppImage");
            json!({"tag_name": format!("v{v}"), "assets": [
                {"name": file, "size": appimage(app, v).len(), "browser_download_url": url(app, v, &file)},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": url(app, v, "SHA256SUMS.txt")},
            ]})
        })
        .collect();
    json!(rels).to_string()
}

/// `(app, version)` from a release download URL.
fn parse(u: &str) -> Option<(String, String)> {
    let rest = u.strip_prefix("https://github.com/storytold/")?;
    let (app, rest) = rest.split_once("/releases/download/v")?;
    let (v, _) = rest.split_once('/')?;
    Some((app.to_string(), v.to_string()))
}

struct Fake;

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = match parse(req.url) {
            Some((app, v)) => {
                let hex: String = sha2::Sha256::digest(appimage(&app, &v)).iter().map(|b| format!("{b:02x}")).collect();
                format!("{hex}  {app}-{v}-linux-x86_64.AppImage\n")
            }
            None => APPS.iter().find(|(a, _)| req.url.contains(&format!("/storytold/{a}/"))).map_or("[]".to_string(), |(a, _)| releases(a)),
        };
        Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, u: &str, from: u64) -> Result<Download, NetError> {
        let (app, v) = parse(u).ok_or_else(|| NetError::Other(u.to_string()))?;
        let body = appimage(&app, &v);
        let rest = body.get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(body.len() as u64) })
    }
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-ui-m3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A session that installs into `root`, with both feeds known (`fetched`: as of now, so no
/// automatic check is due; else never checked).
fn session(root: &Path, fetched: bool) -> Session {
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
    if fetched {
        for (app, _) in APPS {
            s.ingest_releases(app, &releases(app), artcraft_toolbox_engine::time::now_unix()).unwrap();
        }
    }
    s
}

/// The UI in English whatever this computer's language (the tests read English labels).
fn english(mut app: ToolboxApp) -> ToolboxApp {
    app.session.execute("settings.set", serde_json::json!({ "language": "en" })).unwrap();
    app
}

/// Draws the UI only: no background work starts by itself (icons, checks), so a frame count
/// can't depend on the network's speed.
fn harness(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    let app = english(app);
    Harness::builder().with_size(egui::vec2(440.0, 2400.0)).build_ui_state(
        |ui, app: &mut ToolboxApp| {
            ToolboxApp::setup_context(ui.ctx());
            app.show(ui);
        },
        app,
    )
}

/// Draws with `tick` too, as eframe's `logic` does in the app: due checks start by themselves.
fn ticking(app: ToolboxApp) -> Harness<'static, ToolboxApp> {
    let app = english(app);
    Harness::builder().with_size(egui::vec2(440.0, 2400.0)).build_ui_state(
        |ui, app: &mut ToolboxApp| {
            ToolboxApp::setup_context(ui.ctx());
            app.tick(ui.ctx());
            app.show(ui);
        },
        app,
    )
}

/// Draw frames until the background jobs are done (by a deadline: CI machines differ).
fn settle(h: &mut Harness<'static, ToolboxApp>) {
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

fn active(h: &Harness<'static, ToolboxApp>, app: &str) -> String {
    h.state().session.inventory().current(app).map(|i| i.version.to_string()).unwrap_or_default()
}

#[test]
fn update_all_then_switch_versions_on_the_page() {
    let root = temp("update");
    let mut s = session(&root, true);
    s.execute("app.install", json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
    s.execute("app.install", json!({"app": "vectorcraft", "version": "0.6.0"})).unwrap();
    let mut h = harness(ToolboxApp::new(s));
    h.run();
    h.get_by_label("0.5.0 available · 0.4.0 installed");
    assert_eq!(h.get_all_by_label("Update").count(), 2);
    h.get_by_label("Update all (2)").click();
    settle(&mut h);
    assert_eq!(h.state().notice, None);
    assert_eq!((active(&h, "photocraft"), active(&h, "vectorcraft")), ("0.5.0".to_string(), "0.7.0".to_string()));
    h.get_by_label("0.5.0 · up to date");
    assert!(h.query_by_label("Update all (2)").is_none());

    // The page: the signature line, and the kept version to switch back to.
    h.get_by_label("PhotoCraft").click();
    h.run();
    h.get_by_label("No platform signature");
    h.get_by_label("0.5.0 · in use");
    h.get_by_label("0.4.0 · kept").click();
    h.run();
    h.get_by_label("Switch to this version").click();
    h.run();
    assert_eq!(active(&h, "photocraft"), "0.4.0");
    // Update now switches forward to the kept 0.5.0, without a download.
    h.get_by_label("Update").click();
    h.run();
    assert!(!h.state().session.has_jobs());
    assert_eq!(active(&h, "photocraft"), "0.5.0");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_app_installed_by_hand_is_offered_for_adoption() {
    let root = temp("adopt");
    let by_hand = root.join("apps/photocraft/0.3.0/photocraft.AppImage");
    std::fs::create_dir_all(by_hand.parent().unwrap()).unwrap();
    std::fs::write(&by_hand, appimage("photocraft", "0.3.0")).unwrap();
    let mut s = session(&root, true);
    s.rescan();
    let mut h = harness(ToolboxApp::new(s));
    h.run();
    h.get_by_label("Installed");
    h.get_by_label("0.3.0 installed outside the toolbox");
    h.get_by_label("Adopt").click();
    h.run();
    assert_eq!(h.state().notice, None);
    assert_eq!(active(&h, "photocraft"), "0.3.0");
    h.get_by_label("0.5.0 available · 0.3.0 installed");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn automatic_updates_follow_a_check_and_are_announced() {
    let root = temp("auto");
    let mut s = session(&root, true);
    s.execute("app.install", json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
    s.execute("settings.set", json!({"autoUpdate": true})).unwrap();
    // Forget the feeds: the first tick's check finds the update.
    let mut s2 = session(&root, false);
    s2.set_inventory(s.inventory().clone());
    s2.execute("settings.set", json!({"autoUpdate": true})).unwrap();
    let sent: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let log = Arc::clone(&sent);
    let services = Services {
        notify: Some(Box::new(move |title, body| log.lock().unwrap().push((title.into(), body.into())))),
        tray: false,
        popover: false,
        transparent: false,
    };
    let mut h = ticking(ToolboxApp::with_services(s2, services));
    // The check, then the update it starts.
    settle(&mut h);
    settle(&mut h);
    assert_eq!(active(&h, "photocraft"), "0.5.0");
    assert_eq!(*sent.lock().unwrap(), [("Updated".to_string(), "PhotoCraft 0.5.0".to_string())], "no 'available' note for an app that updates itself");
    let _ = std::fs::remove_dir_all(&root);
}
