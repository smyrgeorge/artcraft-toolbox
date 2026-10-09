//! The Settings tab. Every change is a `settings.set` command, like any other action.

use artcraft_toolbox_model::Channel;
use serde_json::json;

use crate::ToolboxApp;
use crate::theme::Tokens;
use crate::widgets::card;

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    let s = app.session.settings().clone();
    card(ui, t, |ui| {
        ui.label(egui::RichText::new("Updates").strong());
        ui.horizontal(|ui| {
            ui.label("Channel:");
            for (c, label) in [(Channel::Stable, "Stable"), (Channel::Prerelease, "Pre-release")] {
                if ui.radio(s.channel == c, label).clicked() && s.channel != c {
                    app.run("settings.set", json!({"channel": c}));
                }
            }
        });
        let mut auto = s.auto_update;
        if ui.checkbox(&mut auto, "Install updates automatically").changed() {
            app.run("settings.set", json!({"autoUpdate": auto}));
        }
        let mut notify = s.notifications;
        let r = ui.add_enabled(app.services.notify.is_some(), egui::Checkbox::new(&mut notify, "Notify me when updates are found"));
        if r.changed() {
            app.run("settings.set", json!({"notifications": notify}));
        }
        r.on_disabled_hover_text("Notifications aren't available here");
        let mut tray = s.close_to_tray;
        let label = if cfg!(target_os = "macos") {
            "Keep running in the menu bar when the window is closed"
        } else {
            "Keep running in the system tray when the window is closed"
        };
        let r = ui.add_enabled(app.services.tray, egui::Checkbox::new(&mut tray, label));
        if r.changed() {
            app.run("settings.set", json!({"closeToTray": tray}));
        }
        r.on_disabled_hover_text("There is no menu bar or tray icon on this desktop");
        ui.horizontal(|ui| {
            ui.label("Check every");
            let mut hours = s.check_interval_hours;
            if ui.add(egui::DragValue::new(&mut hours).range(0..=artcraft_toolbox_model::settings::MAX_CHECK_INTERVAL_HOURS).suffix(" h")).changed() {
                app.run("settings.set", json!({"checkIntervalHours": hours}));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Keep previous versions:");
            let mut keep = s.keep_previous;
            if ui.add(egui::DragValue::new(&mut keep).range(0..=artcraft_toolbox_model::settings::MAX_KEEP_PREVIOUS)).changed() {
                app.run("settings.set", json!({"keepPrevious": keep}));
            }
        });
    });
    ui.add_space(8.0);
    card(ui, t, |ui| {
        ui.label(egui::RichText::new("About").strong());
        ui.label(format!("ArtCraft Toolbox {}", artcraft_toolbox_engine::build_info::long_version()));
        let host = app.session.host().map(|h| h.to_string()).unwrap_or_else(|| "unsupported platform".into());
        ui.label(egui::RichText::new(format!("This computer: {host}")).color(t.text_dim));
        let data = app.session.store().map_or_else(|| "not saved (no data folder)".to_string(), |s| s.root().display().to_string());
        ui.label(egui::RichText::new(format!("Data folder: {data}")).color(t.text_dim));
    });
}
