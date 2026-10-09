//! The Settings tab. Every change is a `settings.set` command, like any other action.

use artcraft_toolbox_model::settings::{MAX_CHECK_INTERVAL_HOURS, MAX_KEEP_PREVIOUS, TEXT_SIZES};
use artcraft_toolbox_model::{Channel, ThemePref};
use serde_json::json;

use crate::ToolboxApp;
use crate::i18n::{self, Lang, fmt};
use crate::theme::Tokens;
use crate::widgets::{card, panel_title, toggle_row};

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    let s = app.session.settings().clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        card(ui, t, |ui| {
            panel_title(ui, tl!("Updates"), t);
            ui.add_space(2.0);
            egui::Grid::new("updates").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tl!("Channel"));
                let channel_name = |c: Channel| match c {
                    Channel::Stable => tl!("Stable"),
                    Channel::Prerelease => tl!("Pre-release"),
                };
                egui::ComboBox::from_id_salt("channel").selected_text(channel_name(s.channel)).show_ui(ui, |ui| {
                    for c in [Channel::Stable, Channel::Prerelease] {
                        if ui.selectable_label(s.channel == c, channel_name(c)).clicked() && s.channel != c {
                            app.run("settings.set", json!({"channel": c}));
                        }
                    }
                });
                ui.end_row();

                ui.label(tl!("Check every"));
                let mut hours = s.check_interval_hours;
                let hours_suffix = format!(" {}", tl!("h"));
                if ui
                    .add(egui::DragValue::new(&mut hours).range(0..=MAX_CHECK_INTERVAL_HOURS).suffix(hours_suffix))
                    .on_hover_text(tl!("0: only when asked"))
                    .changed()
                {
                    app.run("settings.set", json!({"checkIntervalHours": hours}));
                }
                ui.end_row();

                ui.label(tl!("Keep previous versions"));
                let mut keep = s.keep_previous;
                if ui.add(egui::DragValue::new(&mut keep).range(0..=MAX_KEEP_PREVIOUS)).changed() {
                    app.run("settings.set", json!({"keepPrevious": keep}));
                }
                ui.end_row();
            });
            ui.add_space(4.0);
            let mut auto = s.auto_update;
            if toggle_row(ui, &mut auto, tl!("Install updates automatically"), t).changed() {
                app.run("settings.set", json!({"autoUpdate": auto}));
            }
            let mut notify = s.notifications;
            let r = ui.add_enabled_ui(app.services.notify.is_some(), |ui| toggle_row(ui, &mut notify, tl!("Notify me when updates are found"), t)).inner;
            if r.changed() {
                app.run("settings.set", json!({"notifications": notify}));
            }
            r.on_disabled_hover_text(tl!("Notifications aren't available here"));
            let mut tray = s.close_to_tray;
            let label = if cfg!(target_os = "macos") {
                tl!("Keep running in the menu bar when the window is closed")
            } else {
                tl!("Keep running in the system tray when the window is closed")
            };
            let r = ui.add_enabled_ui(app.services.tray, |ui| toggle_row(ui, &mut tray, label, t)).inner;
            if r.changed() {
                app.run("settings.set", json!({"closeToTray": tray}));
            }
            r.on_disabled_hover_text(tl!("There is no menu bar or tray icon on this desktop"));
        });
        ui.add_space(8.0);
        card(ui, t, |ui| {
            panel_title(ui, tl!("Appearance"), t);
            ui.add_space(2.0);
            egui::Grid::new("appearance").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tl!("Language"));
                let auto = fmt(tl!("Automatic ({language})"), &[("language", i18n::system_lang().name())]);
                let current = if s.language == "auto" { auto.clone() } else { Lang::from_pref(&s.language).name().to_string() };
                egui::ComboBox::from_id_salt("language").selected_text(current).show_ui(ui, |ui| {
                    if ui.selectable_label(s.language == "auto", &auto).clicked() && s.language != "auto" {
                        app.run("settings.set", json!({"language": "auto"}));
                    }
                    for lang in Lang::all() {
                        let chosen = s.language == lang.code();
                        if ui.selectable_label(chosen, lang.name()).clicked() && !chosen {
                            app.run("settings.set", json!({"language": lang.code()}));
                        }
                    }
                });
                ui.end_row();

                ui.label(tl!("Theme"));
                let theme_name = |p: ThemePref| match p {
                    ThemePref::System => tl!("Same as the system"),
                    ThemePref::Dark => tl!("Dark"),
                    ThemePref::Light => tl!("Light"),
                };
                egui::ComboBox::from_id_salt("theme").selected_text(theme_name(s.theme)).show_ui(ui, |ui| {
                    for p in [ThemePref::System, ThemePref::Dark, ThemePref::Light] {
                        if ui.selectable_label(s.theme == p, theme_name(p)).clicked() && s.theme != p {
                            app.run("settings.set", json!({"theme": p.as_str()}));
                        }
                    }
                });
                ui.end_row();

                ui.label(tl!("Text size"));
                let percent = |n: u32| fmt(tl!("{n}%"), &[("n", &n.to_string())]);
                egui::ComboBox::from_id_salt("text-size")
                    .selected_text(percent(s.text_size))
                    .show_ui(ui, |ui| {
                        for n in TEXT_SIZES {
                            if ui.selectable_label(s.text_size == n, percent(n)).clicked() && s.text_size != n {
                                app.run("settings.set", json!({"textSize": n}));
                            }
                        }
                    })
                    .response
                    .on_hover_text(fmt(
                        tl!("Also {keys}"),
                        &[("keys", if cfg!(target_os = "macos") { "Cmd +, Cmd -, Cmd 0" } else { "Ctrl +, Ctrl -, Ctrl 0" })],
                    ));
                ui.end_row();
            });
        });
        ui.add_space(8.0);
        card(ui, t, |ui| {
            panel_title(ui, tl!("About"), t);
            ui.add_space(2.0);
            ui.label(format!("ArtCraft Toolbox {}", artcraft_toolbox_engine::build_info::long_version()));
            let host = app.session.host().map(|h| h.to_string()).unwrap_or_else(|| tl!("unsupported platform").to_string());
            ui.label(egui::RichText::new(fmt(tl!("This computer: {host}"), &[("host", &host)])).color(t.text_dim));
            let data = app.session.store().map_or_else(|| tl!("not saved (no data folder)").to_string(), |s| s.root().display().to_string());
            ui.label(egui::RichText::new(fmt(tl!("Data folder: {path}"), &[("path", &data)])).color(t.text_dim));
        });
    });
}
