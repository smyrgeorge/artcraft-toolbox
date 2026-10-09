//! OS notifications (notify-rust: Notification Center, Windows toasts, D-Bus on Linux), sent
//! off the UI thread because some platforms block while delivering. A failure is logged, never
//! shown: a notification is a courtesy.
//!
//! macOS attributes notifications to the sending app's bundle; an unbundled development build is
//! attributed to another app, a packaged one (roadmap M5) to ArtCraft Toolbox.

pub fn show(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let spawned = std::thread::Builder::new().name("notify".into()).spawn(move || {
        let sent = notify_rust::Notification::new().appname("ArtCraft Toolbox").summary(&title).body(&body).show();
        match sent {
            Ok(_) => log::info!("notified: {title}: {body}"),
            Err(e) => log::warn!("notification not shown: {e}"),
        }
    });
    if let Err(e) = spawned {
        log::warn!("notification not shown: {e}");
    }
}
