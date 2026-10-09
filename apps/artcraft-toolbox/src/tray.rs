//! The menu-bar (macOS) or system-tray (Windows, Linux) icon: Open, Check for Updates, Quit.
//!
//! tray-icon delivers clicks through global handlers, on whatever thread the platform uses. They
//! only queue an action and wake the app (`wake`, which requests a repaint); the app applies the
//! actions in `eframe::App::logic`, which runs even while the window is hidden. Creating the icon
//! can fail (a Linux desktop without a StatusNotifier host, for example): the app then runs
//! without one, and closing the window quits as usual.

use std::sync::mpsc::{Receiver, channel};

use artcraft_toolbox_ui_egui::{i18n, wording};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// The macOS menu-bar glyph: a template image (only alpha counts; macOS tints it).
const TEMPLATE_PNG: &[u8] = include_bytes!("../../../assets/app-icon/tray-template-44.png");
/// The colour icon for Windows and Linux trays.
const COLOR_PNG: &[u8] = include_bytes!("../../../assets/app-icon/tray-64.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    Check,
    Quit,
}

pub struct Tray {
    /// Removed from the menu bar when dropped.
    _icon: TrayIcon,
    actions: Receiver<TrayAction>,
    /// Open, Check for Updates, Quit: relabelled when the UI language changes.
    items: [MenuItem; 3],
    labelled_in: Option<&'static str>,
}

impl Tray {
    /// Put the icon in the menu bar or tray. Call on the main thread once the event loop runs
    /// (eframe's app-creation callback).
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Tray, String> {
        let (tx, actions) = channel();
        let [open_label, check_label, quit_label] = wording::tray_labels();
        let open = MenuItem::new(open_label, true, None);
        let check = MenuItem::new(check_label, true, None);
        let quit = MenuItem::new(quit_label, true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &check, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;
        let ids = [(open.id().clone(), TrayAction::Open), (check.id().clone(), TrayAction::Check), (quit.id().clone(), TrayAction::Quit)];
        let wake = std::sync::Arc::new(wake);

        let (menu_tx, menu_wake) = (tx.clone(), std::sync::Arc::clone(&wake));
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, action)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = menu_tx.send(*action);
                menu_wake();
            }
        }));
        // On macOS a click opens the menu (the platform convention); elsewhere a left click opens
        // the window and the right button opens the menu.
        if !cfg!(target_os = "macos") {
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    let _ = tx.send(TrayAction::Open);
                    wake();
                }
            }));
        }

        let macos = cfg!(target_os = "macos");
        let (w, h, rgba) = artcraft_toolbox_engine::icons::decode_png(if macos { TEMPLATE_PNG } else { COLOR_PNG })?;
        let image = tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())?;
        let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("ArtCraft Toolbox").with_menu_on_left_click(macos);
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(image);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(image);
        let icon = builder.build().map_err(|e| e.to_string())?;
        Ok(Tray { _icon: icon, actions, items: [open, check, quit], labelled_in: Some(i18n::current().code()) })
    }

    /// Follow the UI language (cheap when it hasn't changed).
    pub fn relabel(&mut self) {
        let lang = i18n::current().code();
        if self.labelled_in == Some(lang) {
            return;
        }
        for (item, label) in self.items.iter().zip(wording::tray_labels()) {
            item.set_text(label);
        }
        self.labelled_in = Some(lang);
    }

    /// The actions clicked since the last call.
    pub fn actions(&self) -> Vec<TrayAction> {
        self.actions.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_tray_images_decode() {
        for png in [TEMPLATE_PNG, COLOR_PNG] {
            let (w, h, rgba) = artcraft_toolbox_engine::icons::decode_png(png).unwrap();
            assert!(w >= 32 && w == h, "{w}x{h}");
            assert!(tray_icon::Icon::from_rgba(rgba, w, h).is_ok());
        }
    }
}
