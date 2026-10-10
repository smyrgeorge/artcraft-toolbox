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
use std::sync::{Arc, Mutex};

use artcraft_toolbox_automation::{Headless, ToolboxMcp, rpc, security};

use artcraft_toolbox_engine::install_cmds::{INSTALL, LAUNCH, UNINSTALL, UPDATE};
use artcraft_toolbox_engine::net::Transport;
use artcraft_toolbox_engine::toolbox_cmds::UPDATE as TOOLBOX_UPDATE;
use artcraft_toolbox_engine::update_cmds::{CHECK, CheckSummary};
use artcraft_toolbox_engine::versions_cmds::{ADOPT, ROLLBACK, UPDATE_ALL, VERSIONS};
use artcraft_toolbox_engine::{Layout, SelfInstall, Session, Started, Status, Target, build_info, command_specs, setup, time};
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
  install <app> [--version <x.y.z>] [--json]
                                     download, verify (SHA-256) and install an app; progress on
                                     stderr (check first, so the toolbox knows its releases)
  update <app> [--version <x.y.z>] [--json]
                                     update an installed app (or install another release of it);
                                     the version it replaces is kept for rollback (keepPrevious)
  update --all [--json]              update every app that has an update
  rollback <app> [--version <x.y.z>] [--json]
                                     switch to a kept version (default: the newest older one)
  versions <app> [--json]            the versions installed and kept, and their signatures
  adopt <app> [--json]               take over a copy installed outside the toolbox
  uninstall <app> [--json]           remove an app and its kept versions (its documents stay)
  open <app>                         open an installed app
  self-update [--json]               download and verify a newer ArtCraft Toolbox; it is used
                                     the next time the toolbox starts (`run toolbox.apply` swaps
                                     it in now, while the toolbox isn't running)
  commands [--json]                  every engine command with its params
  run <command-id> [<json-params>]   run one engine command and print its JSON result
  serve [--port <port>] [--control-token <64-hex> | --control-token-file <path>]
                                     keep one headless session open and answer JSON lines
                                     ({\"id\",\"method\",\"params\"}) on stdio, or on 127.0.0.1:<port>
                                     behind a token; methods: engine.execute, engine.commands,
                                     jobs.list, jobs.cancel, batch, methods
                                     (docs/control-protocol.md)
  mcp [--bridge <127.0.0.1:port>] [--control-token <64-hex> | --control-token-file <path>]
                                     MCP server on stdio over the command registry: headless, or
                                     bridged to a running `artcraft-toolbox --control <port>`,
                                     which adds the window's state (docs/mcp.md)
  --version                          print the version
  --help                             print this help

environment:
  ARTCRAFT_TOOLBOX_CONFIG_DIR        data folder (settings, installed apps, cached feeds)
  ARTCRAFT_TOOLBOX_GITHUB_TOKEN      GitHub token: 5,000 API requests per hour instead of 60
  ARTCRAFT_TOOLBOX_CONTROL_TOKEN     the control token, instead of --control-token
  ARTCRAFT_TOOLBOX_CONTROL_TOKEN_FILE
                                     the control token file, instead of --control-token-file
";

const OK: i32 = 0;
const FAILED: i32 = 1;
const USAGE_ERROR: i32 = 2;

/// Where the session lives and how it reaches the network. [`Env::from_process`] is the real
/// one; tests pass a temp folder and a fake transport.
pub struct Env {
    pub data_dir: Result<PathBuf, String>,
    pub transport: Option<Arc<dyn Transport>>,
    /// Where apps are installed; `None`: the platform's (docs/architecture.md § 5).
    pub layout: Option<Layout>,
    /// The computer to pick releases for; `None`: this one. Tests use it to install a Linux
    /// AppImage on any OS.
    pub host: Option<Target>,
    /// The toolbox installation to update (`self-update`); `None`: the one this process runs
    /// from, if any. Tests point it at a temp file.
    pub self_install: Option<SelfInstall>,
}

impl Env {
    pub fn from_process() -> Env {
        let data_dir = artcraft_toolbox_engine::setup::data_dir().map_err(|e| e.to_string());
        let token = std::env::var(setup::ENV_GITHUB_TOKEN).ok();
        Env { data_dir, transport: Some(Arc::new(setup::github_client(token))), layout: None, host: None, self_install: None }
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
    // A bridged MCP server is only a client of the running app: no session, no data folder.
    if let Cmd::Mcp { bridge: Some(addr), token, token_file } = cmd {
        return report(mcp_bridge(&addr, token, token_file), err);
    }
    let env = env();
    let (layout, host, self_install) = (env.layout, env.host, env.self_install);
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
    for change in &opened.changes {
        let _ = writeln!(err, "note: {change}");
    }
    let s = &mut opened.session;
    if layout.is_some() {
        s.set_layout(layout);
    }
    if host.is_some() {
        s.set_host(host);
    }
    if self_install.is_some() {
        s.set_self_install(self_install);
    }
    if s.layout().is_some() {
        // Where apps are may have changed above: look again (apps installed by hand, for adopt).
        for change in s.rescan() {
            let _ = writeln!(err, "note: {change}");
        }
    }
    // The servers own the session for the rest of the process.
    let cmd = match cmd {
        Cmd::Serve { port, token, token_file } => return report(serve(opened.session, port, token, token_file, err), err),
        Cmd::Mcp { token: _, token_file: _, bridge: _ } => return report(mcp_headless(opened.session), err),
        other => other,
    };
    let s = &mut opened.session;
    let result = match cmd {
        Cmd::List { json } => list(s, json, out),
        Cmd::Status { json, feeds } => status(s, json, &feeds, out),
        Cmd::Check { json, app, force } => check(s, json, app, force, out, err),
        Cmd::Install { command, app, version, json } => install(s, command, &app, version, json, out, err),
        Cmd::SelfUpdate { json } => self_update(s, json, out, err),
        Cmd::UpdateAll { json } => update_all(s, json, out, err),
        Cmd::Rollback { app, version, json } => {
            let mut params = json!({ "app": app });
            if let Some(v) = version {
                params["version"] = Value::String(v);
            }
            s.execute(ROLLBACK, params).map_err(|e| e.to_string()).and_then(|v| report_change(out, json, &v, "in use"))
        }
        Cmd::Versions { app, json } => s.execute(VERSIONS, json!({ "app": app })).map_err(|e| e.to_string()).and_then(|v| versions(out, json, &v)),
        Cmd::Adopt { app, json } => s.execute(ADOPT, json!({ "app": app })).map_err(|e| e.to_string()).and_then(|v| {
            if json {
                return print_json(out, &v);
            }
            let adopted: Vec<&str> = v["adopted"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let _ = writeln!(out, "{} adopted: {}", v["app"].as_str().unwrap_or(""), adopted.join(", "));
            Ok(())
        }),
        Cmd::Uninstall { app, json } => {
            s.execute(UNINSTALL, json!({ "app": app })).map_err(|e| e.to_string()).and_then(|v| report_change(out, json, &v, "removed"))
        }
        Cmd::Open { app } => s.execute(LAUNCH, json!({ "app": app })).map(|_| ()).map_err(|e| e.to_string()),
        Cmd::Run { id, params } => s.execute(&id, params).map_err(|e| e.to_string()).and_then(|v| print_json(out, &v)),
        // Answered above, before the session was borrowed.
        Cmd::Commands { .. } | Cmd::Serve { .. } | Cmd::Mcp { .. } => Ok(()),
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
    List {
        json: bool,
    },
    Status {
        json: bool,
        feeds: Vec<(String, String)>,
    },
    Check {
        json: bool,
        app: Option<String>,
        force: bool,
    },
    /// `app.install` or `app.update`, in the background.
    Install {
        command: &'static str,
        app: String,
        version: Option<String>,
        json: bool,
    },
    UpdateAll {
        json: bool,
    },
    Rollback {
        app: String,
        version: Option<String>,
        json: bool,
    },
    Versions {
        app: String,
        json: bool,
    },
    Adopt {
        app: String,
        json: bool,
    },
    Uninstall {
        app: String,
        json: bool,
    },
    Open {
        app: String,
    },
    SelfUpdate {
        json: bool,
    },
    Commands {
        json: bool,
    },
    Run {
        id: String,
        params: Value,
    },
    /// The JSON-lines control protocol on stdio, or on a loopback port behind a token.
    Serve {
        port: Option<u16>,
        token: Option<String>,
        token_file: Option<PathBuf>,
    },
    /// The MCP server on stdio: headless, or bridged to a running desktop app.
    Mcp {
        bridge: Option<String>,
        token: Option<String>,
        token_file: Option<PathBuf>,
    },
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
        "self-update" => {
            for a in rest {
                match *a {
                    "--json" => flag(&mut json, "--json")?,
                    other => return Err(format!("unexpected argument `{}`", short(other))),
                }
            }
            Cmd::SelfUpdate { json }
        }
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
        "update" if rest.first() == Some(&"--all") => {
            for a in rest.iter().skip(1) {
                match *a {
                    "--json" => flag(&mut json, "--json")?,
                    other => return Err(format!("unexpected argument `{}`", short(other))),
                }
            }
            Cmd::UpdateAll { json }
        }
        "install" | "update" | "rollback" | "versions" | "adopt" | "uninstall" | "open" => {
            let (app, rest) = match rest.split_first() {
                Some((a, r)) if !a.starts_with('-') => (a.to_string(), r),
                _ if cmd == "update" => return Err("update needs an app id (see `list`), or --all".into()),
                _ => return Err(format!("{cmd} needs an app id (see `list`)")),
            };
            let mut version = None;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match (cmd, *a) {
                    (c, "--json") if c != "open" => flag(&mut json, "--json")?,
                    ("install" | "update" | "rollback", "--version") => {
                        let v = it.next().ok_or("--version needs a version, like 0.5.0")?;
                        if version.replace(v.to_string()).is_some() {
                            return Err("`--version` given twice".into());
                        }
                    }
                    (_, other) => return Err(format!("unexpected argument `{}`", short(other))),
                }
            }
            match cmd {
                "install" => Cmd::Install { command: INSTALL, app, version, json },
                "update" => Cmd::Install { command: UPDATE, app, version, json },
                "rollback" => Cmd::Rollback { app, version, json },
                "versions" => Cmd::Versions { app, json },
                "adopt" => Cmd::Adopt { app, json },
                "uninstall" => Cmd::Uninstall { app, json },
                _ => Cmd::Open { app },
            }
        }
        "serve" | "mcp" => {
            let (mut port, mut bridge, mut token, mut token_file) = (None, None, None, None);
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match (cmd, *a) {
                    ("serve", "--port") => {
                        let v = it.next().ok_or("--port needs a port number")?;
                        let p: u16 = v.trim().parse().map_err(|_| format!("--port `{}` is not a port (0 to 65535)", short(v)))?;
                        if port.replace(p).is_some() {
                            return Err("`--port` given twice".into());
                        }
                    }
                    ("mcp", "--bridge") => {
                        let v = it.next().ok_or("--bridge needs the app's control address, 127.0.0.1:<port>")?;
                        if bridge.replace(v.to_string()).is_some() {
                            return Err("`--bridge` given twice".into());
                        }
                    }
                    (_, "--control-token") => {
                        let v = it.next().ok_or("--control-token needs the token (64 hex characters)")?;
                        if token.replace(v.to_string()).is_some() {
                            return Err("`--control-token` given twice".into());
                        }
                    }
                    (_, "--control-token-file") => {
                        let v = it.next().ok_or("--control-token-file needs a path")?;
                        if token_file.replace(PathBuf::from(v)).is_some() {
                            return Err("`--control-token-file` given twice".into());
                        }
                    }
                    (_, other) => return Err(format!("unexpected argument `{}`", short(other))),
                }
            }
            if token.is_some() && token_file.is_some() {
                return Err("give --control-token or --control-token-file, not both".into());
            }
            let has_token = token.is_some() || token_file.is_some();
            match cmd {
                "serve" if has_token && port.is_none() => return Err("a control token needs --port (stdio needs no authentication)".into()),
                "serve" => Cmd::Serve { port, token, token_file },
                _ if has_token && bridge.is_none() => return Err("a control token needs --bridge (a headless mcp has no authentication)".into()),
                _ => Cmd::Mcp { bridge, token, token_file },
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

/// `serve`: the control protocol on stdio, or on `127.0.0.1:<port>` behind a token. Where the
/// token comes from goes to stderr, and a generated one with it (the terminal that started
/// the server is the only place it is ever printed).
fn serve(session: Session, port: Option<u16>, token: Option<String>, token_file: Option<PathBuf>, err: &mut dyn Write) -> Result<(), String> {
    let h = Arc::new(Mutex::new(Headless::new(session)));
    let Some(port) = port else {
        let stdin = std::io::stdin();
        return rpc::serve_lines(&h, stdin.lock(), std::io::stdout()).map_err(|e| e.to_string());
    };
    let (supplied, token_file) = security::token_inputs(token, token_file);
    let token = security::server_token(supplied.as_deref(), token_file.as_deref()).map_err(|e| e.to_string())?;
    if let Some(path) = &token_file {
        let _ = writeln!(err, "artcraft-toolbox-cli control token file: {}", path.display());
    } else if supplied.is_none() {
        let _ = writeln!(err, "artcraft-toolbox-cli control token: {token}");
    }
    rpc::serve_tcp(&format!("127.0.0.1:{port}"), h, token, |local| {
        let _ = writeln!(err, "artcraft-toolbox-cli serving on {local}");
    })
    .map_err(|e| e.to_string())
}

/// `mcp` without `--bridge`: the MCP server over an in-process session, on stdio.
fn mcp_headless(session: Session) -> Result<(), String> {
    block_on(ToolboxMcp::headless(session).serve_stdio())
}

/// `mcp --bridge`: the MCP server forwarding to the running desktop app, on stdio.
fn mcp_bridge(addr: &str, token: Option<String>, token_file: Option<PathBuf>) -> Result<(), String> {
    let (supplied, token_file) = security::token_inputs(token, token_file);
    let token = security::client_token(supplied.as_deref(), token_file.as_deref()).map_err(|e| e.to_string())?;
    let server = ToolboxMcp::bridge(addr, &token).map_err(|e| e.to_string())?;
    block_on(server.serve_stdio())
}

fn block_on(f: impl std::future::Future<Output = Result<(), artcraft_toolbox_automation::AutomationError>>) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(f).map_err(|e| e.to_string())
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
        let pinned = r.pinned.map(|v| format!(" · pinned to {v}")).unwrap_or_default();
        let failed = r.error.map(|e| format!("  (last check failed: {e})")).unwrap_or_default();
        let _ = writeln!(out, "{:<12} {:<12} {}{pinned}{failed}", r.id, r.name, r.status.label());
    }
    // The toolbox itself, last.
    if let Some(st) = s.self_status() {
        let line = match (&st.staged, &st.status) {
            (Some(v), _) => format!("{} running · {v} downloaded, used at the next start", st.installed),
            (None, Status::UpdateAvailable { latest, .. }) => format!("{latest} available · {} running", st.installed),
            (None, Status::UpToDate { .. }) => format!("{} · up to date", st.installed),
            (None, _) => format!("{} running", st.installed),
        };
        let failed = st.error.map(|e| format!("  (last check failed: {e})")).unwrap_or_default();
        let _ = writeln!(out, "{:<12} {:<12} {line}{failed}", "toolbox", st.name);
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

/// `installed: /path` (or the JSON result).
fn report_change(out: &mut dyn Write, json: bool, v: &Value, what: &str) -> Result<(), String> {
    if json {
        return print_json(out, v);
    }
    let path = v.get("path").or_else(|| v.get("removed")).and_then(Value::as_str).unwrap_or("");
    let _ = writeln!(out, "{} {} {what}: {path}", v["app"].as_str().unwrap_or(""), v["version"].as_str().unwrap_or(""));
    Ok(())
}

/// Install or update in the background, printing progress to stderr until it ends.
fn install(s: &mut Session, command: &str, app: &str, version: Option<String>, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), String> {
    let mut params = json!({ "app": app });
    if let Some(v) = version {
        params["version"] = Value::String(v);
    }
    let done = if command == UPDATE { "updated" } else { "installed" };
    let v = follow(s, command, app, params, err)?;
    report_change(out, json, &v, done)
}

/// Download and stage a newer toolbox (`toolbox.update`), with the same progress on stderr.
fn self_update(s: &mut Session, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), String> {
    let id = s.toolbox().map(|t| t.id.clone()).ok_or("the catalog names no release feed for the toolbox")?;
    let v = follow(s, TOOLBOX_UPDATE, &id, json!({}), err)?;
    if json {
        return print_json(out, &v);
    }
    let _ = writeln!(
        out,
        "ArtCraft Toolbox {} downloaded and verified: {}\nIt is used the next time ArtCraft Toolbox starts (or run `artcraft-toolbox-cli run toolbox.apply` while it isn't running).",
        v["version"].as_str().unwrap_or(""),
        v["path"].as_str().unwrap_or("")
    );
    Ok(())
}

/// Start a background command for `item` and wait for it, printing its phases to stderr.
fn follow(s: &mut Session, command: &str, item: &str, params: Value, err: &mut dyn Write) -> Result<Value, String> {
    let id = match s.start(command, params).map_err(|e| e.to_string())? {
        Started::Job(id) => id,
        Started::Done(v) => return Ok(v),
    };
    let mut last = String::new();
    loop {
        // The phase before polling: the job's state only moves when `poll_jobs` applies the
        // worker's messages, so the first line is always "Starting", however fast the worker.
        if let Some(job) = s.job_for(command, item) {
            let line = match (job.phase.as_deref(), job.fraction) {
                (Some("Downloading"), Some(f)) => format!("Downloading {:.0}%", f * 100.0),
                (Some(p), _) => p.to_string(),
                _ => String::new(),
            };
            if line != last && !line.is_empty() {
                let _ = write!(err, "\r{line:<24}");
                let _ = err.flush();
                last = line;
            }
        }
        for event in s.poll_jobs() {
            if event.id == id {
                if !last.is_empty() {
                    let _ = writeln!(err);
                }
                return event.result;
            }
        }
        if !s.has_jobs() {
            return Err("the install stopped unexpectedly".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Update every app that has an update and wait for them all; skipped apps are reported on
/// stderr. Fails when an update failed.
fn update_all(s: &mut Session, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), String> {
    let started = s.execute(UPDATE_ALL, json!({})).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let mut failed = 0;
    for job in started["started"].as_array().into_iter().flatten() {
        let (Some(id), app) = (job["job"].as_u64(), job["app"].as_str().unwrap_or("")) else { continue };
        match s.wait_job(artcraft_toolbox_engine::JobId(id)) {
            Ok(v) => results.push(v),
            Err(e) => {
                failed += 1;
                let _ = writeln!(err, "error: {app}: {e}");
            }
        }
    }
    for skipped in started["skipped"].as_array().into_iter().flatten() {
        let _ = writeln!(err, "skipped {}: {}", skipped["app"].as_str().unwrap_or(""), skipped["reason"].as_str().unwrap_or(""));
    }
    if json {
        print_json(out, &json!({"updated": results, "switched": started["switched"], "skipped": started["skipped"]}))?;
    } else {
        for v in &results {
            report_change(out, false, v, "updated")?;
        }
        for v in started["switched"].as_array().into_iter().flatten() {
            let _ = writeln!(out, "{} {} in use (it was kept)", v["app"].as_str().unwrap_or(""), v["version"].as_str().unwrap_or(""));
        }
        if results.is_empty() && started["switched"].as_array().is_none_or(Vec::is_empty) && failed == 0 {
            let _ = writeln!(out, "nothing to update");
        }
    }
    if failed > 0 { Err(format!("{failed} update(s) failed")) } else { Ok(()) }
}

/// `0.5.0  in use  Signed by …  /path`, one line per version.
fn versions(out: &mut dyn Write, json: bool, v: &Value) -> Result<(), String> {
    if json {
        return print_json(out, v);
    }
    for i in v["installed"].as_array().into_iter().flatten() {
        let trust =
            serde_json::from_value::<artcraft_toolbox_engine::Trust>(i["trust"].clone()).map(|t| t.label()).unwrap_or_else(|_| "signature not recorded".into());
        let state = if i["active"].as_bool() == Some(true) { "in use" } else { "kept" };
        let _ = writeln!(out, "{:<10} {state:<7} {trust}  {}", i["version"].as_str().unwrap_or(""), i["path"].as_str().unwrap_or(""));
    }
    for f in v["foundOutsideToolbox"].as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "{:<10} found outside the toolbox (adopt it to manage it)  {}",
            f["version"].as_str().unwrap_or(""),
            f["path"].as_str().unwrap_or("")
        );
    }
    if v["installed"].as_array().is_none_or(Vec::is_empty) && v["foundOutsideToolbox"].as_array().is_none_or(Vec::is_empty) {
        let _ = writeln!(out, "{} isn't installed", v["app"].as_str().unwrap_or(""));
    }
    Ok(())
}
