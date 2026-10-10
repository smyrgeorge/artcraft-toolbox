//! Exercise the actual line transport: a malformed line gets -32700 and the next one is served,
//! and a peer on the 2026-07-28 protocol gets the modern result fields.
use artcraft_toolbox_automation::ToolboxMcp;
use artcraft_toolbox_engine::Session;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test(flavor = "multi_thread")]
async fn malformed_line_recovers_and_lists_are_complete() {
    let (mut input, server_in) = tokio::io::duplex(1 << 20);
    let (server_out, output) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(ToolboxMcp::headless(Session::new().unwrap()).serve_io(server_in, server_out));
    let mut lines = BufReader::new(output).lines();
    let init = json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-06-18", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}}});
    input.write_all(format!("{init}\n").as_bytes()).await.unwrap();
    lines.next_line().await.unwrap().unwrap();
    input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\nBROKEN\n").await.unwrap();
    let error: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(error["id"], Value::Null);
    assert_eq!(error["error"]["code"], -32700);
    // A peer that negotiated the legacy version gets the historical shape (no resultType).
    input.write_all(format!("{}\n", json!({"jsonrpc":"2.0", "id":9, "method":"tools/list", "params":{}})).as_bytes()).await.unwrap();
    let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["id"], 9);
    assert!(reply["result"]["resultType"].is_null(), "{reply}");
    assert_eq!(reply["result"]["tools"].as_array().map(Vec::len), Some(9));
    let meta = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28", "io.modelcontextprotocol/clientInfo":{"name":"test", "version":"1"}, "io.modelcontextprotocol/clientCapabilities":{}});
    for (id, method, extra) in
        [(2, "tools/list", json!({})), (3, "resources/list", json!({})), (4, "resources/read", json!({"uri":"artcraft-toolbox://commands"}))]
    {
        let mut params = extra;
        params["_meta"] = meta.clone();
        input.write_all(format!("{}\n", json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})).as_bytes()).await.unwrap();
        let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
        assert!(reply["result"]["ttlMs"].is_number());
        assert_eq!(reply["result"]["cacheScope"], "private");
    }
    let call = json!({"jsonrpc":"2.0", "id":5, "method":"tools/call", "params":{"name":"command_run", "arguments":{"id":"catalog.list"}}});
    input.write_all(format!("{call}\n").as_bytes()).await.unwrap();
    let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let apps: Value = serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(apps.as_array().map(Vec::len), Some(12));
    drop(input);
    tokio::time::timeout(std::time::Duration::from_secs(5), server).await.unwrap().unwrap().unwrap();
}
