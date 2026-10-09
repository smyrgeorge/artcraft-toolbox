//! ArtCraft Toolbox desktop app.
//!
//! Usage: `artcraft-toolbox [--version] [--help]`

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--version` output then only shows when redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod crash_guard;
mod logging;
mod notify;
mod tray;

use std::ffi::OsString;
use std::process::ExitCode;

use artcraft_toolbox_engine::{build_info, setup};
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use tray::{Tray, TrayAction};

/// Matches the `.desktop` file and hicolor icon name (packaging, docs/roadmap.md M5), so Wayland
/// docks pick up the icon.
const APP_ID: &str = "ai.storyteller.toolbox";

const USAGE: &str = "usage: artcraft-toolbox [--version] [--help]";

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Run,
    Version,
    Help,
}

/// `args_os`, not `args`: a non-Unicode argument must be a usage error, not a panic.
fn parse_args(args: &[OsString]) -> Result<Action, String> {
    let mut action = Action::Run;
    for a in args {
        match a.to_str() {
            Some("--version" | "-V") => action = Action::Version,
            Some("--help" | "-h") => action = Action::Help,
            Some(other) => return Err(format!("unknown argument `{}`", other.chars().take(80).collect::<String>())),
            None => return Err(format!("argument is not valid UTF-8: {}", a.to_string_lossy())),
        }
    }
    Ok(action)
}

/// The window icon (placeholder art until M5's real icon).
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

/// The main window: compact and tall, like a launcher, centred on the main monitor.
fn native_options() -> eframe::NativeOptions {
    let mut viewport =
        egui::ViewportBuilder::default().with_app_id(APP_ID).with_title("ArtCraft Toolbox").with_inner_size([440.0, 720.0]).with_min_inner_size([360.0, 480.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::NativeOptions { viewport, centered: true, ..Default::default() }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match parse_args(&args) {
        Ok(Action::Run) => {}
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
    }
    let logger = logging::install();
    crash_guard::install_hook();
    log::info!("ArtCraft Toolbox {} starting", build_info::long_version());
    let opened = match setup::open_user_session() {
        Ok(o) => o,
        Err(e) => {
            log::error!("{e}");
            eprintln!("ArtCraft Toolbox could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let (Some(logger), Some(store)) = (logger, opened.session.store()) {
        match logger.attach_dir(&store.logs_dir()) {
            Ok(path) => log::info!("logging to {}", path.display()),
            Err(e) => log::warn!("no log file: {e}"),
        }
    }
    for w in &opened.warnings {
        log::warn!("{w}");
    }
    let (session, warning) = (opened.session, opened.warnings.into_iter().next());
    let result = eframe::run_native(
        "ArtCraft Toolbox",
        native_options(),
        Box::new(move |cc| {
            ToolboxApp::setup_context(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let tray = match Tray::new(move || ctx.request_repaint()) {
                Ok(t) => {
                    log::info!("menu-bar/tray icon ready");
                    Some(t)
                }
                Err(e) => {
                    log::warn!("no menu-bar or tray icon: {e}; closing the window quits");
                    None
                }
            };
            let services = Services { notify: Some(Box::new(notify::show)), tray: tray.is_some() };
            let mut app = ToolboxApp::with_services(session, services);
            app.background();
            // A file that couldn't be read matters more than "checking started".
            if warning.is_some() {
                app.notice = warning;
            }
            Ok(Box::new(Desktop { app, tray }))
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
}

impl eframe::App for Desktop {
    /// Also runs while the window is hidden (after a tray click wakes it, or the idle tick).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for action in self.tray.as_ref().map(Tray::actions).unwrap_or_default() {
            match action {
                TrayAction::Open => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::Check => self.app.start_check(),
                TrayAction::Quit => {
                    self.app.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        self.app.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.show(ui);
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
        assert_eq!(parse_args(&[]), Ok(Action::Run));
        assert_eq!(parse_args(&os(&["--version"])), Ok(Action::Version));
        assert_eq!(parse_args(&os(&["-h"])), Ok(Action::Help));
        assert!(parse_args(&os(&["--nope"])).unwrap_err().contains("unknown argument"));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_argument_is_an_error() {
        use std::os::unix::ffi::OsStringExt;
        assert!(parse_args(&[OsString::from_vec(vec![0x66, 0xff])]).unwrap_err().contains("UTF-8"));
    }
}
