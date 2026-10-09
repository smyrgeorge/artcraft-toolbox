//! Fuzz every command with adversarial params: a command must return `Err`, never panic or hang
//! (AGENTS.md, Never crash). Each call runs on its own thread with a timeout. Fast enough to run
//! on every `cargo test` while the registry is small; mark it `#[ignore]` if it ever isn't.

use std::sync::mpsc;
use std::time::Duration;

use artcraft_toolbox_engine::{Session, command_specs};
use serde_json::{Value, json};

fn adversarial() -> Vec<Value> {
    let long = "x".repeat(100_000);
    let mut out = vec![
        Value::Null,
        json!({}),
        json!([]),
        json!(1),
        json!("photocraft"),
        json!({"app": null}),
        json!({"app": 1}),
        json!({"app": ""}),
        json!({"app": long}),
        json!({"app": "../../etc/passwd"}),
        json!({"app": "photocraft\u{0}"}),
        json!({"app": "ΦωτοCraft"}),
        json!({"app": ["photocraft"]}),
        json!({"app": {"id": "photocraft"}}),
        json!({"channel": "nightly"}),
        json!({"checkIntervalHours": -1}),
        json!({"checkIntervalHours": 1e308}),
        json!({"keepPrevious": u64::MAX}),
        json!({"installDir": ""}),
        json!({"installDir": "\u{0}"}),
        json!({"autoUpdate": "true"}),
    ];
    out.push(json!({ (long.clone()): 1 }));
    out
}

#[test]
fn no_command_panics_or_hangs_on_adversarial_params() {
    let mut failures = Vec::new();
    for spec in command_specs() {
        for p in adversarial() {
            let id = spec.id;
            let (tx, rx) = mpsc::channel();
            let p2 = p.clone();
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(|| {
                    let mut s = Session::new().unwrap();
                    let _ = s.execute(id, p2);
                });
                let _ = tx.send(r.is_ok());
            });
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(true) => {}
                Ok(false) => failures.push(format!("{id} panicked on {}", short(&p))),
                Err(_) => failures.push(format!("{id} hung on {}", short(&p))),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn unknown_and_malformed_calls_are_errors() {
    let mut s = Session::new().unwrap();
    assert!(s.execute("nope.nope", json!({})).unwrap_err().to_string().contains("unknown command"));
    assert!(s.execute("catalog.list", json!([1])).unwrap_err().to_string().contains("JSON object"));
    let msg = s.execute(&"x".repeat(10_000), json!({})).unwrap_err().to_string();
    assert!(msg.len() < 200);
}

fn short(v: &Value) -> String {
    v.to_string().chars().take(60).collect()
}
