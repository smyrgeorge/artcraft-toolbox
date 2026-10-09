//! ArtCraft Toolbox desktop app.
//!
//! Usage: `artcraft-toolbox [--version] [--help]`

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--version` output then only shows when redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod crash_guard;
mod logging;

use std::ffi::OsString;
use std::process::ExitCode;

use artcraft_toolbox_engine::{Session, build_info};
use artcraft_toolbox_ui_egui::ToolboxApp;

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

/// The main window: compact and tall, like a launcher, centred on the main monitor.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title("ArtCraft Toolbox")
            .with_inner_size([440.0, 720.0])
            .with_min_inner_size([360.0, 480.0]),
        centered: true,
        ..Default::default()
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
    logging::init();
    crash_guard::install_hook();
    log::info!("ArtCraft Toolbox {} starting", build_info::long_version());
    let session = match Session::new() {
        Ok(s) => s,
        Err(e) => {
            log::error!("{e}");
            eprintln!("ArtCraft Toolbox could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = eframe::run_native(
        "ArtCraft Toolbox",
        native_options(),
        Box::new(move |cc| {
            ToolboxApp::setup_context(&cc.egui_ctx);
            Ok(Box::new(ToolboxApp::new(session)))
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

#[cfg(test)]
mod tests {
    use super::*;

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
