//! Localhost JSON-lines control server (one request per line, one reply per line), loopback
//! only, authenticated by the first frame (PhotoCraft's `control_server.rs`, ported). The
//! transport the MCP bridge wraps; the handlers are `artcraft_toolbox_ui_egui::control`.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use artcraft_toolbox_automation::budgets::write_reply;
use artcraft_toolbox_automation::security::{
    ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream, read_bounded_line,
};
use artcraft_toolbox_ui_egui::ControlRequest;
use serde_json::{Value, json};

/// How long a request may wait for the window to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Listen on `127.0.0.1:<port>` and hand requests to the window through the returned receiver
/// (`ToolboxApp::with_control`); `ctx` is woken for each. A port that can't be bound is logged
/// and the receiver stays silent.
pub fn start(port: u16, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    match listener.local_addr() {
        Ok(addr) => log::info!("control server listening on {addr}"),
        Err(_) => log::info!("control server listening on 127.0.0.1:{port}"),
    }
    let spawned = std::thread::Builder::new().name("control-listen".into()).spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        for mut stream in listener.incoming().flatten() {
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                continue;
            };
            let tx = tx.clone();
            let ctx = ctx.clone();
            let token = Arc::clone(&token);
            let spawned = std::thread::Builder::new().name("control-conn".into()).spawn(move || {
                let _permit = permit;
                serve(stream, &token, tx, ctx);
            });
            if let Err(e) = spawned {
                log::error!("couldn't serve a control connection: {e}");
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("couldn't start the control server: {e}");
    }
    rx
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read);
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({"id": null, "ok": false, "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes")});
                let _ = write_reply(&mut out, &reply);
                let _ = out.flush();
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() || !authenticated {
                break;
            }
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let deadline = Instant::now() + REPLY_TIMEOUT;
                let (mut req, rrx) = ControlRequest::new(method, params);
                req.deadline = Some(deadline);
                if tx.send(req).is_err() {
                    break;
                }
                ctx.request_repaint();
                let mut r = rrx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}));
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// One connection served by `serve`, with a handler thread answering through the channel.
    fn pair() -> (TcpStream, BufReader<TcpStream>, Receiver<ControlRequest>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        (stream, reader, rx, server)
    }

    fn line(reader: &mut BufReader<TcpStream>) -> Value {
        let mut s = String::new();
        reader.read_line(&mut s).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn requests_reach_the_handler_after_authentication_and_bad_json_keeps_serving() {
        let (mut stream, mut reader, rx, server) = pair();
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.one");
            assert_eq!(req.params, json!({"x": 1}));
            assert!(req.deadline.is_some_and(|d| d > Instant::now()));
            req.reply.send(json!({"ok": true, "result": "one"})).unwrap();
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.two");
            req.reply.send(json!({"ok": true, "result": "x".repeat(artcraft_toolbox_automation::budgets::MAX_RESPONSE_BYTES)})).unwrap();
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.three");
            req.reply.send(json!({"ok": true, "result": "three"})).unwrap();
        });
        // Nothing before the token.
        writeln!(stream, "{}", json!({"id": 0, "method": "test.one"})).unwrap();
        let v = line(&mut reader);
        assert_eq!((v["ok"].as_bool(), v["error"].as_str()), (Some(false), Some("authentication required")));
        // That connection is over; a new one authenticates.
        drop(reader);
        drop(stream);
        server.join().unwrap();
        let (mut stream, mut reader, rx2, server) = pair();
        let handler2 = std::thread::spawn(move || {
            for expected in ["test.one", "test.two", "test.three"] {
                let req = rx2.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(req.method, expected);
                let result = if expected == "test.two" { json!("x".repeat(artcraft_toolbox_automation::budgets::MAX_RESPONSE_BYTES)) } else { json!(expected) };
                req.reply.send(json!({"ok": true, "result": result})).unwrap();
            }
        });
        writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).unwrap();
        assert_eq!(line(&mut reader)["result"]["authenticated"], true);
        writeln!(stream, "{}", json!({"id": 1, "method": "test.one", "params": {"x": 1}})).unwrap();
        let v = line(&mut reader);
        assert_eq!((v["id"].as_u64(), v["result"].as_str()), (Some(1), Some("test.one")));
        // Bad JSON: an error with a null id, and the connection keeps serving.
        writeln!(stream, "{{not json}}").unwrap();
        let v = line(&mut reader);
        assert_eq!(v["id"], Value::Null);
        assert!(v["error"].as_str().unwrap().starts_with("bad JSON"));
        // An oversized reply is replaced by an error carrying the request's id.
        writeln!(stream, "{}", json!({"id": 2, "method": "test.two"})).unwrap();
        let v = line(&mut reader);
        assert_eq!((v["id"].as_u64(), v["ok"].as_bool()), (Some(2), Some(false)));
        assert!(v["error"].as_str().unwrap().contains("operation may have completed"));
        writeln!(stream, "{}", json!({"id": 3, "method": "test.three"})).unwrap();
        assert_eq!(line(&mut reader)["result"], "test.three");
        drop(reader);
        drop(stream);
        handler.join().unwrap_or(());
        handler2.join().unwrap();
        server.join().unwrap();
    }
}
