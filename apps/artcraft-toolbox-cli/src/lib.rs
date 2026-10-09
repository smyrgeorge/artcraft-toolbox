//! Headless ArtCraft Toolbox: the same engine, commands and data directory as the desktop app,
//! for scripts and agents. Exit codes: 0 success, 1 a command failed (or a check couldn't reach
//! every app), 2 a usage error.
//!
//! Arguments are checked before anything else happens: a usage error never opens the data
//! directory or the network.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use artcraft_toolbox_engine::net::Transport;
use artcraft_toolbox_engine::update_cmds::{CHECK, CheckSummary};
use artcraft_toolbox_engine::{Session, build_info, command_specs, setup, time};
use serde_json::{Value, json};

pub const USAGE: &str = "\
usage: artcraft-toolbox-cli <command> [options]

commands:
  list [--json]                      the Crafting Apps the toolbox manages
  status [--json] [--feed <app>=<releases.json>]...
                                     each app's install and update status, from the last check
                                     (--feed loads a saved GitHub \"list releases\" response instead)
  check [--json] [--app <id>] [--force]
                                     check GitHub for new releases (all apps, or one); apps
                                     checked in the last minute are skipped unless --force
  commands [--json]                  every engine command with its params
  run <command-id> [<json-params>]   run one engine command and print its JSON result
  --version                          print the version
  --help                             print this help

environment:
  ARTCRAFT_TOOLBOX_CONFIG_DIR        data folder (settings, installed apps, cached feeds)
  ARTCRAFT_TOOLBOX_GITHUB_TOKEN      GitHub token: 5,000 API requests per hour instead of 60
";

const OK: i32 = 0;
const FAILED: i32 = 1;
const USAGE_ERROR: i32 = 2;

/// Where the session lives and how it reaches the network. [`Env::from_process`] is the real
/// one; tests pass a temp folder and a fake transport.
pub struct Env {
    pub data_dir: Result<PathBuf, String>,
    pub transport: Option<Arc<dyn Transport>>,
}

impl Env {
    pub fn from_process() -> Env {
        let data_dir = artcraft_toolbox_engine::setup::data_dir().map_err(|e| e.to_string());
        let token = std::env::var(setup::ENV_GITHUB_TOKEN).ok();
        Env { data_dir, transport: Some(Arc::new(setup::github_client(token))) }
    }
}

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
    run(&strs, out, err, Env::from_process)
}

/// Run the CLI. `env` is only called when the command needs the session.
pub fn run(args: &[&str], out: &mut dyn Write, err: &mut dyn Write, env: impl FnOnce() -> Env) -> i32 {
    let cmd = match parse(args) {
        Ok(Parsed::Help) => {
            let _ = write!(out, "{USAGE}");
            return OK;
        }
        Ok(Parsed::Version) => {
            let _ = writeln!(out, "artcraft-toolbox-cli {}", build_info::long_version());
            return OK;
        }
        Ok(Parsed::Cmd(cmd)) => cmd,
        Err(msg) => {
            let _ = writeln!(err, "error: {msg}\n\n{USAGE}");
            return USAGE_ERROR;
        }
    };
    if let Cmd::Commands { json } = cmd {
        return report(commands(json, out), err);
    }
    let env = env();
    let mut opened = match setup::open_in(env.data_dir, env.transport) {
        Ok(o) => o,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            return FAILED;
        }
    };
    for w in &opened.warnings {
        let _ = writeln!(err, "warning: {w}");
    }
    let s = &mut opened.session;
    let result = match cmd {
        Cmd::List { json } => list(s, json, out),
        Cmd::Status { json, feeds } => status(s, json, &feeds, out),
        Cmd::Check { json, app, force } => check(s, json, app, force, out, err),
        Cmd::Run { id, params } => s.execute(&id, params).map_err(|e| e.to_string()).and_then(|v| print_json(out, &v)),
        Cmd::Commands { .. } => Ok(()),
    };
    report(result, err)
}

fn report(result: Result<(), String>, err: &mut dyn Write) -> i32 {
    match result {
        Ok(()) => OK,
        Err(m) => {
            let _ = writeln!(err, "error: {m}");
            FAILED
        }
    }
}

enum Parsed {
    Help,
    Version,
    Cmd(Cmd),
}

enum Cmd {
    List { json: bool },
    Status { json: bool, feeds: Vec<(String, String)> },
    Check { json: bool, app: Option<String>, force: bool },
    Commands { json: bool },
    Run { id: String, params: Value },
}

fn short(s: &str) -> String {
    s.chars().take(80).collect()
}

/// Every usage error is found here, before anything runs.
fn parse(args: &[&str]) -> Result<Parsed, String> {
    let (cmd, rest) = match args {
        [] | ["-h" | "--help" | "help"] => return Ok(Parsed::Help),
        ["-V" | "--version"] => return Ok(Parsed::Version),
        [cmd, rest @ ..] => (*cmd, rest),
    };
    if rest.iter().any(|a| matches!(*a, "-h" | "--help")) {
        return Ok(Parsed::Help);
    }
    let mut json = false;
    let mut feeds = Vec::new();
    let mut app = None;
    let mut force = false;
    let flag = |seen: &mut bool, name: &str| if std::mem::replace(seen, true) { Err(format!("`{name}` given twice")) } else { Ok(()) };
    let cmd = match cmd {
        "list" | "commands" | "status" | "check" => {
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match (cmd, *a) {
                    (_, "--json") => flag(&mut json, "--json")?,
                    ("status", "--feed") => {
                        let spec = it.next().ok_or("--feed needs <app>=<releases.json>")?;
                        let (a, f) = spec.split_once('=').ok_or_else(|| format!("--feed `{}` is not <app>=<file>", short(spec)))?;
                        feeds.push((a.to_string(), f.to_string()));
                    }
                    ("check", "--app") => {
                        let id = it.next().ok_or("--app needs an app id (see `list`)")?;
                        if app.replace(id.to_string()).is_some() {
                            return Err("`--app` given twice (check one app, or all)".into());
                        }
                    }
                    ("check", "--force") => flag(&mut force, "--force")?,
                    (_, other) => return Err(format!("unexpected argument `{}`", short(other))),
                }
            }
            match cmd {
                "list" => Cmd::List { json },
                "commands" => Cmd::Commands { json },
                "status" => Cmd::Status { json, feeds },
                _ => Cmd::Check { json, app, force },
            }
        }
        "run" => match rest {
            [id] => Cmd::Run { id: id.to_string(), params: Value::Null },
            [id, p] => Cmd::Run { id: id.to_string(), params: serde_json::from_str(p).map_err(|e| format!("params are not JSON: {e}"))? },
            [] => return Err("run needs a command id (see `commands`)".into()),
            _ => return Err("run takes a command id and one JSON params argument".into()),
        },
        other => return Err(format!("unknown command `{}`", short(other))),
    };
    Ok(Parsed::Cmd(cmd))
}

fn print_json(out: &mut dyn Write, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    let _ = writeln!(out, "{text}");
    Ok(())
}

fn list(s: &mut Session, json: bool, out: &mut dyn Write) -> Result<(), String> {
    let v = s.execute("catalog.list", Value::Null).map_err(|e| e.to_string())?;
    if json {
        return print_json(out, &v);
    }
    for app in v.as_array().into_iter().flatten() {
        let _ = writeln!(out, "{:<12} {:<12} {}", app["id"].as_str().unwrap_or(""), app["name"].as_str().unwrap_or(""), app["tagline"].as_str().unwrap_or(""));
    }
    Ok(())
}

fn status(s: &mut Session, json: bool, feeds: &[(String, String)], out: &mut dyn Write) -> Result<(), String> {
    for (app, file) in feeds {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", short(file)))?;
        s.ingest_releases(app, &text, s.now()).map_err(|e| format!("{app}: {e}"))?;
    }
    print_statuses(s, json, out)
}

fn print_statuses(s: &Session, json: bool, out: &mut dyn Write) -> Result<(), String> {
    let rows = s.statuses();
    if json {
        return print_json(out, &serde_json::to_value(&rows).map_err(|e| e.to_string())?);
    }
    for r in rows {
        let failed = r.error.map(|e| format!("  (last check failed: {e})")).unwrap_or_default();
        let _ = writeln!(out, "{:<12} {:<12} {}{failed}", r.id, r.name, r.status.label());
    }
    let _ = match s.last_checked() {
        Some(at) => writeln!(out, "\nChecked {}.", time::ago(s.now(), at)),
        None => writeln!(out, "\nNot checked yet: run `artcraft-toolbox-cli check`."),
    };
    Ok(())
}

/// Exit 1 when the check couldn't reach every app it tried (the statuses still print).
fn check(s: &mut Session, json: bool, app: Option<String>, force: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), String> {
    let mut params = json!({});
    if let Some(id) = app {
        params["app"] = Value::String(id);
    }
    if force {
        params["force"] = Value::Bool(true);
    }
    let result = s.execute(CHECK, params).map_err(|e| e.to_string())?;
    let summary: CheckSummary = serde_json::from_value(result.clone()).map_err(|e| e.to_string())?;
    if json {
        print_json(out, &json!({"check": result, "apps": s.statuses()}))?;
    } else {
        print_statuses(s, false, out)?;
    }
    match s.check_notice(&result) {
        Some(notice) => {
            let _ = writeln!(err, "{notice}");
            Err(format!("{} of {} apps couldn't be checked", summary.failed.len() + summary.skipped.len(), summary.done()))
        }
        None => Ok(()),
    }
}

fn commands(json: bool, out: &mut dyn Write) -> Result<(), String> {
    if json {
        let v: Vec<Value> =
            command_specs().iter().map(|s| json!({"id": s.id, "label": s.label, "params": s.params, "background": s.start.is_some()})).collect();
        return print_json(out, &Value::Array(v));
    }
    for s in command_specs() {
        let _ = writeln!(out, "{:<14} {}", s.id, s.params);
    }
    Ok(())
}
