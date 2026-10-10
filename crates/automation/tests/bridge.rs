//! The MCP bridge against a real loopback control server. The headless server speaks the same
//! protocol as the desktop app (minus the window's methods), so it stands in for it here:
//! authentication, forwarding, error pass-through, and a wrong token.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use artcraft_toolbox_automation::{Headless, ToolboxMcp, rpc};
use artcraft_toolbox_engine::Session;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

/// A headless control server on a free loopback port, for the rest of the test process.
fn control_server() -> String {
    let h = Arc::new(Mutex::new(Headless::new(Session::new().unwrap())));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        rpc::serve_tcp("127.0.0.1:0", h, TOKEN.to_string(), |addr| tx.send(addr).unwrap()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("the control server did not start").to_string()
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
async fn the_bridge_forwards_every_tool_over_one_authenticated_connection() {
    let addr = control_server();
    let client = connect(ToolboxMcp::bridge(&addr, TOKEN).unwrap()).await;
    let all = json_of(&call(&client, "command_list", json!({"filter": "settings"})).await);
    let ids: Vec<&str> = all.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"settings.get") && ids.contains(&"settings.set"), "{ids:?}");
    assert!(ids.iter().all(|id| id.contains("settings")), "the filter was applied: {ids:?}");
    let r = json_of(&call(&client, "command_run", json!({"id": "settings.set", "params": {"channel": "prerelease"}})).await);
    assert_eq!(r["channel"], "prerelease");
    // The same session answers the next call (the setting stuck), on the same connection.
    let r = json_of(&call(&client, "command_run", json!({"id": "settings.get"})).await);
    assert_eq!(r["channel"], "prerelease");
    let r = json_of(&call(&client, "apps_status", json!({})).await);
    assert_eq!(r["apps"].as_array().map(Vec::len), Some(12));
    assert_eq!(r["toolbox"]["id"], "artcraft-toolbox");
    assert_eq!(json_of(&call(&client, "jobs_list", json!({})).await)["jobs"], json!([]));
    assert_eq!(json_of(&call(&client, "jobs_cancel", json!({})).await)["cancelled"], 0);
    let r = json_of(&call(&client, "command_batch", json!({"steps": [{"id": "settings.set", "params": {"keepPrevious": 3}}, {"id": "settings.get"}]})).await);
    assert_eq!((r["completed"].as_u64(), r["failed"].as_u64()), (Some(2), Some(0)));
    assert_eq!(r["results"][1]["result"]["keepPrevious"], 3);
    let r = json_of(&call(&client, "control_call", json!({"method": "methods"})).await);
    assert!(r.as_array().unwrap().iter().any(|m| m == "batch"), "{r}");
    // The app's errors come back as tool errors, verbatim.
    let r = call(&client, "command_run", json!({"id": "nope.nope"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("nope.nope"), "{}", text(&r));
    let r = call(&client, "command_run", json!({"id": "settings.get", "params": "x"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("JSON object"), "{}", text(&r));
    // A method this server doesn't have (the window's) is an error, not a hang.
    let r = call(&client, "ui_get", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("unknown method"), "{}", text(&r));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_token_or_a_dead_server_is_a_clear_tool_error() {
    let addr = control_server();
    let other = "f".repeat(64);
    let client = connect(ToolboxMcp::bridge(&addr, &other).unwrap()).await;
    let r = call(&client, "command_list", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).to_lowercase().contains("token") || text(&r).to_lowercase().contains("auth"), "{}", text(&r));
    client.cancel().await.unwrap();
    // Nothing listens here: the error says how to start the app.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let client = connect(ToolboxMcp::bridge(&free, TOKEN).unwrap()).await;
    let r = call(&client, "jobs_list", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("--control"), "{}", text(&r));
    client.cancel().await.unwrap();
}
