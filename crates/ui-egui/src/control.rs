//! Programmatic control of the running window, for agents, tests and MCP (PhotoCraft's
//! `control.rs`, ported to the toolbox's much smaller UI).
//!
//! Transport-agnostic: a transport thread (TCP in the desktop app, `apps/artcraft-toolbox/src/
//! control_server.rs`; a channel in tests) sends [`ControlRequest`]s; the UI thread handles
//! them between frames ([`ToolboxApp::drain_control`]) and replies with JSON.
//!
//! Methods (`docs/control-protocol.md`):
//! - `engine.execute {command, params?, wait?}`: any engine command by id; a background command
//!   replies when it ends, or at once with `{job, pending: true}` when `wait` is false
//! - `engine.commands {filter?}`: the registry, with each command's enabled state
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs
//! - `ui.get`: the window's state ([`UiState`](crate::state::UiState), the banner, window size,
//!   services)
//! - `ui.set {tab?, search?, searchOpen?, availableFolded?, selected?, confirmUninstall?, notice?}`:
//!   change it; every field is validated before the first one is applied ([`UI_SET_FIELDS`])
//! - `app.quit`
//! - `methods`: this list

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use artcraft_toolbox_engine::{JobEvent, JobId, Started, command_specs};
use serde_json::{Value, json};

use crate::ToolboxApp;
use crate::state::{Tab, UiState};

pub type ControlResponse = Value;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
    /// Requests still queued at this instant are rejected; already dispatched work continues.
    pub deadline: Option<Instant>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, deadline: None }, rx)
    }
}

pub enum Outcome {
    Done(Value),
    /// A command started a background job and the caller waits for it: reply with its result
    /// once it has been applied (or with its error).
    AfterJob(JobId),
}

/// Method names the window answers.
pub const METHODS: &[&str] = &["engine.execute", "engine.commands", "jobs.list", "jobs.cancel", "ui.get", "ui.set", "app.quit", "methods"];

/// The fields `ui.set` reads. Anything else is rejected before a field is applied, and every
/// field's value is validated before the first one is applied, so a typo, an unknown field or a
/// bad value can't reply with success while nothing, or only half of it, changed.
pub const UI_SET_FIELDS: [&str; 7] = ["tab", "search", "searchOpen", "availableFolded", "selected", "confirmUninstall", "notice"];

/// Longest search text accepted.
const MAX_SEARCH: usize = 200;

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}

fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The command registry with each command's enabled state (the shape `engine.commands` and
/// the headless server share).
pub fn command_list(app: &ToolboxApp) -> Value {
    Value::Array(
        command_specs()
            .iter()
            .map(|c| {
                let reason = app.session.disabled_reason(c.id);
                json!({"id": c.id, "label": c.label, "params": c.params, "background": c.start.is_some(), "enabled": reason.is_none(), "disabledReason": reason})
            })
            .collect(),
    )
}

pub fn handle(app: &mut ToolboxApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = if req.params.is_null() { json!({}) } else { req.params.clone() };
    if !p.is_object() {
        return err(format!("params must be a JSON object, got {}", kind(&p)));
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    match req.method.as_str() {
        "engine.execute" => {
            let Some(id) = s("command") else { return err("engine.execute needs `command`") };
            let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
            let params = match p.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(other) => return err(format!("`{}`: params must be a JSON object, got {}", id.chars().take(80).collect::<String>(), kind(other))),
            };
            match app.session.start(id, params) {
                Ok(Started::Done(v)) => ok(v),
                Ok(Started::Job(job)) if wait => Outcome::AfterJob(job),
                Ok(Started::Job(job)) => ok(json!({"job": job.0, "pending": true})),
                Err(e) => err(e),
            }
        }
        "engine.commands" => {
            let all = command_list(app);
            let Some(needle) = s("filter").map(str::to_lowercase).filter(|n| !n.is_empty()) else { return ok(all) };
            ok(Value::Array(
                all.as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| format!("{} {}", c["id"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or("")).to_lowercase().contains(&needle))
                    .cloned()
                    .collect(),
            ))
        }
        "jobs.list" => ok(json!({"jobs": app.session.jobs()})),
        "jobs.cancel" => match p.get("job") {
            None | Some(Value::Null) => {
                let ids: Vec<JobId> = app.session.jobs().into_iter().map(|j| j.id).collect();
                for id in &ids {
                    app.session.cancel_job(*id);
                }
                ok(json!({"cancelled": ids.len()}))
            }
            Some(v) => match v.as_u64() {
                Some(id) if app.session.cancel_job(JobId(id)) => ok(json!({"cancelled": 1})),
                Some(id) => err(format!("no running job {id}")),
                None => err("`job` must be a job id"),
            },
        },
        "ui.get" => ok(inspect(app, ctx)),
        "ui.set" => match ui_set(app, &p) {
            Ok(()) => ok(inspect(app, ctx)),
            Err(e) => err(e),
        },
        "app.quit" => {
            app.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(json!({"quitting": true}))
        }
        "methods" => ok(json!(METHODS)),
        other => err(format!("unknown method `{}` (try `methods`)", other.chars().take(80).collect::<String>())),
    }
}

/// Validate every field of `p`, then apply them all; an error applies nothing.
fn ui_set(app: &mut ToolboxApp, p: &Value) -> Result<(), String> {
    let fields = p.as_object().ok_or("params must be a JSON object")?;
    if let Some(field) = fields.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str())) {
        return Err(format!("unknown field `{}` (fields: {})", field.chars().take(40).collect::<String>(), UI_SET_FIELDS.join(", ")));
    }
    let mut next: UiState = app.ui.clone();
    let mut notice: Option<Option<String>> = None;
    let app_id = |field: &str, v: &Value| -> Result<Option<String>, String> {
        match v {
            Value::Null => Ok(None),
            Value::String(id) if app.session.catalog().get(id).is_some() => Ok(Some(id.clone())),
            Value::String(id) => Err(format!("`{field}`: unknown app `{}`", id.chars().take(40).collect::<String>())),
            other => Err(format!("`{field}` must be an app id or null, got {}", kind(other))),
        }
    };
    let bool_field = |field: &str, v: &Value| v.as_bool().ok_or_else(|| format!("`{field}` must be true or false, got {}", kind(v)));
    for (k, v) in fields {
        match k.as_str() {
            "tab" => {
                next.tab = match v.as_str() {
                    Some("apps") => Tab::Apps,
                    Some("settings") => Tab::Settings,
                    _ => return Err("`tab` must be \"apps\" or \"settings\"".into()),
                }
            }
            "search" => {
                let text = v.as_str().ok_or_else(|| format!("`search` must be a string, got {}", kind(v)))?;
                if text.chars().count() > MAX_SEARCH || text.chars().any(char::is_control) {
                    return Err(format!("`search` must be at most {MAX_SEARCH} characters without control characters"));
                }
                next.search = text.to_string();
            }
            "searchOpen" => next.search_open = bool_field("searchOpen", v)?,
            "availableFolded" => next.available_folded = bool_field("availableFolded", v)?,
            "selected" => next.selected = app_id("selected", v)?,
            "confirmUninstall" => next.confirm_uninstall = app_id("confirmUninstall", v)?,
            "notice" => {
                notice = Some(match v {
                    Value::Null => None,
                    Value::String(text) if text.chars().count() <= 1000 => Some(text.clone()),
                    Value::String(_) => return Err("`notice` must be at most 1000 characters".into()),
                    other => return Err(format!("`notice` must be a string or null, got {}", kind(other))),
                })
            }
            _ => {}
        }
    }
    // The uninstall question only makes sense on an app the toolbox installed; the page it
    // opens on may be any app. An open search shows the field; a closed one keeps its text.
    app.ui = next;
    if let Some(n) = notice {
        app.notice = n;
    }
    Ok(())
}

/// Everything about the window an agent can see: the view state, the banner, the window's
/// size, what the platform provides, and the UI language.
pub fn inspect(app: &ToolboxApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "ui": app.ui,
        "notice": app.notice,
        "restartWanted": app.restart_wanted,
        "quitting": app.quitting,
        "services": {"notify": app.services.notify.is_some(), "tray": app.services.tray, "popover": app.services.popover},
        "language": crate::i18n::current().code(),
        "jobs": app.session.jobs(),
    })
}

impl ToolboxApp {
    /// Take control requests from `rx` between frames (the desktop app's control server).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Answer every queued control request. A request past its deadline is refused before
    /// dispatch; one that started a job it waits for is answered when the job ends
    /// ([`ToolboxApp::reply_job_waiters`]).
    pub(crate) fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            if req.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let _ = reply.send(json!({"ok": false, "error": "timeout"}));
                continue;
            }
            match handle(self, ctx, &req) {
                Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                Outcome::AfterJob(job) => self.control_waiters.push((job, reply)),
            }
        }
        self.control_rx = Some(rx);
    }

    /// A job ended: answer the control requests that waited for it.
    pub(crate) fn reply_job_waiters(&mut self, event: &JobEvent) {
        let (done, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.control_waiters).into_iter().partition(|(job, _)| *job == event.id);
        self.control_waiters = rest;
        for (_, reply) in done {
            let v = match &event.result {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": e}),
            };
            let _ = reply.send(v);
        }
    }
}
