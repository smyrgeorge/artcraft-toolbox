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
mod popover;
mod tray;

use std::ffi::OsString;
use std::process::ExitCode;

use artcraft_toolbox_engine::{build_info, setup};
use artcraft_toolbox_ui_egui::{Services, ToolboxApp};
use popover::Popover;
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
