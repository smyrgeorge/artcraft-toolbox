//! A [`Session`] driven by the control protocol's methods, without a window: what
//! `artcraft-toolbox-cli serve` and the headless MCP server run. The same method names the
//! desktop app's control channel answers (`ui_egui::control`), minus the `ui.*` ones.
//!
//! Methods (camelCase params; `docs/control-protocol.md`):
//! - `engine.execute {command, params?, wait?}`: any engine command by id; a background command
//!   (`updates.check`, `app.install`, …) runs to its end unless `wait` is false, which replies
//!   `{job, pending: true}` at once
//! - `engine.commands {filter?}`: the registry, with each command's enabled state
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs
//! - `batch {steps: [{command, params?, wait?} | {method, params?}], stopOnError?}`
//! - `methods`: this list

use artcraft_toolbox_engine::{JobId, Session, Started, command_specs};
use serde_json::{Value, json};

use crate::AutomationError;
use crate::budgets::BatchReplyBudget;
use crate::security::MAX_BATCH_STEPS;

/// Method names served by [`Headless::handle`].
pub const METHODS: &[&str] = &["engine.execute", "engine.commands", "jobs.list", "jobs.cancel", "batch", "methods"];

pub struct Headless {
    pub session: Session,
}

fn bad(msg: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(msg.into())
}

pub(crate) fn str_of<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

/// `params` as a JSON object: absent or null is empty; anything else is an error naming the
/// command (an array or a string would otherwise reach the engine as a bad request).
pub(crate) fn params_object(command: &str, params: Option<&Value>) -> Result<Value, AutomationError> {
    match params {
        None | Some(Value::Null) => Ok(json!({})),
        Some(v @ Value::Object(_)) => Ok(v.clone()),
        Some(other) => Err(bad(format!("`{command}`: params must be a JSON object, got {}", kind(other)))),
    }
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

/// The command registry as agents see it: id, label, params doc, whether it runs in the
/// background, and whether it can run right now in `session`.
pub fn command_list(session: &Session) -> Value {
    Value::Array(
        command_specs()
            .iter()
            .map(|c| {
                let reason = session.disabled_reason(c.id);
                json!({"id": c.id, "label": c.label, "params": c.params, "background": c.start.is_some(), "enabled": reason.is_none(), "disabledReason": reason})
            })
            .collect(),
    )
}

/// `command_list` narrowed to ids or labels containing `filter` (case-insensitive).
pub fn filter_commands(all: Value, filter: Option<&str>) -> Value {
    let Some(needle) = filter.map(str::to_lowercase).filter(|n| !n.is_empty()) else { return all };
    Value::Array(
        all.as_array()
            .into_iter()
            .flatten()
            .filter(|c| format!("{} {}", str_of(c, "id").unwrap_or(""), str_of(c, "label").unwrap_or("")).to_lowercase().contains(&needle))
            .cloned()
            .collect(),
    )
}

/// `jobs.list`'s result: the running jobs.
pub fn jobs_list(session: &Session) -> Value {
    json!({"jobs": session.jobs()})
}

/// `jobs.cancel`: one job by id, or every running one.
pub fn jobs_cancel(session: &mut Session, p: &Value) -> Result<Value, AutomationError> {
    match p.get("job") {
        None | Some(Value::Null) => {
            let ids: Vec<JobId> = session.jobs().into_iter().map(|j| j.id).collect();
            for id in &ids {
                session.cancel_job(*id);
            }
            Ok(json!({"cancelled": ids.len()}))
        }
        Some(v) => {
            let id = v.as_u64().ok_or_else(|| bad("`job` must be a job id"))?;
            if session.cancel_job(JobId(id)) { Ok(json!({"cancelled": 1})) } else { Err(bad(format!("no running job {id}"))) }
        }
    }
}

impl Headless {
    pub fn new(session: Session) -> Headless {
        Headless { session }
    }

    /// Apply what finished in the background (every request sees it).
    pub fn sync_jobs(&mut self) {
        self.session.poll_jobs();
    }

    /// Run a command; with `wait` false a background command returns `{job, pending: true}` at
    /// once (poll `jobs.list`, stop it with `jobs.cancel`). Other commands finish either way.
    pub fn command_start(&mut self, id: &str, params: Option<&Value>, wait: bool) -> Result<Value, AutomationError> {
        let params = params_object(id, params)?;
        self.sync_jobs();
        if wait {
            return Ok(self.session.execute(id, params)?);
        }
        Ok(match self.session.start(id, params)? {
            Started::Done(v) => v,
            Started::Job(job) => json!({"job": job.0, "pending": true}),
        })
    }

    /// Dispatch one request. Unknown methods and bad params are errors, never panics.
    pub fn handle(&mut self, method: &str, params: Value) -> Result<Value, AutomationError> {
        self.sync_jobs();
        let p = if params.is_null() { json!({}) } else { params };
        match method {
            "engine.execute" => {
                let id = str_of(&p, "command").ok_or_else(|| bad("engine.execute needs `command`"))?;
                let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
                self.command_start(id, p.get("params"), wait)
            }
            "engine.commands" => Ok(filter_commands(command_list(&self.session), str_of(&p, "filter"))),
            "jobs.list" => Ok(jobs_list(&self.session)),
            "jobs.cancel" => jobs_cancel(&mut self.session, &p),
            "batch" => self.batch(&p),
            "methods" => Ok(json!(METHODS)),
            other => Err(bad(format!("unknown method `{}` (try `methods`)", other.chars().take(80).collect::<String>()))),
        }
    }

    /// Run `steps` in order. Each step is `{command, params?, wait?}` (an engine command) or
    /// `{method, params?}` (any [`METHODS`] entry but `batch`). Stops at the first error unless
    /// `stopOnError` is false; the reply lists every step's result.
    pub fn batch(&mut self, p: &Value) -> Result<Value, AutomationError> {
        self.batch_with_budget(p, BatchReplyBudget::default())
    }

    /// [`Headless::batch`] with a caller's reply budget (MCP charges escaped sizes). Steps stop
    /// once the budget runs out.
    pub fn batch_with_budget(&mut self, p: &Value, mut reply_budget: BatchReplyBudget) -> Result<Value, AutomationError> {
        let steps = p.get("steps").and_then(Value::as_array).ok_or_else(|| bad("batch needs `steps`"))?;
        if steps.len() > MAX_BATCH_STEPS {
            return Err(bad(format!("batch contains {} steps; maximum is {MAX_BATCH_STEPS}", steps.len())));
        }
        let stop = p.get("stopOnError").and_then(Value::as_bool).unwrap_or(true);
        let mut results = Vec::with_capacity(steps.len());
        let mut failed = 0usize;
        for (i, s) in steps.iter().enumerate() {
            let r = if let Some(c) = str_of(s, "command") {
                let wait = s.get("wait").and_then(Value::as_bool).unwrap_or(true);
                self.command_start(c, s.get("params"), wait)
            } else if let Some(m) = str_of(s, "method") {
                if m == "batch" { Err(bad("nested batch")) } else { self.handle(m, s.get("params").cloned().unwrap_or(Value::Null)) }
            } else {
                Err(bad(format!("step {i} needs `command` or `method`")))
            };
            let was_error = r.is_err();
            let result = match r {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            };
            if let Err(error) = reply_budget.charge(&result) {
                failed += 1;
                results.push(json!({"ok": false, "error": format!("batch response budget exceeded; current step may have completed: {error}")}));
                break;
            }
            results.push(result);
            if was_error {
                failed += 1;
                if stop {
                    break;
                }
            }
        }
        Ok(json!({"completed": results.len() - failed, "failed": failed, "results": results}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless() -> Headless {
        let mut s = Session::new().unwrap();
        s.set_host(artcraft_toolbox_engine::Target::from_consts("linux", "x86_64"));
        s.ingest_releases("photocraft", include_str!("../../feed/tests/fixtures/photocraft-releases.json"), 1).unwrap();
        Headless::new(s)
    }

    #[test]
    fn engine_methods_run_commands_and_list_them() {
        let mut h = headless();
        let r = h.handle("engine.execute", json!({"command": "app.status", "params": {"app": "photocraft"}})).unwrap();
        assert_eq!(r["status"]["state"], "notInstalled");
        let all = h.handle("engine.commands", json!({})).unwrap();
        let ids: Vec<&str> = all.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"app.install") && ids.contains(&"toolbox.update"));
        let install = all.as_array().unwrap().iter().find(|c| c["id"] == "app.install").unwrap();
        assert_eq!((install["enabled"].as_bool(), install["background"].as_bool()), (Some(false), Some(true)));
        assert!(install["disabledReason"].as_str().unwrap().contains("no network access"));
        let few = h.handle("engine.commands", json!({"filter": "SETTINGS"})).unwrap();
        assert!(few.as_array().unwrap().iter().all(|c| c["id"].as_str().unwrap().contains("settings")));
        assert!(!few.as_array().unwrap().is_empty());
        assert_eq!(h.handle("methods", Value::Null).unwrap(), json!(METHODS));
        for m in METHODS {
            if let Err(e) = h.handle(m, Value::Null) {
                assert!(!e.to_string().contains("unknown method"), "{m} is listed but not served: {e}");
            }
        }
    }

    #[test]
    fn bad_requests_are_errors_not_panics() {
        let mut h = headless();
        for (method, params, want) in [
            ("engine.execute", json!({}), "needs `command`"),
            ("engine.execute", json!({"command": "nope.nope"}), "unknown command"),
            ("engine.execute", json!({"command": "app.status", "params": [1]}), "must be a JSON object"),
            ("engine.execute", json!({"command": "app.status", "params": "x"}), "must be a JSON object"),
            ("jobs.cancel", json!({"job": "x"}), "must be a job id"),
            ("jobs.cancel", json!({"job": 99}), "no running job"),
            ("nope", json!({}), "unknown method"),
            ("batch", json!({}), "needs `steps`"),
            ("batch", json!({"steps": [{"method": "batch"}]}), "nested batch"),
        ] {
            let r = h.handle(method, params.clone());
            match r {
                Err(e) => assert!(e.to_string().contains(want), "{method} {params}: {e}"),
                // A batch reports its step's error inside the result.
                Ok(v) => assert!(v["results"][0]["error"].as_str().is_some_and(|e| e.contains(want)), "{method} {params}: {v}"),
            }
        }
        assert_eq!(h.handle("jobs.cancel", json!({})).unwrap()["cancelled"], 0);
        assert_eq!(h.handle("jobs.list", json!({})).unwrap()["jobs"], json!([]));
    }

    #[test]
    fn batch_stops_on_error_unless_told_not_to() {
        let mut h = headless();
        let steps = json!([
            {"command": "settings.set", "params": {"channel": "prerelease"}},
            {"command": "no.such.command"},
            {"command": "settings.get"},
            {"method": "methods"}
        ]);
        let r = h.handle("batch", json!({"steps": steps})).unwrap();
        assert_eq!((r["completed"].as_u64(), r["failed"].as_u64()), (Some(1), Some(1)));
        assert_eq!(r["results"].as_array().unwrap().len(), 2);
        let r = h.handle("batch", json!({"steps": steps, "stopOnError": false})).unwrap();
        assert_eq!(r["completed"], 3);
        assert_eq!(r["results"][2]["result"]["channel"], "prerelease");
        assert_eq!(r["results"][3]["result"], json!(METHODS));
        let too_many = vec![json!({"method": "methods"}); MAX_BATCH_STEPS + 1];
        assert!(h.handle("batch", json!({"steps": too_many})).unwrap_err().to_string().contains("maximum is 256"));
    }

    #[test]
    fn batch_response_budget_stops_later_steps() {
        let mut h = headless();
        let mut steps = vec![json!({"command": "catalog.list"}); MAX_BATCH_STEPS - 1];
        steps.push(json!({"command": "settings.set", "params": {"channel": "prerelease"}}));
        let r = h.batch_with_budget(&json!({"steps": steps, "stopOnError": false}), BatchReplyBudget { remaining: 40_000, escaped: false }).unwrap();
        assert_eq!(r["failed"], 1);
        assert!(r["results"].as_array().unwrap().last().unwrap()["error"].as_str().unwrap().contains("batch response budget exceeded"));
        let settings = h.handle("engine.execute", json!({"command": "settings.get"})).unwrap();
        assert_eq!(settings["channel"], "stable", "the step after the budget ran out did not run");
    }
}
