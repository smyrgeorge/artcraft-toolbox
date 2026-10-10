//! The MCP server (rmcp; PhotoCraft's `server.rs`, ported). Tool names use `snake_case` with
//! underscores (dots are not valid in every client's tool-name grammar).
//!
//! Headless mode drives an in-process [`Headless`] session (the same data folder as the desktop
//! app: inventory, settings, cached feeds); bridge mode forwards every tool to a running
//! desktop app over the control protocol, so agents also see and drive the live window
//! (`ui_get`, `ui_set`).

use std::sync::{Arc, Mutex, PoisonError};

use artcraft_toolbox_engine::Session;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock as Content};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AutomationError;
use crate::bridge::BridgeClient;
use crate::budgets::{BatchReplyBudget, json_bytes};
use crate::headless::{Headless, command_list, filter_commands, jobs_cancel, jobs_list};
use crate::security::MAX_BATCH_STEPS;

/// Where tools are executed.
pub enum Backend {
    /// An in-process engine session.
    Headless(Arc<Mutex<Headless>>),
    /// A running desktop app reached over the control protocol.
    Bridge(Arc<BridgeClient>),
}

#[derive(Clone)]
pub struct ToolboxMcp {
    backend: Arc<Backend>,
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// Only commands whose id or label contains this text (case-insensitive).
    #[serde(default)]
    pub filter: Option<String>,
    /// Only commands that can run right now.
    #[serde(default)]
    pub enabled_only: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    /// Command id, e.g. `updates.check`, `app.install`, `toolbox.status`.
    pub id: String,
    /// Command parameters as a JSON object (see the `params` doc in `command_list`).
    #[serde(default)]
    pub params: Option<Value>,
    /// Wait for a background command (`updates.check`, `app.install`, `app.update`,
    /// `toolbox.update`, …) to finish (default true). With false it runs in the background and
    /// the result is `{job, pending}`: poll it with `jobs_list`, stop it with `jobs_cancel`.
    #[serde(default)]
    pub wait: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobCancelParams {
    /// The job id from `command_run` / `jobs_list` (omit to cancel every running job).
    #[serde(default)]
    pub job: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchParams {
    /// Commands to run in order: `[{"id": "updates.check"}, {"id": "app.install", "params": {"app": "photocraft"}}]`.
    /// A step with `"wait": false` starts a background command and moves on (its result is
    /// `{job, pending}`, as in `command_run`).
    pub steps: Vec<RunParams>,
    /// Stop at the first failing step (default true).
    #[serde(default)]
    pub stop_on_error: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiSetParams {
    /// Fields for the control method `ui.set`: `tab` ("apps" or "settings"), `search` (the list's
    /// filter text), `searchOpen`, `availableFolded`, `selected` (an app id whose page is open, or
    /// null), `confirmUninstall` (an app id whose uninstall question is open, or null), `notice`
    /// (the banner's text, or null to dismiss it). Any other field is an error, checked before
    /// anything changes.
    pub fields: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlParams {
    /// Any control-protocol method (docs/control-protocol.md), e.g. `ui.get`.
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Rewrites a tool's input schema into the subset strict tool-schema validators accept: `$ref`s
/// into `$defs` are inlined and `$defs` dropped, and a `true` schema (any value, as schemars
/// writes for `Value`) becomes `{}`. Tool schemas travel to every provider a client uses, and
/// some reject the whole request over either construct.
fn strict_schema(schema: &mut serde_json::Map<String, Value>) {
    let defs = schema.remove("$defs").and_then(|d| d.as_object().cloned()).unwrap_or_default();
    let mut root = Value::Object(std::mem::take(schema));
    strict_node(&mut root, &defs, 0, true);
    if let Value::Object(m) = root {
        *schema = m;
    }
}

/// `v` (a schema when `is_schema`, else any value that may contain schemas) made strict. Inlining
/// stops at a fixed depth so a recursive definition can't loop.
fn strict_node(v: &mut Value, defs: &serde_json::Map<String, Value>, depth: usize, is_schema: bool) {
    const MAX_DEPTH: usize = 32;
    if is_schema && *v == Value::Bool(true) {
        *v = json!({});
        return;
    }
    let Value::Object(m) = v else {
        if let Value::Array(a) = v {
            for x in a {
                strict_node(x, defs, depth, is_schema);
            }
        }
        return;
    };
    if let Some(def) = m.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")).and_then(|name| defs.get(name))
        && depth < MAX_DEPTH
    {
        let mut inlined = def.clone();
        if let Value::Object(im) = &mut inlined {
            // Keep the use site's own annotations (its description) over the definition's.
            for (k, x) in m.iter().filter(|(k, _)| k.as_str() != "$ref") {
                im.insert(k.clone(), x.clone());
            }
        }
        *v = inlined;
        strict_node(v, defs, depth + 1, true);
        return;
    }
    for (k, x) in m.iter_mut() {
        match k.as_str() {
            "properties" | "patternProperties" => {
                if let Value::Object(props) = x {
                    for p in props.values_mut() {
                        strict_node(p, defs, depth, true);
                    }
                }
            }
            "items" | "additionalProperties" | "not" | "anyOf" | "oneOf" | "allOf" | "prefixItems" => strict_node(x, defs, depth, true),
            _ => {}
        }
    }
}

fn ok_json(v: &Value) -> CallToolResult {
    let text = match json_bytes(v).and_then(|bytes| String::from_utf8(bytes).map_err(|error| AutomationError::Other(error.to_string()))) {
        Ok(text) => text,
        Err(error) => return fail(format!("{error}; operation may have completed")),
    };
    bounded_tool_result(CallToolResult::success(vec![Content::text(text)]))
}

fn bounded_tool_result(result: CallToolResult) -> CallToolResult {
    match json_bytes(&result) {
        Ok(_) => result,
        Err(error) => fail(format!("{error}; operation may have completed")),
    }
}

fn fail(e: impl std::fmt::Display) -> CallToolResult {
    let result = CallToolResult::error(vec![Content::text(e.to_string())]);
    if json_bytes(&result).is_err() {
        return CallToolResult::error(vec![Content::text("response budget exceeded; operation may have completed")]);
    }
    result
}

/// Neither backend took the call: only guards against a future backend kind.
fn no_backend() -> CallToolResult {
    fail("internal error: no headless session or app bridge")
}

fn to_result(r: Result<Value, AutomationError>) -> Result<CallToolResult, McpError> {
    Ok(match r {
        Ok(v) => ok_json(&v),
        Err(e) => fail(e),
    })
}

fn bridge_only(name: &str) -> Result<CallToolResult, McpError> {
    Ok(fail(format!(
        "`{name}` drives the live window and needs bridge mode: start the app with \
         `artcraft-toolbox --control <port> --control-token-file <path>`, then run \
         `artcraft-toolbox-cli mcp --bridge 127.0.0.1:<port> --control-token-file <path>`"
    )))
}

impl ToolboxMcp {
    /// An in-process session (the CLI opens it on the toolbox's data folder).
    pub fn headless(session: Session) -> Self {
        Self::with_backend(Backend::Headless(Arc::new(Mutex::new(Headless::new(session)))))
    }

    /// Forward everything to the desktop app listening on `addr` (loopback) with `token`.
    pub fn bridge(addr: &str, token: &str) -> Result<Self, AutomationError> {
        Ok(Self::with_backend(Backend::Bridge(Arc::new(BridgeClient::new(addr, token)?))))
    }

    pub fn with_backend(backend: Backend) -> Self {
        let mut tool_router = Self::tool_router();
        for route in tool_router.map.values_mut() {
            let mut schema = (*route.attr.input_schema).clone();
            strict_schema(&mut schema);
            route.attr.input_schema = Arc::new(schema);
        }
        ToolboxMcp { backend: Arc::new(backend), tool_router }
    }

    /// Serve MCP over stdin/stdout until the client disconnects.
    pub async fn serve_stdio(self) -> Result<(), AutomationError> {
        self.serve_io(tokio::io::stdin(), tokio::io::stdout()).await
    }

    /// Run `f` on the headless session on a blocking thread. `None` in bridge mode.
    async fn headless_op<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Headless) -> Result<T, AutomationError> + Send + 'static,
    ) -> Option<Result<T, AutomationError>> {
        let Backend::Headless(h) = &*self.backend else {
            return None;
        };
        let h = h.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                // A panicking command poisons the lock, but `Session::execute` catches a
                // command's panic before it changes anything: keep serving.
                let mut g = h.lock().unwrap_or_else(PoisonError::into_inner);
                g.sync_jobs();
                f(&mut g)
            })
            .await
            .unwrap_or_else(|e| Err(AutomationError::Other(format!("task failed: {e}")))),
        )
    }

    fn bridge_client(&self) -> Option<&BridgeClient> {
        match &*self.backend {
            Backend::Bridge(b) => Some(b),
            Backend::Headless(_) => None,
        }
    }

    async fn run_command(&self, id: String, params: Value, wait: bool) -> Result<CallToolResult, McpError> {
        let (id2, params2) = (id.clone(), params.clone());
        if let Some(r) = self.headless_op(move |h| h.command_start(&id2, Some(&params2), wait)).await {
            return to_result(r);
        }
        let Some(b) = self.bridge_client() else {
            return Ok(no_backend());
        };
        to_result(b.call("engine.execute", json!({"command": id, "params": params, "wait": wait})).await)
    }

    /// A control method in either backend (the headless session answers the same names).
    async fn call(&self, method: &'static str, params: Value) -> Result<Value, AutomationError> {
        let p2 = params.clone();
        if let Some(r) = self.headless_op(move |h| h.handle(method, p2)).await {
            return r;
        }
        match self.bridge_client() {
            Some(b) => b.call(method, params).await,
            None => Err(AutomationError::Other("no headless session or app bridge".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router]
impl ToolboxMcp {
    #[tool(description = "List engine commands: id, label, parameter doc, whether it runs in the background, and whether it \
        can run now (with the reason when it can't). Every action of the toolbox is one of these.")]
    async fn command_list(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let all = if let Some(r) = self.headless_op(|h| Ok(command_list(&h.session))).await {
            r
        } else {
            let Some(b) = self.bridge_client() else {
                return Ok(no_backend());
            };
            b.call("engine.commands", json!({})).await
        };
        let all = match all {
            Ok(v) => filter_commands(v, p.filter.as_deref()),
            Err(e) => return Ok(fail(e)),
        };
        let enabled_only = p.enabled_only.unwrap_or(false);
        let items: Vec<Value> = all
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !enabled_only || c.get("enabled").and_then(Value::as_bool).unwrap_or(true))
            .collect();
        Ok(ok_json(&Value::Array(items)))
    }

    #[tool(description = "Run an engine command by id with JSON params (see command_list). Returns the command's JSON result. \
        Background commands (updates.check, app.install, app.update, apps.updateAll's jobs, toolbox.update) wait to finish \
        unless `wait` is false: then the result is {job, pending} (see jobs_list, jobs_cancel).")]
    async fn command_run(&self, Parameters(p): Parameters<RunParams>) -> Result<CallToolResult, McpError> {
        self.run_command(p.id, p.params.unwrap_or_else(|| json!({})), p.wait.unwrap_or(true)).await
    }

    #[tool(description = "Run several engine commands in one call (fewer round trips). Returns {completed, failed, \
        results:[{ok, result|error}]}; stops at the first error unless stop_on_error is false.")]
    async fn command_batch(&self, Parameters(p): Parameters<BatchParams>) -> Result<CallToolResult, McpError> {
        if p.steps.len() > MAX_BATCH_STEPS {
            return Ok(fail(format!("batch contains {} steps; maximum is {MAX_BATCH_STEPS}", p.steps.len())));
        }
        let stop = p.stop_on_error.unwrap_or(true);
        let steps: Vec<Value> =
            p.steps.into_iter().map(|s| json!({"command": s.id, "params": s.params.unwrap_or_else(|| json!({})), "wait": s.wait.unwrap_or(true)})).collect();
        let args = json!({"steps": steps, "stopOnError": stop});
        // The reply travels as escaped JSON text, so steps are charged their escaped size.
        if let Some(r) = self.headless_op(move |h| h.batch_with_budget(&args, BatchReplyBudget::escaped())).await {
            return to_result(r);
        }
        let Some(b) = self.bridge_client() else {
            return Ok(no_backend());
        };
        let mut results = Vec::new();
        let mut failed = 0;
        let mut reply_budget = BatchReplyBudget::escaped();
        for s in &steps {
            let response = b.call("engine.execute", s.clone()).await;
            let was_error = response.is_err();
            let result = match response {
                Ok(value) => json!({"ok": true, "result": value}),
                Err(error) => json!({"ok": false, "error": error.to_string()}),
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
        Ok(ok_json(&json!({"completed": results.len() - failed, "failed": failed, "results": results})))
    }

    #[tool(description = "Every Crafting App's install and update status (what apps.status returns) and the toolbox's own \
        (toolbox.status): installed and latest versions, channel, pin, the last check, errors.")]
    async fn apps_status(&self) -> Result<CallToolResult, McpError> {
        let apps = self.call("engine.execute", json!({"command": "apps.status"})).await;
        let toolbox = self.call("engine.execute", json!({"command": "toolbox.status"})).await;
        match (apps, toolbox) {
            (Ok(apps), toolbox) => Ok(ok_json(&json!({"apps": apps, "toolbox": toolbox.unwrap_or(Value::Null)}))),
            (Err(e), _) => Ok(fail(e)),
        }
    }

    #[tool(description = "List the running background jobs: id, command, label, progress (done of total items, a phase and a \
        fraction for installs) and the apps being worked on.")]
    async fn jobs_list(&self) -> Result<CallToolResult, McpError> {
        if let Some(r) = self.headless_op(|h| Ok(jobs_list(&h.session))).await {
            return to_result(r);
        }
        let Some(b) = self.bridge_client() else {
            return Ok(no_backend());
        };
        to_result(b.call("jobs.list", json!({})).await)
    }

    #[tool(description = "Cancel a background job by id (or every running job when `job` is omitted). A cancelled install \
        keeps its partial download for the next attempt; the version in use is untouched.")]
    async fn jobs_cancel(&self, Parameters(p): Parameters<JobCancelParams>) -> Result<CallToolResult, McpError> {
        let params = p.job.map_or_else(|| json!({}), |j| json!({"job": j}));
        let p2 = params.clone();
        if let Some(r) = self.headless_op(move |h| jobs_cancel(&mut h.session, &p2)).await {
            return to_result(r);
        }
        let Some(b) = self.bridge_client() else {
            return Ok(no_backend());
        };
        to_result(b.call("jobs.cancel", params).await)
    }

    // ----- live-window tools (bridge mode) -----

    #[tool(description = "Bridge mode: the live window's state (tab, search, the open app page, the uninstall question, the \
        banner, window size) and the services the platform provides.")]
    async fn ui_get(&self) -> Result<CallToolResult, McpError> {
        match self.bridge_client() {
            Some(b) => to_result(b.call("ui.get", json!({})).await),
            None => bridge_only("ui_get"),
        }
    }

    #[tool(description = "Bridge mode: change the live window's state (tab, search, searchOpen, availableFolded, selected, \
        confirmUninstall, notice; see `fields`). Unknown fields are an error and nothing changes.")]
    async fn ui_set(&self, Parameters(p): Parameters<UiSetParams>) -> Result<CallToolResult, McpError> {
        match self.bridge_client() {
            Some(b) => to_result(b.call("ui.set", p.fields).await),
            None => bridge_only("ui_set"),
        }
    }

    #[tool(description = "Bridge mode: call any control-protocol method (docs/control-protocol.md) with raw params.")]
    async fn control_call(&self, Parameters(p): Parameters<ControlParams>) -> Result<CallToolResult, McpError> {
        match self.bridge_client() {
            Some(b) => to_result(b.call(&p.method, p.params.unwrap_or_else(|| json!({}))).await),
            None => bridge_only("control_call"),
        }
    }
}

mod conventions;
mod transport;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_schema_inlines_refs_and_stops_on_recursive_definitions() {
        let mut s = json!({
            "type": "object",
            "properties": {
                "node": {"$ref": "#/$defs/Node", "description": "use-site text"},
                "any": true,
                "list": {"type": "array", "items": true}
            },
            "$defs": {"Node": {"type": "object", "description": "def text", "properties": {"child": {"$ref": "#/$defs/Node"}}}}
        });
        let m = s.as_object_mut().unwrap();
        strict_schema(m);
        assert!(m.get("$defs").is_none());
        let props = &m["properties"];
        assert_eq!(props["node"]["type"], "object");
        assert_eq!(props["node"]["description"], "use-site text");
        assert_eq!(props["node"]["properties"]["child"]["type"], "object");
        assert_eq!((&props["any"], &props["list"]["items"]), (&json!({}), &json!({})));
        assert!(serde_json::to_string(&s).unwrap().len() < 1 << 20);
    }

    #[test]
    fn mcp_text_checks_the_encoded_tool_envelope() {
        let result = ok_json(&json!("\n".repeat(crate::budgets::MAX_RESPONSE_BYTES / 3)));
        assert_eq!(result.is_error, Some(true));
        assert!(json_bytes(&result).is_ok());
        let error = fail("x".repeat(crate::budgets::MAX_RESPONSE_BYTES));
        assert_eq!(error.is_error, Some(true));
        assert!(json_bytes(&error).is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_panicking_operation_does_not_wedge_the_session() {
        let mcp = ToolboxMcp::headless(Session::new().unwrap());
        #[allow(clippy::panic)]
        let r = mcp.headless_op(|_| -> Result<(), AutomationError> { panic!("boom") }).await.unwrap();
        assert!(r.is_err());
        let r = mcp.headless_op(|h| h.command_start("catalog.list", None, true)).await.unwrap();
        assert!(r.is_ok(), "session still usable after a panic: {r:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mcp_command_batch_stopped_by_the_budget_keeps_its_step_results() {
        let mcp = ToolboxMcp::headless(Session::new().unwrap());
        let step = |id: &str, params: Value| RunParams { id: id.into(), params: Some(params), wait: None };
        // The catalog is small, so a budget test needs the steps to add up: each catalog.list is
        // a few KB; 255 of them stay under the 8 MiB budget. Exercise the loop shape instead.
        let mut steps: Vec<RunParams> = (0..MAX_BATCH_STEPS - 1).map(|_| step("catalog.list", json!({}))).collect();
        steps.push(step("settings.set", json!({"channel": "prerelease"})));
        let result = mcp.command_batch(Parameters(BatchParams { steps, stop_on_error: Some(false) })).await.unwrap();
        assert_ne!(result.is_error, Some(true));
        let text = &result.content.first().and_then(|c| c.as_text()).unwrap().text;
        let reply: Value = serde_json::from_str(text).unwrap();
        assert_eq!(reply["completed"].as_u64().unwrap() as usize, MAX_BATCH_STEPS);
        assert_eq!(reply["failed"], 0);
        let too_many: Vec<RunParams> = (0..MAX_BATCH_STEPS + 1).map(|_| step("catalog.list", json!({}))).collect();
        let result = mcp.command_batch(Parameters(BatchParams { steps: too_many, stop_on_error: None })).await.unwrap();
        assert_eq!(result.is_error, Some(true));
    }
}
