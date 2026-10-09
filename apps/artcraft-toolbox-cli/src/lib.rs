//! Headless ArtCraft Toolbox: the same engine and commands as the desktop app, for scripts and
//! agents. Exit codes: 0 success, 1 a command failed, 2 a usage error.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::OsString;
use std::io::Write;

use artcraft_toolbox_engine::{Session, build_info, command_specs};
use serde_json::Value;

pub const USAGE: &str = "\
usage: artcraft-toolbox-cli <command> [options]

commands:
  list [--json]                      the Crafting Apps the toolbox manages
  status [--json] [--feed <app>=<releases.json>]...
                                     each app's install and update status; --feed loads a saved
                                     GitHub \"list releases\" response (offline, for scripts and tests)
  commands [--json]                  every engine command with its params
  run <command-id> [<json-params>]   run one engine command and print its JSON result
  --version                          print the version
  --help                             print this help
";

const OK: i32 = 0;
const FAILED: i32 = 1;
const USAGE_ERROR: i32 = 2;

/// Entry point for `main`: arguments as the OS gave them.
pub fn run_os(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut strs = Vec::with_capacity(args.len());
    for a in args {
        match a.to_str() {
            Some(s) => strs.push(s),
            None => {
                let _ = writeln!(err, "error: argument is not valid UTF-8: {}", a.to_string_lossy());
                return USAGE_ERROR;
            }
        }
    }
    run(&strs, out, err)
}

pub fn run(args: &[&str], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let usage = |err: &mut dyn Write, msg: String| {
        let _ = writeln!(err, "error: {msg}\n\n{USAGE}");
        USAGE_ERROR
    };
    let (cmd, rest) = match args {
        [] | ["-h" | "--help" | "help"] => {
            let _ = write!(out, "{USAGE}");
            return OK;
        }
        ["-V" | "--version"] => {
            let _ = writeln!(out, "artcraft-toolbox-cli {}", build_info::long_version());
            return OK;
        }
        [cmd, rest @ ..] => (*cmd, rest),
    };
    if rest.iter().any(|a| matches!(*a, "-h" | "--help")) {
        let _ = write!(out, "{USAGE}");
        return OK;
    }
    let mut session = match Session::new() {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            return FAILED;
        }
    };
    let result = match cmd {
        "list" => flags(rest, &["--json"]).map_err(Usage).and_then(|f| list(&mut session, f.contains(&"--json"), out)),
        "status" => status(&mut session, rest, out),
        "commands" => flags(rest, &["--json"]).map_err(Usage).and_then(|f| commands(f.contains(&"--json"), out)),
        "run" => run_command(&mut session, rest, out),
        other => Err(Usage(format!("unknown command `{}`", short(other)))),
    };
    match result {
        Ok(()) => OK,
        Err(Usage(m)) => usage(err, m),
        Err(Failed(m)) => {
            let _ = writeln!(err, "error: {m}");
            FAILED
        }
    }
}

enum CliError {
    Usage(String),
    Failed(String),
}
use CliError::{Failed, Usage};

fn short(s: &str) -> String {
    s.chars().take(80).collect()
}

/// Only these flags, each at most once.
fn flags<'a>(rest: &[&'a str], allowed: &[&str]) -> Result<Vec<&'a str>, String> {
    let mut seen = Vec::new();
    for a in rest {
        if !allowed.contains(a) {
            return Err(format!("unexpected argument `{}`", short(a)));
        }
        if seen.contains(a) {
            return Err(format!("`{a}` given twice"));
        }
        seen.push(*a);
    }
    Ok(seen)
}

fn print_json(out: &mut dyn Write, v: &Value) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(v).map_err(|e| Failed(e.to_string()))?;
    let _ = writeln!(out, "{text}");
    Ok(())
}

fn exec(session: &mut Session, id: &str, params: Value) -> Result<Value, CliError> {
    session.execute(id, params).map_err(|e| Failed(e.to_string()))
}

fn list(session: &mut Session, json: bool, out: &mut dyn Write) -> Result<(), CliError> {
    let v = exec(session, "catalog.list", Value::Null)?;
    if json {
        return print_json(out, &v);
    }
    for app in v.as_array().into_iter().flatten() {
        let _ = writeln!(out, "{:<12} {:<12} {}", app["id"].as_str().unwrap_or(""), app["name"].as_str().unwrap_or(""), app["tagline"].as_str().unwrap_or(""));
    }
    Ok(())
}

fn status(session: &mut Session, rest: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
    let mut json = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--json" => json = true,
            "--feed" => {
                let spec = it.next().ok_or_else(|| Usage("--feed needs <app>=<releases.json>".into()))?;
                let (app, file) = spec.split_once('=').ok_or_else(|| Usage(format!("--feed `{}` is not <app>=<file>", short(spec))))?;
                let text = std::fs::read_to_string(file).map_err(|e| Failed(format!("{}: {e}", short(file))))?;
                session.ingest_releases(app, &text, 0).map_err(|e| Failed(format!("{app}: {e}")))?;
            }
            other => return Err(Usage(format!("unexpected argument `{}`", short(other)))),
        }
    }
    let rows = session.statuses();
    if json {
        return print_json(out, &serde_json::to_value(&rows).map_err(|e| Failed(e.to_string()))?);
    }
    for r in rows {
        let _ = writeln!(out, "{:<12} {:<12} {}", r.id, r.name, r.status.label());
    }
    Ok(())
}

fn commands(json: bool, out: &mut dyn Write) -> Result<(), CliError> {
    if json {
        let v: Vec<Value> = command_specs().iter().map(|s| serde_json::json!({"id": s.id, "label": s.label, "params": s.params})).collect();
        return print_json(out, &Value::Array(v));
    }
    for s in command_specs() {
        let _ = writeln!(out, "{:<14} {}", s.id, s.params);
    }
    Ok(())
}

fn run_command(session: &mut Session, rest: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
    let (id, params) = match rest {
        [id] => (*id, Value::Null),
        [id, p] => (*id, serde_json::from_str(p).map_err(|e| Usage(format!("params are not JSON: {e}")))?),
        [] => return Err(Usage("run needs a command id (see `commands`)".into())),
        _ => return Err(Usage("run takes a command id and one JSON params argument".into())),
    };
    let v = exec(session, id, params)?;
    print_json(out, &v)
}
