//! M6: the control channel drives the window. Every UI state is readable (`ui.get`) and
//! settable (`ui.set`, validated as a whole before anything changes), engine commands and jobs
//! run by id, a waiting request is answered when its job ends, a stale request is refused.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use artcraft_toolbox_engine::net::{NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Session, Target};
use artcraft_toolbox_ui_egui::control::{METHODS, UI_SET_FIELDS};
use artcraft_toolbox_ui_egui::state::Tab;
use artcraft_toolbox_ui_egui::{ControlRequest, ControlResponse, ToolboxApp};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

/// PhotoCraft's real feed; an empty feed for every other app.
struct Fake;

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = if req.url.contains("/photocraft/") { PHOTOCRAFT } else { "[]" };
        Ok(Response::Ok { body: body.as_bytes().to_vec(), etag: None, rate: RateLimit::default() })
    }
}

struct Control {
    h: Harness<'static, ToolboxApp>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
}

impl Control {
    fn new() -> Control {
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), None, Some(Arc::new(Fake)));
        s.set_host(Target::from_consts("linux", "x86_64"));
        // No automatic check: the tests say when the network is used.
        s.execute("settings.set", json!({ "language": "en", "checkIntervalHours": 0 })).unwrap();
        let (tx, rx) = channel();
        let app = ToolboxApp::new(s).with_control(rx);
        let mut h = Harness::builder().with_size(egui::vec2(440.0, 1400.0)).build_ui_state(
            |ui, app: &mut ToolboxApp| {
                ToolboxApp::setup_context(ui.ctx());
                app.tick(ui.ctx());
                app.show(ui);
            },
            app,
        );
        h.run();
        Control { h, tx }
    }

    /// Send a request and run frames until its reply arrives (a job may take a moment).
    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, rx) = ControlRequest::new(method, params);
        self.send(req, rx)
    }

    fn send(&mut self, req: ControlRequest, rx: Receiver<ControlResponse>) -> Value {
        self.tx.send(req).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.h.step();
            if let Ok(v) = rx.try_recv() {
                return v;
            }
            assert!(Instant::now() < deadline, "no reply to the control request within 20 s");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params);
        assert_eq!(v["ok"], true, "{method}: {v}");
        v["result"].clone()
    }

    fn err(&mut self, method: &str, params: Value) -> String {
        let v = self.call(method, params);
        assert_eq!(v["ok"], false, "{method} succeeded: {v}");
        v["error"].as_str().unwrap_or_default().to_string()
    }
}

#[test]
fn every_ui_state_is_readable_and_settable() {
    let mut c = Control::new();
    let methods = c.ok("methods", json!({}));
    assert_eq!(methods, json!(METHODS));
    let state = c.ok("ui.get", json!({}));
    assert_eq!(state["ui"]["tab"], "apps");
    assert_eq!(state["ui"]["search"], "");
    assert_eq!(state["ui"]["selected"], Value::Null);
    assert_eq!(state["notice"], Value::Null);
    assert_eq!(state["language"], "en");
    assert_eq!(state["window"]["width"], 440.0);
    assert_eq!(state["services"]["tray"], false);
    assert_eq!(state["jobs"], json!([]));
    // Every field `ui.set` accepts is reported by `ui.get`.
    for field in UI_SET_FIELDS {
        assert!(state["ui"].get(field).is_some() || state.get(field).is_some(), "`{field}` is not readable: {state}");
    }
    // Set several at once; the window follows.
    let after = c.ok("ui.set", json!({"tab": "settings", "search": "vector", "searchOpen": true, "availableFolded": true, "notice": "Hello from a test"}));
    assert_eq!(after["ui"]["tab"], "settings");
    assert_eq!(c.h.state().ui.tab, Tab::Settings);
    assert_eq!(c.h.state().ui.search, "vector");
    assert!(c.h.state().ui.search_open && c.h.state().ui.available_folded);
    c.h.get_by_label("Hello from a test");
    // The page of an app, then back to the list; the banner dismissed with null.
    c.ok("ui.set", json!({"tab": "apps", "search": "", "searchOpen": false, "availableFolded": false, "selected": "photocraft", "notice": null}));
    assert_eq!(c.h.state().ui.selected.as_deref(), Some("photocraft"));
    assert!(c.h.state().notice.is_none());
    c.h.get_by_label("Back");
    c.ok("ui.set", json!({"selected": null}));
    c.h.get_by_label("VectorCraft");
    c.ok("ui.set", json!({"confirmUninstall": "photocraft"}));
    assert_eq!(c.h.state().ui.confirm_uninstall.as_deref(), Some("photocraft"));
    c.ok("ui.set", json!({"confirmUninstall": null}));
    assert!(c.h.state().ui.confirm_uninstall.is_none());
}

#[test]
fn ui_set_is_validated_as_a_whole_before_anything_changes() {
    let mut c = Control::new();
    c.ok("ui.set", json!({"search": "photo"}));
    // One bad field among good ones: nothing changes.
    let e = c.err("ui.set", json!({"search": "vector", "tab": "nope"}));
    assert!(e.contains("`tab`"), "{e}");
    assert_eq!(c.h.state().ui.search, "photo");
    let e = c.err("ui.set", json!({"search": "vector", "typo": 1}));
    assert!(e.contains("unknown field `typo`") && e.contains("tab, search"), "{e}");
    assert_eq!(c.h.state().ui.search, "photo");
    let e = c.err("ui.set", json!({"selected": "nope"}));
    assert!(e.contains("unknown app"), "{e}");
    let e = c.err("ui.set", json!({"selected": 7}));
    assert!(e.contains("app id or null"), "{e}");
    let e = c.err("ui.set", json!({"searchOpen": "yes"}));
    assert!(e.contains("true or false"), "{e}");
    let e = c.err("ui.set", json!({"search": "a".repeat(201)}));
    assert!(e.contains("200"), "{e}");
    let e = c.err("ui.set", json!({"search": "a\u{7}b"}));
    assert!(e.contains("control characters"), "{e}");
    let e = c.err("ui.set", json!({"notice": "n".repeat(1001)}));
    assert!(e.contains("1000"), "{e}");
    let e = c.err("ui.set", json!([1]));
    assert!(e.contains("JSON object"), "{e}");
    let e = c.err("ui.set", json!({"search": "x", "notice": 5}));
    assert!(e.contains("`notice`"), "{e}");
    assert_eq!(c.h.state().ui.search, "photo");
    let e = c.err("nope.nope", json!({}));
    assert!(e.contains("unknown method") && e.contains("methods"), "{e}");
}

#[test]
fn engine_commands_and_jobs_run_by_id() {
    let mut c = Control::new();
    let settings = c.ok("engine.execute", json!({"command": "settings.get"}));
    assert_eq!(settings["channel"], "stable");
    let e = c.err("engine.execute", json!({}));
    assert!(e.contains("`command`"), "{e}");
    let e = c.err("engine.execute", json!({"command": "settings.set", "params": [1]}));
    assert!(e.contains("JSON object"), "{e}");
    let e = c.err("engine.execute", json!({"command": "app.status", "params": {"app": "nope"}}));
    assert!(!e.is_empty());
    let list = c.ok("engine.commands", json!({"filter": "toolbox"}));
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|cmd| cmd["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"toolbox.status") && ids.contains(&"toolbox.update") && !ids.contains(&"settings.get"), "{ids:?}");
    let update = list.as_array().unwrap().iter().find(|cmd| cmd["id"] == "toolbox.update").unwrap();
    assert_eq!((update["background"].as_bool(), update["enabled"].as_bool()), (Some(true), Some(false)));
    assert!(update["disabledReason"].is_string());
    // A background command: the reply waits for the job, and carries its result.
    let before = Instant::now();
    let checked = c.ok("engine.execute", json!({"command": "updates.check", "params": {"app": "photocraft"}}));
    assert_eq!(checked["fetched"], json!(["photocraft"]), "{checked}");
    assert!(before.elapsed() < Duration::from_secs(20));
    let status = c.ok("engine.execute", json!({"command": "app.status", "params": {"app": "photocraft"}}));
    assert_eq!(status["status"]["latest"], "0.5.0");
    assert_eq!(c.ok("jobs.list", json!({}))["jobs"], json!([]));
    // Without waiting: a job id comes back at once; cancelling what isn't running is an error.
    let started = c.ok("engine.execute", json!({"command": "updates.check", "params": {"force": true}, "wait": false}));
    assert_eq!(started["pending"], true, "{started}");
    let job = started["job"].as_u64().unwrap();
    let cancelled = c.ok("jobs.cancel", json!({}));
    assert!(cancelled["cancelled"].as_u64().unwrap() <= 1);
    let e = c.err("jobs.cancel", json!({"job": job + 1000}));
    assert!(e.contains("no running job"), "{e}");
    let e = c.err("jobs.cancel", json!({"job": "seven"}));
    assert!(e.contains("job id"), "{e}");
    // A request that waited too long in the queue is refused, and nothing ran.
    let (mut req, rx) = ControlRequest::new("engine.execute", json!({"command": "settings.set", "params": {"channel": "prerelease"}}));
    req.deadline = Some(Instant::now() - Duration::from_secs(1));
    let v = c.send(req, rx);
    assert_eq!((v["ok"].as_bool(), v["error"].as_str()), (Some(false), Some("timeout")));
    assert_eq!(c.ok("engine.execute", json!({"command": "settings.get"}))["channel"], "stable");
    // Quit.
    assert_eq!(c.ok("app.quit", json!({}))["quitting"], true);
    assert!(c.h.state().quitting);
}
