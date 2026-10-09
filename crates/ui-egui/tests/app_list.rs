//! The UI drives the engine: rows come from the session, and controls dispatch commands.
//! Runs without a GPU (no rendering, accessibility tree only).

use std::sync::Arc;
use std::time::Duration;

use artcraft_toolbox_engine::net::{NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Session, Target};
use artcraft_toolbox_model::Channel;
use artcraft_toolbox_ui_egui::ToolboxApp;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn harness() -> Harness<'static, ToolboxApp> {
    harness_with(Session::new().unwrap())
}

fn harness_with(session: Session) -> Harness<'static, ToolboxApp> {
    let app = ToolboxApp::new(session);
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

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

/// PhotoCraft's real feed, a timeout for VectorCraft, no releases for the rest.
struct Fake;

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = if req.url.contains("/photocraft/") {
            PHOTOCRAFT
        } else if req.url.contains("/vectorcraft/") {
            return Err(NetError::Timeout);
        } else {
            "[]"
        };
        Ok(Response::Ok { body: body.as_bytes().to_vec(), etag: None, rate: RateLimit::default() })
    }
}

#[test]
fn check_for_updates_fills_in_the_rows() {
    let (mut session, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Fake)));
    session.set_host(Target::from_consts("linux", "x86_64"));
    let mut h = harness_with(session);
    h.get_by_label("Not checked yet · 12 apps · linux-x86_64");
    h.get_by_label("Check for updates").click();
    // A frame first: the click only starts the check when the next frame handles it. Then frames
    // until it is done, by a deadline, not a frame count: CI machines differ.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().session.has_jobs() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the check still runs after 20 s");
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run();
    assert!(!h.state().session.has_jobs());
    h.get_by_label("0.5.0 available");
    h.get_by_label("Couldn't check: the server took too long to answer");
    h.get_by_label("Couldn't check VectorCraft: the server took too long to answer");
    assert_eq!(h.state().notice.as_deref(), Some("Couldn't check VectorCraft: the server took too long to answer"));
}

#[test]
fn an_offline_session_explains_why_it_cannot_check() {
    let h = harness();
    let button = h.get_by_label("Check for updates");
    assert!(button.accesskit_node().is_disabled());
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
    texts.extend(
        [
            "INSTALLED · 1",
            "No app matches “x”",
            "0.1.0 (dev build)",
            "Check for updates",
            "Checking 3 of 12",
            "Checked 5 min ago · 12 apps · macos-aarch64",
            "Not checked yet",
            "Couldn't check: the server took too long to answer",
            "Back",
            "GitHub",
            "Releases",
            "Website",
            "Open on GitHub",
            "SETTINGS FOR PHOTOCRAFT",
            "Default (Stable)",
            "Default (ask first)",
            "Pre-release",
            "Pin to 0.3.0",
            "Pinned to 0.3.0",
            "0.5.0 · 2026-10-08 · pre-release · installed · pinned · no build for this computer",
            "0.3.0 available · pinned to 0.3.0",
            "Not checked yet: check for updates to see versions and release notes.",
            "Notify me when updates are found",
            "Keep running in the menu bar when the window is closed",
            "Updates available",
            "PhotoCraft 0.5.0, VectorCraft 0.7.0 and 2 more",
        ]
        .map(String::from),
    );
    texts.push(artcraft_toolbox_engine::update_cmds::rate_limit_text(0, 1200));
    for t in &texts {
        let missing: Vec<char> = t.chars().filter(|c| !h.ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(13.0), *c))).collect();
        assert!(missing.is_empty(), "{t:?} has no glyph for {missing:?}");
    }
}
