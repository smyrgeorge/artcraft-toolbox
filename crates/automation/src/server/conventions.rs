//! Catalog metadata, strict tool envelopes and live resources.
use super::*;
use rmcp::RoleServer;
use rmcp::model::*;
use rmcp::service::RequestContext;

const COMMANDS: &str = "artcraft-toolbox://commands";
const APPS: &str = "artcraft-toolbox://apps";
const INSTRUCTIONS: &str = "ArtCraft Toolbox installs, updates, rolls back and launches the Crafting Apps (PhotoCraft, VectorCraft, …). \
Start with apps_status (every app's install and update state), discover command ids and parameter docs with command_list, \
and act with command_run or command_batch: updates.check fetches the release feeds, app.install / app.update / app.rollback / \
app.uninstall / app.launch manage one app, apps.updateAll every app, toolbox.update the toolbox itself. Background commands \
wait by default (wait: false returns a job for jobs_list / jobs_cancel). Every download is verified before it is installed; \
nothing ever elevates. ui_get, ui_set and control_call need bridge mode (a running desktop app). \
Read artcraft-toolbox://apps and artcraft-toolbox://commands for live JSON state.";

fn annotated(mut tool: Tool) -> Tool {
    let words = tool.name.replace('_', " ");
    let title = match words.strip_prefix("ui ") {
        Some(rest) => format!("UI {rest}"),
        None => {
            let mut chars = words.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    };
    let read = matches!(tool.name.as_ref(), "command_list" | "apps_status" | "jobs_list" | "ui_get");
    tool.title = Some(title.clone());
    tool.annotations = Some(ToolAnnotations::from_raw(Some(title), Some(read), Some(!read), Some(read), Some(false)));
    Arc::make_mut(&mut tool.input_schema).insert("additionalProperties".into(), json!(false));
    tool
}

impl ServerHandler for ToolboxMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("artcraft-toolbox", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        let tools: Vec<_> = self.tool_router.list_all().into_iter().map(annotated).collect();
        serde_json::from_value(json!({"resultType":"complete", "ttlMs":600000, "cacheScope":"private", "tools":tools}))
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        if let Some(tool) = self.tool_router.get(&request.name) {
            let properties = tool.input_schema.get("properties").and_then(Value::as_object);
            if let Some(args) = &request.arguments
                && let Some(key) = args.keys().find(|k| properties.is_none_or(|p| !p.contains_key(*k)))
            {
                return Err(McpError::invalid_params(format!("unknown argument `{key}` for {}", request.name), None));
            }
        }
        if request.name == "command_batch" {
            let args = Value::Object(request.arguments.clone().unwrap_or_default());
            let _: BatchParams = serde_json::from_value(args).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_resources(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListResourcesResult, McpError> {
        serde_json::from_value(json!({"resultType":"complete", "ttlMs":600000, "cacheScope":"private", "resources":[
            {"uri":APPS, "name":"apps", "title":"Apps and their updates", "mimeType":"application/json", "description":"Live apps_status: every Crafting App's install and update state, and the toolbox's own."},
            {"uri":COMMANDS, "name":"commands", "title":"Command catalog", "mimeType":"application/json", "description":"Live command_list including enabled state."}
        ]})).map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    async fn read_resource(&self, request: ReadResourceRequestParams, _: RequestContext<RoleServer>) -> Result<ReadResourceResponse, McpError> {
        let result = match request.uri.as_str() {
            APPS => self.apps_status().await?,
            COMMANDS => self.command_list(Parameters(ListParams::default())).await?,
            uri => return Err(McpError::resource_not_found(format!("unknown resource `{}`", uri.chars().take(80).collect::<String>()), None)),
        };
        let text = result.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.as_str()).collect::<Vec<_>>().join("\n");
        if result.is_error == Some(true) {
            return Err(McpError::internal_error(text, None));
        }
        let result: ReadResourceResult = serde_json::from_value(json!({"resultType":"complete", "ttlMs":0, "cacheScope":"private", "contents":[
            {"uri":request.uri, "mimeType":"application/json", "text":text}
        ]}))
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(result.into())
    }
}
