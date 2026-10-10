//! The toolbox's own update (docs/architecture.md § 7): the offer at the top of the Apps tab when
//! a newer ArtCraft Toolbox is published, and the same action on the About card. `Update` starts
//! `toolbox.update` (download, verify, stage); once staged, `Restart to update` asks the desktop
//! app to swap the new version in and start it ([`ToolboxApp::restart_wanted`]).

use artcraft_toolbox_engine::toolbox_cmds::UPDATE;
use artcraft_toolbox_engine::{SelfStatus, Status};
use serde_json::Value;

use crate::actions;
use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::widgets::{self, panel, primary_button};
use crate::{ToolboxApp, icons, wording};

/// Is there something to offer: an update, or one already downloaded?
pub fn offered(st: &SelfStatus) -> bool {
    st.staged.is_some() || matches!(st.status, Status::UpdateAvailable { .. })
}

/// The panel at the top of the Apps tab: the toolbox's mark, its name, the status line and the
/// action. Draws nothing when there is nothing to offer.
pub fn offer(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    let Some(st) = app.session.self_status().filter(offered) else { return };
    panel(ui, t, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(icons::logo(widgets::TILE));
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    action(app, ui, &st, t);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add(egui::Label::new(egui::RichText::new(&st.name).font(widgets::name_font()).color(t.text)).truncate().selectable(false));
                            status_line(app, ui, &st, t);
                        });
                    });
                });
            });
        });
    });
    ui.add_space(8.0);
}

/// The status line, with the download's progress bar while it runs.
pub fn status_line(app: &ToolboxApp, ui: &mut egui::Ui, st: &SelfStatus, t: &Tokens) {
    match app.session.job_for(UPDATE, &st.id) {
        Some(job) => {
            ui.add(egui::Label::new(egui::RichText::new(actions::progress_text(&job)).small().color(t.accent_fg)).wrap().selectable(false));
            ui.add_space(2.0);
            actions::bar(ui, &job, ui.available_width().clamp(48.0, 260.0), t);
        }
        None => {
            // Wrapped, not truncated: "restart to update" must stay readable beside the button.
            let (line, color) = wording::self_status(st, t);
            ui.add(egui::Label::new(egui::RichText::new(line).small().color(color)).wrap().selectable(false));
            if let (Some(why), Status::UpdateAvailable { .. }) = (&st.cannot_update, &st.status) {
                let text = fmt(tl!("This copy can't update itself: {reason}"), &[("reason", wording::engine(why))]);
                ui.add(egui::Label::new(egui::RichText::new(text).small().color(t.text_dim)).wrap().selectable(false));
            }
        }
    }
}

/// `Update`, `Cancel` while it downloads, or `Restart to update` once it is downloaded. Nothing
/// when this copy can't update itself (the status line says why).
pub fn action(app: &mut ToolboxApp, ui: &mut egui::Ui, st: &SelfStatus, t: &Tokens) {
    if let Some(job) = app.session.job_for(UPDATE, &st.id) {
        if ui.button(tl!("Cancel")).on_hover_text(tl!("Stop; installing again resumes the download")).clicked() {
            app.session.cancel_job(job.id);
        }
        return;
    }
    if let Some(v) = &st.staged {
        let hover = fmt(tl!("Quit ArtCraft Toolbox and start version {version}"), &[("version", &v.to_string())]);
        if ui.add(primary_button(tl!("Restart to update"), t)).on_hover_text(hover).clicked() {
            app.restart_wanted = true;
        }
        return;
    }
    if let Status::UpdateAvailable { latest, .. } = &st.status
        && st.cannot_update.is_none()
    {
        let hover = fmt(tl!("Update ArtCraft Toolbox to {version}"), &[("version", &latest.to_string())]);
        if actions::command_button_with(ui, &app.session, UPDATE, primary_button(tl!("Update"), t), hover) {
            app.start_self_update();
        }
    }
}

impl ToolboxApp {
    /// Download and stage the newer toolbox (`toolbox.update`); errors land in the banner.
    pub fn start_self_update(&mut self) {
        match self.session.start(UPDATE, Value::Null) {
            Ok(_) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// After a check: download the toolbox's own update by itself when updates are automatic
    /// (`autoUpdate`), quietly; it is announced once it is ready.
    pub(crate) fn self_update_if_automatic(&mut self) {
        if !self.session.settings().auto_update || self.session.disabled_reason(UPDATE).is_some() {
            return;
        }
        let Some(st) = self.session.self_status() else { return };
        if st.staged.is_some() || !matches!(st.status, Status::UpdateAvailable { .. }) {
            return;
        }
        match self.session.start(UPDATE, Value::Null) {
            Ok(artcraft_toolbox_engine::Started::Job(id)) => {
                self.auto_self_update = Some(id);
            }
            Ok(_) => {}
            Err(e) => log::warn!("automatic toolbox update: {e}"),
        }
    }

    /// "ArtCraft Toolbox 0.2.0 will be used after a restart", when notifications are on.
    pub(crate) fn announce_self_update_ready(&self, version: &str) {
        if !self.session.settings().notifications {
            return;
        }
        if let Some(notify) = &self.services.notify {
            notify(tl!("Update ready"), &fmt(tl!("ArtCraft Toolbox {version} will be used after a restart"), &[("version", version)]));
        }
    }
}
