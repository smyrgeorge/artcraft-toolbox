//! A native alert for a start that failed before any window showed (AGENTS.md, Never crash: an
//! error the user can act on). Opened from Finder, the toolbox has no terminal for its standard
//! error and, as a menu-bar app, no Dock icon: a Mac whose graphics couldn't draw the window (no
//! Metal, as on some Macs running macOS through OpenCore Legacy Patcher) saw nothing at all.
//! macOS only for now; elsewhere the error is in the log and on standard error, as before.

use std::path::Path;

use artcraft_toolbox_ui_egui::wording;

/// The longest excerpt of the error shown (the log has all of it).
const MAX_ERROR_CHARS: usize = 400;

/// Tell the user the toolbox couldn't start, and wait until they dismiss it. `graphics`: the
/// renderer couldn't start. In the current UI language (`i18n::set_current`), on this thread.
pub fn startup_failed(error: &str, graphics: bool, log: Option<&Path>) {
    let error: String = error.chars().take(MAX_ERROR_CHARS).collect();
    let log = log.map(|p| p.display().to_string());
    let (title, message) = wording::startup_failure(&error, graphics, log.as_deref());
    show(&title, &message);
}

#[cfg(target_os = "macos")]
fn show(title: &str, message: &str) {
    match std::process::Command::new("/usr/bin/osascript").args(osascript_args(title, message)).status() {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("the alert didn't show (osascript: {status})"),
        Err(e) => log::warn!("the alert didn't show: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn show(_title: &str, _message: &str) {}

/// `osascript`'s arguments: a fixed script, then the texts as its arguments (`argv`), never
/// spliced into the script.
#[cfg(any(target_os = "macos", test))]
fn osascript_args<'a>(title: &'a str, message: &'a str) -> [&'a str; 8] {
    ["-e", "on run argv", "-e", "display alert (item 1 of argv) message (item 2 of argv) as critical", "-e", "end run", title, message]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texts_are_arguments_not_script() {
        let hostile = "x\" & (do shell script \"touch /tmp/pwned\") & \"";
        let args = osascript_args(hostile, "line 1\nline 2");
        assert_eq!(&args[6..], [hostile, "line 1\nline 2"]);
        assert!(args[..6].iter().all(|a| !a.contains("pwned")));
    }

    #[test]
    fn the_message_names_the_graphics_and_the_log() {
        artcraft_toolbox_ui_egui::i18n::set_current(artcraft_toolbox_ui_egui::i18n::Lang::EN);
        let (title, message) = wording::startup_failure("WGPU error: no adapter", true, Some("/logs/artcraft-toolbox.log"));
        assert_eq!(title, "ArtCraft Toolbox couldn't start");
        assert!(message.starts_with("Metal graphics aren't available on this Mac"), "{message}");
        assert!(message.contains("WGPU error: no adapter") && message.ends_with("Details are in the log: /logs/artcraft-toolbox.log"), "{message}");
        let (_, plain) = wording::startup_failure("no display", false, None);
        assert_eq!(plain, "no display");
    }
}
