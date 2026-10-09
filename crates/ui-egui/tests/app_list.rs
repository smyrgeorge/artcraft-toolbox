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
    for _ in 0..400 {
        h.step();
        if !h.state().session.has_jobs() {
            break;
        }
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
        ]
        .map(String::from),
    );
    texts.push(artcraft_toolbox_engine::update_cmds::rate_limit_text(0, 1200));
    for t in &texts {
        let missing: Vec<char> = t.chars().filter(|c| !h.ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(13.0), *c))).collect();
        assert!(missing.is_empty(), "{t:?} has no glyph for {missing:?}");
    }
}
