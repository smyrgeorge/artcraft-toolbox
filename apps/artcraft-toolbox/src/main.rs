//! ArtCraft Toolbox desktop app.
//!
//! Usage: `artcraft-toolbox [--control <port>] [--control-token <64-hex> | --control-token-file <path>]
//! [--version] [--help]`
//!
//! `--control <port>` (or `ARTCRAFT_TOOLBOX_CONTROL_PORT`) starts a localhost JSON-lines control
//! server for agents and tests (`docs/control-protocol.md`): one request per line
//! (`{"id":1,"method":"ui.get","params":{}}`), one reply per line, the first frame a bearer
//! token. `artcraft-toolbox-cli mcp --bridge 127.0.0.1:<port>` wraps it as an MCP server.

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--version` output then only shows when redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control_server;
mod crash_guard;
mod logging;
mod notify;
mod popover;
mod tray;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use artcraft_toolbox_engine::{build_info, setup};
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use popover::Popover;
use tray::{Tray, TrayAction};

/// Matches the `.desktop` file and hicolor icon name (packaging, docs/roadmap.md M5), so Wayland
/// docks pick up the icon.
const APP_ID: &str = "ai.storyteller.toolbox";

const USAGE: &str = "usage: artcraft-toolbox [--control <port>] [--control-token <64-hex> | --control-token-file <path>] [--version] [--help]";

/// The control server's settings (`docs/control-protocol.md`).
#[derive(Debug, Default, PartialEq, Eq)]
struct Control {
    /// `--control <port>` or `ARTCRAFT_TOOLBOX_CONTROL_PORT`; none: no server.
    port: Option<u16>,
    token: Option<String>,
    token_file: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Run(Control),
    Version,
    Help,
}

/// A port, naming a bad value: a launcher must learn that its control server didn't start.
fn parse_control_port(value: &str, source: &str) -> Result<u16, String> {
    value.trim().parse::<u16>().map_err(|_| format!("{source}: `{}` is not a port (0 to 65535)", value.chars().take(40).collect::<String>()))
}

/// `args_os`, not `args`: a non-Unicode argument must be a usage error, not a panic.
fn parse_args(args: &[OsString], env_port: Option<&str>) -> Result<Action, String> {
    let mut control = Control::default();
    if let Some(v) = env_port.filter(|v| !v.trim().is_empty()) {
        control.port = Some(parse_control_port(v, "ARTCRAFT_TOOLBOX_CONTROL_PORT")?);
    }
    let mut action = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let text = a.to_str().ok_or_else(|| format!("argument is not valid UTF-8: {}", a.to_string_lossy()))?;
        let value = |it: &mut std::slice::Iter<'_, OsString>, flag: &str| -> Result<String, String> {
            let v = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            v.to_str().map(str::to_string).ok_or_else(|| format!("{flag}: value is not valid UTF-8"))
        };
        match text {
            "--version" | "-V" => action = Some(Action::Version),
            "--help" | "-h" => action = Some(Action::Help),
            "--control" => control.port = Some(parse_control_port(&value(&mut it, "--control")?, "--control")?),
            "--control-token" => control.token = Some(value(&mut it, "--control-token")?),
            "--control-token-file" => control.token_file = Some(PathBuf::from(value(&mut it, "--control-token-file")?)),
            other => return Err(format!("unknown argument `{}`", other.chars().take(80).collect::<String>())),
        }
    }
    Ok(action.unwrap_or(Action::Run(control)))
}

/// The window icon (`assets/app-icon/`, rendered by `packaging/icons.sh`).
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/artcraft-toolbox-256.png");

fn window_icon() -> Option<std::sync::Arc<egui::IconData>> {
    match artcraft_toolbox_engine::icons::decode_png(WINDOW_ICON_PNG) {
        Ok((width, height, rgba)) => Some(std::sync::Arc::new(egui::IconData { rgba, width, height })),
        Err(e) => {
            log::warn!("window icon: {e}");
            None
        }
    }
}

/// The popover's window is transparent outside its rounded edge where the renderer can show
/// through (Metal); elsewhere its edge is square.
const TRANSPARENT: bool = cfg!(target_os = "macos");

/// The main window. A popover (macOS, Windows): no title bar, above other windows, out of the
/// taskbar, hidden until placed under the tray icon; on macOS a menu-bar app, without a Dock icon.
/// Otherwise (Linux): a normal window, compact and tall like a launcher, centred.
fn native_options(popover: bool) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default().with_app_id(APP_ID).with_title("ArtCraft Toolbox");
    viewport = if popover {
        viewport
            .with_inner_size(popover::SIZE)
            .with_position(popover::PARKED)
            .with_decorations(false)
            .with_transparent(TRANSPARENT)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_taskbar(false)
    } else {
        viewport.with_inner_size([440.0, 720.0]).with_min_inner_size([360.0, 480.0])
    };
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, centered: !popover, ..Default::default() };
    #[cfg(target_os = "macos")]
    if popover {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder.with_activation_policy(ActivationPolicy::Accessory);
        }));
    }
    options
}

/// Back to a normal window (the tray icon couldn't be made, so nothing would show the popover).
fn as_window(ctx: &egui::Context) {
    for cmd in [
        egui::ViewportCommand::Decorations(true),
        egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal),
        egui::ViewportCommand::Resizable(true),
        egui::ViewportCommand::InnerSize(egui::vec2(440.0, 720.0)),
        egui::ViewportCommand::Visible(true),
        egui::ViewportCommand::Focus,
    ] {
        ctx.send_viewport_cmd(cmd);
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let env_port = std::env::var("ARTCRAFT_TOOLBOX_CONTROL_PORT").ok();
    let control = match parse_args(&args, env_port.as_deref()) {
        Ok(Action::Run(control)) => control,
        Ok(Action::Version) => {
            println!("ArtCraft Toolbox {}", build_info::long_version());
            return ExitCode::SUCCESS;
        }
        Ok(Action::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let logger = logging::install();
    crash_guard::install_hook();
    log::info!("ArtCraft Toolbox {} starting", build_info::long_version());
    // The control server's token, before anything else: a bad one is a usage error.
    let control = match control.port {
        None => None,
        Some(port) => {
            use artcraft_toolbox_automation::security;
            let (supplied, token_file) = security::token_inputs(control.token, control.token_file);
            let token = match security::server_token(supplied.as_deref(), token_file.as_deref()) {
                Ok(token) => token,
                Err(e) => {
                    eprintln!("error: cannot configure control authentication: {e}\n{USAGE}");
                    return ExitCode::from(2);
                }
            };
            // Where the token is, never the token itself in the log; a generated one goes to
            // standard error only, for the terminal that started the app.
            match (&token_file, supplied.is_some()) {
                (Some(path), _) => log::info!("control token file: {}", path.display()),
                (None, true) => log::info!("using the supplied control token"),
                (None, false) => eprintln!("artcraft-toolbox: control token: {token}"),
            }
            Some((port, token))
        }
    };
    let mut opened = match setup::open_user_session() {
        Ok(o) => o,
        Err(e) => {
            log::error!("{e}");
            eprintln!("ArtCraft Toolbox could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    // A newer toolbox downloaded earlier (`toolbox.update`): put it in place and start it
    // instead of this version (docs/architecture.md § 7).
    match opened.session.apply_staged_update(true) {
        Ok(Some(applied)) => match applied.relaunch() {
            Ok(()) => {
                log::info!("ArtCraft Toolbox {} starts in place of this version", applied.version);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                log::error!("ArtCraft Toolbox {} is in place but couldn't be started: {e}", applied.version);
                opened.warnings.push(format!("ArtCraft Toolbox {} is installed; start it again to use it ({e})", applied.version));
            }
        },
        Ok(None) => opened.session.clean_previous_self(),
        Err(e) => {
            log::warn!("the downloaded toolbox update wasn't applied: {e}");
            opened.warnings.push(e.to_string());
        }
    }
    if let (Some(logger), Some(store)) = (logger, opened.session.store()) {
        match logger.attach_dir(&store.logs_dir()) {
            Ok(path) => log::info!("logging to {}", path.display()),
            Err(e) => log::warn!("no log file: {e}"),
        }
    }
    for w in &opened.warnings {
        log::warn!("{w}");
    }
    for change in &opened.changes {
        log::info!("{change}");
    }
    let (session, warning) = (opened.session, opened.warnings.into_iter().next());
    let popover = popover::SUPPORTED;
    let result = eframe::run_native(
        "ArtCraft Toolbox",
        native_options(popover),
        Box::new(move |cc| {
            ToolboxApp::setup_context(&cc.egui_ctx);
            // The tray's labels are made now: in the saved language.
            artcraft_toolbox_ui_egui::i18n::set_current(artcraft_toolbox_ui_egui::i18n::Lang::from_pref(&session.settings().language));
            let ctx = cc.egui_ctx.clone();
            let tray = match Tray::new(popover, move || ctx.request_repaint()) {
                Ok(t) => {
                    log::info!("menu-bar/tray icon ready");
                    Some(t)
                }
                Err(e) => {
                    log::warn!("no menu-bar or tray icon: {e}; closing the window quits");
                    None
                }
            };
            // A popover needs its icon: without one, a normal window.
            let popover = popover && tray.is_some();
            if popover::SUPPORTED && !popover {
                as_window(&cc.egui_ctx);
            }
            let services = Services { notify: Some(Box::new(notify::show)), tray: tray.is_some(), popover, transparent: popover && TRANSPARENT };
            let mut app = ToolboxApp::with_services(session, services);
            if let Some((port, token)) = control {
                app = app.with_control(control_server::start(port, token, cc.egui_ctx.clone()));
            }
            app.background();
            // A file that couldn't be read matters more than "checking started".
            if warning.is_some() {
                app.notice = warning;
            }
            let popover = popover.then(Popover::starting);
            Ok(Box::new(Desktop { app, tray, popover }))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("window: {e}");
            eprintln!("ArtCraft Toolbox could not open its window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The toolbox in its window, plus the tray icon.
struct Desktop {
    app: ToolboxApp,
    tray: Option<Tray>,
    popover: Option<Popover>,
}

impl Desktop {
    fn quit(&mut self, ctx: &egui::Context) {
        self.app.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// `Restart to update`: swap the downloaded toolbox version in, start it, and quit.
    fn restart_into_update(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.app.restart_wanted) {
            return;
        }
        match self.app.session.apply_staged_update(true) {
            Ok(Some(applied)) => match applied.relaunch() {
                Ok(()) => {
                    log::info!("ArtCraft Toolbox {} starts; quitting this version", applied.version);
                    self.quit(ctx);
                }
                Err(e) => self.app.notice = Some(format!("ArtCraft Toolbox {} is installed; start it again to use it ({e})", applied.version)),
            },
            Ok(None) => self.app.notice = Some("no ArtCraft Toolbox update is waiting".into()),
            Err(e) => self.app.notice = Some(e.to_string()),
        }
    }

    /// A popover hides on losing the focus and on Escape (unless Escape is for something in it:
    /// the search, a dialog, an open menu); Cmd+Q quits (a menu-bar app has no app menu).
    fn popover_keys_and_focus(&mut self, ctx: &egui::Context) {
        let Some(popover) = &mut self.popover else { return };
        popover.start(ctx, self.tray.as_ref().and_then(Tray::rect));
        // A close request (Alt+F4) hides it to the tray, or quits (`ToolboxApp::tick`).
        if ctx.input(|i| i.viewport().close_requested()) {
            popover.hidden();
        }
        popover.refine(ctx);
        popover.follow_focus(ctx);
        let busy = self.app.ui.search_open || self.app.ui.confirm_uninstall.is_some() || egui::Popup::is_any_open(ctx);
        if popover.shown() && !busy && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            popover.hide(ctx);
        }
        let quit = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Q);
        if ctx.input_mut(|i| i.consume_shortcut(&quit)) {
            self.quit(ctx);
        }
    }
}

impl eframe::App for Desktop {
    /// Also runs while the window is hidden (after a tray click wakes it, or the idle tick).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for action in self.tray.as_ref().map(Tray::actions).unwrap_or_default() {
            match (action, &mut self.popover) {
                (TrayAction::Toggle(rect), Some(popover)) => popover.toggle(ctx, Some(&rect)),
                (TrayAction::Open, Some(popover)) => {
                    let rect = self.tray.as_ref().and_then(Tray::rect);
                    popover.show(ctx, rect.as_ref());
                }
                (TrayAction::Open | TrayAction::Toggle(_), None) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                (TrayAction::Check, _) => self.app.start_check(),
                (TrayAction::Quit, _) => self.quit(ctx),
            }
        }
        self.popover_keys_and_focus(ctx);
        self.app.tick(ctx);
        self.restart_into_update(ctx);
        if let Some(tray) = &mut self.tray {
            tray.relabel();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.show(ui);
    }

    /// A transparent popover shows through outside its rounded edge.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.app, visuals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_icon_decodes() {
        let icon = window_icon().unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn arguments() {
        assert_eq!(parse_args(&[], None), Ok(Action::Run(Control::default())));
        assert_eq!(parse_args(&os(&["--version"]), None), Ok(Action::Version));
        assert_eq!(parse_args(&os(&["-h"]), None), Ok(Action::Help));
        assert!(parse_args(&os(&["--nope"]), None).unwrap_err().contains("unknown argument"));
        // The control server: port from the argument or the environment, token or token file.
        let run = parse_args(&os(&["--control", "7878", "--control-token-file", "/private/t"]), None).unwrap();
        assert_eq!(run, Action::Run(Control { port: Some(7878), token: None, token_file: Some(PathBuf::from("/private/t")) }));
        assert_eq!(parse_args(&[], Some("50494")).unwrap(), Action::Run(Control { port: Some(50494), ..Control::default() }));
        assert_eq!(parse_args(&os(&["--control", "1"]), Some("2")).unwrap(), Action::Run(Control { port: Some(1), ..Control::default() }));
        assert!(parse_args(&[], Some("  ")).is_ok(), "an empty variable is no port");
        for bad in [&["--control", "nope"][..], &["--control", "78787"], &["--control"], &["--control-token"], &["--control-token-file"]] {
            assert!(parse_args(&os(bad), None).is_err(), "{bad:?}");
        }
        assert!(parse_args(&[], Some("x")).unwrap_err().contains("ARTCRAFT_TOOLBOX_CONTROL_PORT"));
        assert_eq!(parse_args(&os(&["--control", "7878", "--version"]), None), Ok(Action::Version));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_argument_is_an_error() {
        use std::os::unix::ffi::OsStringExt;
        assert!(parse_args(&[OsString::from_vec(vec![0x66, 0xff])], None).unwrap_err().contains("UTF-8"));
    }
}
