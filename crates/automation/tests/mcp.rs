//! Spin the MCP server in-process (tokio duplex), call tools as a client.

use artcraft_toolbox_automation::ToolboxMcp;
use artcraft_toolbox_engine::{Session, Target};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

fn session() -> Session {
    let mut s = Session::new().unwrap();
    s.set_host(Target::from_consts("linux", "x86_64"));
    s.ingest_releases("photocraft", PHOTOCRAFT, 1).unwrap();
    s
}

async fn connect(server: ToolboxMcp) -> RunningService<RoleClient, Client> {
    let (s, c) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(s).await {
            let _ = running.waiting().await;
        }
    });
    Client.serve(c).await.expect("client init")
}

async fn call(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    client.call_tool(p).await.expect("call_tool transport")
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

fn json_of(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "tool error: {}", text(r));
    serde_json::from_str(&text(r)).unwrap_or_else(|e| panic!("not JSON ({e}): {}", text(r)))
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_expected_tools_and_resources() {
    let client = connect(ToolboxMcp::headless(session())).await;
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    for n in ["command_list", "command_run", "command_batch", "apps_status", "jobs_list", "jobs_cancel", "ui_get", "ui_set", "control_call"] {
        assert!(names.contains(&n.to_string()), "missing tool {n}: {names:?}");
    }
    for t in &tools {
        assert!(t.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "tool name `{}`", t.name);
        assert!(t.description.as_ref().is_some_and(|d| !d.is_empty()));
        assert!(t.annotations.is_some() && t.title.is_some(), "{}", t.name);
        assert_eq!(t.input_schema.get("additionalProperties"), Some(&json!(false)), "{}", t.name);
        assert!(!serde_json::to_string(&t.input_schema).unwrap().contains("$ref"), "{}", t.name);
    }
    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().unwrap().name, "artcraft-toolbox");
    assert!(info.instructions.as_ref().is_some_and(|i| i.contains("apps_status") && i.contains("command_list")));
    let resources = client.list_all_resources().await.unwrap();
    let uris: Vec<String> = resources.iter().map(|r| r.uri.to_string()).collect();
    assert!(uris.contains(&"artcraft-toolbox://apps".to_string()) && uris.contains(&"artcraft-toolbox://commands".to_string()), "{uris:?}");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_tools_drive_the_session() {
    let client = connect(ToolboxMcp::headless(session())).await;
    // The apps and the toolbox's own status, as JSON.
    let r = json_of(&call(&client, "apps_status", json!({})).await);
    let photocraft = r["apps"].as_array().unwrap().iter().find(|a| a["id"] == "photocraft").unwrap();
    assert_eq!(photocraft["status"]["state"], "notInstalled");
    assert_eq!(photocraft["status"]["latest"], "0.5.0");
    assert_eq!(r["toolbox"]["id"], "artcraft-toolbox");
    // Commands: listed, filtered, run.
    let all = json_of(&call(&client, "command_list", json!({})).await);
    assert!(all.as_array().unwrap().iter().any(|c| c["id"] == "app.install" && c["enabled"] == false && c["background"] == true));
    let few = json_of(&call(&client, "command_list", json!({"filter": "toolbox", "enabled_only": true})).await);
    let ids: Vec<&str> = few.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"toolbox.status") && !ids.contains(&"toolbox.update"), "{ids:?}");
    let r = json_of(&call(&client, "command_run", json!({"id": "settings.set", "params": {"channel": "prerelease"}})).await);
    assert_eq!(r["channel"], "prerelease");
    let r = json_of(&call(&client, "command_run", json!({"id": "app.status", "params": {"app": "photocraft"}})).await);
    assert_eq!(
        r["status"]["latest"],
        "0.6.0-rc.1".to_string().replace("0.6.0-rc.1", r["status"]["latest"].as_str().unwrap()),
        "a pre-release channel offers the newest"
    );
    // A failing command is a tool error, not a transport error.
    let r = call(&client, "command_run", json!({"id": "app.install", "params": {"app": "photocraft"}})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("no network access"), "{}", text(&r));
    let r = call(&client, "command_run", json!({"id": "app.status", "params": [1]})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("must be a JSON object"), "{}", text(&r));
    // Batches keep every step's outcome.
    let r = json_of(
        &call(
            &client,
            "command_batch",
            json!({"steps": [{"id": "settings.set", "params": {"keepPrevious": 2}}, {"id": "nope.nope"}, {"id": "settings.get"}], "stop_on_error": false}),
        )
        .await,
    );
    assert_eq!((r["completed"].as_u64(), r["failed"].as_u64()), (Some(2), Some(1)));
    assert_eq!(r["results"][2]["result"]["keepPrevious"], 2);
    // Jobs: none running; cancelling nothing is fine; a bogus id is an error.
    assert_eq!(json_of(&call(&client, "jobs_list", json!({})).await)["jobs"], json!([]));
    assert_eq!(json_of(&call(&client, "jobs_cancel", json!({})).await)["cancelled"], 0);
    assert_eq!(call(&client, "jobs_cancel", json!({"job": 7})).await.is_error, Some(true));
    // Window tools need the bridge.
    let r = call(&client, "ui_get", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("bridge mode") && text(&r).contains("--control"), "{}", text(&r));
    assert_eq!(call(&client, "ui_set", json!({"fields": {"tab": "settings"}})).await.is_error, Some(true));
    assert_eq!(call(&client, "control_call", json!({"method": "ui.get"})).await.is_error, Some(true));
    // Resources are the live JSON.
    let read = client.read_resource(rmcp::model::ReadResourceRequestParams::new("artcraft-toolbox://apps")).await.unwrap();
    let body = read.contents.iter().find_map(|c| match c {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => Some(text.clone()),
        _ => None,
    });
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(v["apps"].as_array().map(Vec::len), Some(13));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_arguments_are_rejected_before_anything_runs() {
    let client = connect(ToolboxMcp::headless(session())).await;
    let mut p = CallToolRequestParams::new("command_run");
    p = p.with_arguments(json!({"id": "settings.set", "params": {"channel": "prerelease"}, "typo": 1}).as_object().unwrap().clone());
    let err = client.call_tool(p).await.unwrap_err();
    assert!(err.to_string().contains("unknown argument `typo`"), "{err}");
    let r = json_of(&call(&client, "command_run", json!({"id": "settings.get"})).await);
    assert_eq!(r["channel"], "stable", "the rejected call changed nothing");
    client.cancel().await.unwrap();
}
