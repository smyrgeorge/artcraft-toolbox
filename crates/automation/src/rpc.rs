//! Headless JSON-lines server: the control protocol's envelope
//! (`{"id","method","params"}` → `{"id","ok","result"|"error"}`) over a [`Headless`] session,
//! so scripts and agents keep one session open without MCP or the window.
//! `artcraft-toolbox-cli serve` runs it on stdio, or on a loopback TCP port with the same
//! first-frame `auth` as the desktop app's control channel.

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::budgets::write_reply;
use crate::security::{
    ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream, discard_rest_of_line, read_bounded_line,
};
use crate::{AutomationError, Headless};

/// Answer one request line. Malformed JSON gets an error reply with `id: null`.
pub fn respond(h: &Mutex<Headless>, line: &str) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return json!({"id": id, "ok": false, "error": "missing `method`"});
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    // Last-resort guard (AGENTS.md, Never crash): a method that panics answers this request with
    // an error instead of taking down the connection. The session is kept (a poisoned lock is
    // still usable: `Session::execute` catches a command's panic before it changes anything).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.lock().unwrap_or_else(PoisonError::into_inner).handle(method, params)))
        .unwrap_or_else(|_| Err(AutomationError::Other(format!("internal error: `{method}` panicked"))));
    match r {
        Ok(v) => json!({"id": id, "ok": true, "result": v}),
        Err(e) => json!({"id": id, "ok": false, "error": e.to_string()}),
    }
}

/// Serve JSON lines from `r` to `w` until EOF. Blank lines are ignored. An over-long or non-UTF-8
/// line gets an error reply and is skipped; the session keeps serving.
pub fn serve_lines(h: &Mutex<Headless>, mut r: impl BufRead, mut w: impl Write) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        let (reply, unread_rest) = match read_bounded_line(&mut r, &mut line) {
            Ok(LineRead::Eof) => return Ok(()),
            Ok(LineRead::TooLong) => (json!({"id": null, "ok": false, "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes")}), !line.ends_with('\n')),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => (json!({"id": null, "ok": false, "error": "request is not valid UTF-8"}), false),
            Err(e) => return Err(e),
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => (respond(h, &line), false),
        };
        write_reply(&mut w, &reply)?;
        w.flush()?;
        if unread_rest {
            discard_rest_of_line(&mut r)?;
        }
    }
}

fn serve_tcp_connection(h: &Mutex<Headless>, stream: std::net::TcpStream, token: &str) -> std::io::Result<()> {
    configure_stream(&stream)?;
    let read = stream.try_clone()?;
    let mut reader = std::io::BufReader::new(read);
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong => {
                write_reply(&mut out, &json!({"id": null, "ok": false, "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes")}))?;
                out.flush()?;
                return Ok(());
            }
            LineRead::Line if line.trim().is_empty() => continue,
            LineRead::Line => {}
        }
        let reply = if authenticated {
            respond(h, &line)
        } else {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            reply
        };
        write_reply(&mut out, &reply)?;
        out.flush()?;
        if !authenticated {
            return Ok(());
        }
    }
}

/// Serve authenticated JSON lines on a loopback TCP address with one shared session.
/// Refuses non-loopback addresses and caps active connections and request bytes. `ready` hears
/// the bound address (a port of 0 picks a free one).
pub fn serve_tcp(addr: &str, h: Arc<Mutex<Headless>>, token: String, ready: impl FnOnce(std::net::SocketAddr)) -> Result<(), AutomationError> {
    let listener = TcpListener::bind(addr).map_err(|e| AutomationError::Io(format!("bind {addr}: {e}")))?;
    let local = listener.local_addr().map_err(|e| AutomationError::Io(e.to_string()))?;
    if !local.ip().is_loopback() {
        return Err(AutomationError::BadRequest(format!("{addr} is not a loopback address")));
    }
    ready(local);
    let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
    let token = Arc::new(token);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Some(permit) = limiter.try_acquire() else {
            let _ = configure_stream(&stream);
            let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
            continue;
        };
        let h = h.clone();
        let token = Arc::clone(&token);
        let spawned = std::thread::Builder::new().name("control-conn".into()).spawn(move || {
            let _permit = permit;
            let _ = serve_tcp_connection(&h, stream, &token);
        });
        if let Err(e) = spawned {
            log::error!("couldn't serve a control connection: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use artcraft_toolbox_engine::Session;

    fn session() -> Mutex<Headless> {
        Mutex::new(Headless::new(Session::new().unwrap()))
    }

    #[test]
    fn lines_round_trip_and_errors() {
        let h = session();
        let input = concat!(
            r#"{"id":1,"method":"engine.execute","params":{"command":"settings.set","params":{"channel":"prerelease"}}}"#,
            "\n",
            "\n",
            r#"{"id":2,"method":"engine.execute","params":{"command":"settings.get"}}"#,
            "\n",
            r#"{"id":"x","method":"nope"}"#,
            "\n",
            r#"{"id":4}"#,
            "\n",
            "not json\n",
        );
        let mut out = Vec::new();
        serve_lines(&h, input.as_bytes(), &mut out).unwrap();
        let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 5);
        assert!(replies[0]["ok"].as_bool().unwrap());
        assert_eq!((replies[1]["id"].as_u64(), replies[1]["result"]["channel"].as_str()), (Some(2), Some("prerelease")));
        assert_eq!(replies[2]["id"], "x");
        assert!(replies[2]["error"].as_str().unwrap().contains("unknown method"));
        assert!(replies[3]["error"].as_str().unwrap().contains("missing `method`"));
        assert_eq!(replies[4]["id"], Value::Null);
        assert!(replies[4]["error"].as_str().unwrap().starts_with("bad JSON"));
    }

    #[test]
    fn poisoned_session_keeps_serving() {
        let h = session();
        let _ = std::thread::scope(|sc| {
            sc.spawn(|| {
                let _g = h.lock().unwrap();
                #[allow(clippy::panic)]
                {
                    panic!("poison the session lock");
                }
            })
            .join()
        });
        assert!(h.is_poisoned());
        let r = respond(&h, r#"{"id":1,"method":"methods"}"#);
        assert_eq!(r["ok"], true, "{r}");
    }

    #[test]
    fn stdio_skips_a_rejected_line_and_keeps_the_session() {
        let h = session();
        let mut input = b"{\"id\":1,\"method\":\"engine.execute\",\"params\":{\"command\":\"settings.set\",\"params\":{\"keepPrevious\":3}}}\n".to_vec();
        input.extend(" ".repeat(MAX_REQUEST_BYTES + 1).into_bytes());
        input.extend(b"{\"id\":9,\"method\":\"methods\"}\n");
        input.extend(b"\xff\xfe{\"id\":8,\"method\":\"methods\"}\n");
        input.extend(b"{\"id\":2,\"method\":\"engine.execute\",\"params\":{\"command\":\"settings.get\"}}\n");
        let mut out = Vec::new();
        serve_lines(&h, input.as_slice(), &mut out).unwrap();
        let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 4, "{replies:?}");
        assert_eq!(replies[0]["ok"], true);
        assert!(replies[1]["error"].as_str().unwrap().contains("request exceeds"), "{}", replies[1]);
        assert!(replies[2]["error"].as_str().unwrap().contains("not valid UTF-8"), "{}", replies[2]);
        assert_eq!(replies[3]["id"], 2);
        assert_eq!(replies[3]["result"]["keepPrevious"], 3, "the session's state stays");
    }

    #[test]
    fn tcp_serves_loopback_after_authentication_only() {
        use std::io::{BufReader, Write as _};
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let h = Arc::new(Mutex::new(Headless::new(Session::new().unwrap())));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = serve_tcp("127.0.0.1:0", h, TOKEN.into(), move |a| tx.send(a).unwrap());
        });
        let addr = rx.recv().unwrap();
        // Before authentication nothing is dispatched, and the connection ends.
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        writeln!(s, r#"{{"id":1,"method":"methods"}}"#).unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!((v["ok"].as_bool(), v["error"].as_str()), (Some(false), Some("authentication required")));
        assert!(v.get("result").is_none());
        // Authenticated: served.
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        writeln!(s, r#"{{"id":"auth","method":"auth","params":{{"token":"{TOKEN}"}}}}"#).unwrap();
        let mut reader = BufReader::new(s.try_clone().unwrap());
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"]["authenticated"], true);
        writeln!(s, r#"{{"id":"a","method":"methods"}}"#).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], "a");
        assert!(v["result"].as_array().unwrap().iter().any(|m| m == "batch"));
        assert!(serve_tcp("0.0.0.0:0", Arc::new(Mutex::new(Headless::new(Session::new().unwrap()))), TOKEN.into(), |_| {}).is_err(), "loopback only");
    }
}
