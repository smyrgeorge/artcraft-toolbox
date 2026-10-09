//! The CLI's contract: output, exit codes, and no panic on bad input.

use std::process::Command;

fn cli(args: &[&str]) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = artcraft_toolbox_cli::run(args, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

const FEED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/feed/tests/fixtures/photocraft-releases.json");

#[test]
fn help_and_version() {
    let (code, out, _) = cli(&[]);
    assert_eq!(code, 0);
    assert!(out.contains("usage: artcraft-toolbox-cli"));
    let (code, out, _) = cli(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("artcraft-toolbox-cli 0."));
    assert_eq!(cli(&["status", "--help"]).0, 0);
}

#[test]
fn list_and_commands() {
    let (code, out, _) = cli(&["list"]);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l.starts_with("photocraft") && l.contains("PhotoCraft")));
    let (code, out, _) = cli(&["list", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(12));
    let (code, out, _) = cli(&["commands", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"apps.status\""));
}

#[test]
fn status_with_an_offline_feed() {
    let feed = format!("photocraft={FEED}");
    let (code, out, err) = cli(&["status", "--json", "--feed", &feed]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let pc = v.as_array().unwrap().iter().find(|r| r["id"] == "photocraft").unwrap();
    // Every CI and dev platform gets a PhotoCraft build.
    assert_eq!(pc["status"], serde_json::json!({"state": "notInstalled", "latest": "0.5.0"}));
    let wc = v.as_array().unwrap().iter().find(|r| r["id"] == "wordcraft").unwrap();
    assert_eq!(wc["status"]["state"], "unknown");
}

#[test]
fn run_dispatches_engine_commands() {
    let (code, out, _) = cli(&["run", "app.status", r#"{"app":"vectorcraft"}"#]);
    assert_eq!(code, 0);
    assert!(out.contains("\"VectorCraft\""));
    let (code, _, err) = cli(&["run", "app.status", r#"{"app":"nope"}"#]);
    assert_eq!(code, 1);
    assert!(err.contains("unknown app"));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec!["nope"],
        vec!["list", "--nope"],
        vec!["list", "--json", "--json"],
        vec!["status", "--feed"],
        vec!["status", "--feed", "photocraft"],
        vec!["run"],
        vec!["run", "app.status", "{not json"],
        vec!["run", "a", "{}", "extra"],
    ] {
        let (code, _, err) = cli(&args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains("usage:"), "{args:?}");
    }
    // A missing feed file is a failure, not a usage error.
    assert_eq!(cli(&["status", "--feed", "photocraft=/definitely/missing.json"]).0, 1);
}

#[test]
fn binary_exit_codes() {
    let exe = env!("CARGO_BIN_EXE_artcraft-toolbox-cli");
    let ok = Command::new(exe).arg("--version").output().unwrap();
    assert!(ok.status.success());
    let bad = Command::new(exe).arg("--bogus").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
}
