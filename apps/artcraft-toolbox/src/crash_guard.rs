//! Last-resort crash guard (AGENTS.md, Never crash).
//!
//! Code must return errors, never panic. This is the safety net for a panic that escapes anyway:
//! the hook logs it (so it reaches the log a bug report attaches) before the default hook prints
//! it. Commands get their own net in `Session::execute`.

/// Log every panic (with its location) before the default hook prints it.
pub fn install_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("ArtCraft Toolbox internal error: {info}");
        default(info);
    }));
}
