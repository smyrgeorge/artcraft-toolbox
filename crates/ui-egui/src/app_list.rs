//! The Apps tab: every Crafting App with its status and its one action (Install, Update, Open).

use artcraft_toolbox_engine::update_cmds::CHECK;
use artcraft_toolbox_engine::{AppStatus, Status};

use crate::ToolboxApp;
use crate::theme::Tokens;
use crate::widgets::{TILE, app_tile, card, section};

/// Room kept for a row's action button (or spinner) on the right.
const ACTION_WIDTH: f32 = 84.0;

/// Why the action buttons are disabled: installing lands with milestone M2 (docs/roadmap.md).
pub(crate) const NOT_YET: &str = "Installing and updating apps is not available yet.";

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    toolbar(app, ui, t);
    let query = app.ui.search.trim().to_lowercase();
    let (installed, available): (Vec<AppStatus>, Vec<AppStatus>) =
        app.session.statuses().into_iter().filter(|r| matches(r, &query)).partition(|r| installed_version(&r.status).is_some());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if installed.is_empty() && available.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(format!("No app matches “{}”", app.ui.search.trim())).color(t.text_dim)));
            return;
        }
        if !installed.is_empty() {
            section(ui, "Installed", installed.len(), t);
            for r in &installed {
                row(app, ui, r, t);
            }
        }
        if !available.is_empty() {
            section(ui, "Available", available.len(), t);
            for r in &available {
                row(app, ui, r, t);
            }
        }
    });
}

/// Search, and "Check for updates" (or the running check's progress).
fn toolbar(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let check = app.session.jobs().into_iter().find(|j| j.command == CHECK);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match &check {
                Some(job) => {
                    ui.label(egui::RichText::new(format!("Checking {} of {}", job.done.min(job.total), job.total)).small().color(t.text_dim));
                    ui.spinner();
                }
                None => {
                    let reason = app.session.disabled_reason(CHECK);
                    let button = ui.add_enabled(reason.is_none(), egui::Button::new("Check for updates"));
                    let button = match &reason {
                        Some(why) => button.on_disabled_hover_text(why),
                        None => button.on_hover_text("Look for new versions of every app now"),
                    };
                    if button.clicked() {
                        app.start_check();
                    }
                }
            }
            ui.add(egui::TextEdit::singleline(&mut app.ui.search).hint_text("Search apps").desired_width(f32::INFINITY));
        });
    });
}

/// Case-insensitive match on name, id and tagline; an empty query matches everything.
pub fn matches(r: &AppStatus, query: &str) -> bool {
    query.is_empty() || [&r.name, &r.id, &r.tagline].iter().any(|s| s.to_lowercase().contains(query))
}

fn installed_version(s: &Status) -> Option<&artcraft_toolbox_engine::Version> {
    match s {
        Status::Unknown { installed } | Status::Unsupported { installed } => installed.as_ref(),
        Status::UpToDate { installed } | Status::UpdateAvailable { installed, .. } => Some(installed),
        Status::NotInstalled { .. } => None,
    }
}

/// One app: click anywhere but its action to open its details.
fn row(app: &mut ToolboxApp, ui: &mut egui::Ui, r: &AppStatus, t: &Tokens) {
    let icon = app.icon_texture(ui.ctx(), &r.id);
    // The row's own background takes the click; the action button, drawn on top, keeps its own.
    let clicked = ui
        .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| row_card(ui, r, icon.as_ref(), t))
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    if clicked {
        app.ui.selected = Some(r.id.clone());
    }
    ui.add_space(4.0);
}

fn row_card(ui: &mut egui::Ui, r: &AppStatus, icon: Option<&egui::TextureHandle>, t: &Tokens) {
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            app_tile(ui, &r.id, &r.name, icon, TILE, t);
            // Leave room for the action on the right; long lines end in an ellipsis (the tooltip
            // has the full text).
            let text_width = (ui.available_width() - ACTION_WIDTH).max(80.0);
            ui.vertical(|ui| {
                ui.set_max_width(text_width);
                // Not selectable: a selectable label would take the click meant for the row.
                ui.add(egui::Label::new(egui::RichText::new(&r.name).strong().color(t.text)).truncate().selectable(false));
                let (line, color) = match (&r.status, &r.error) {
                    (Status::Unknown { .. }, Some(e)) => (format!("Couldn't check: {e}"), t.danger),
                    _ => status_text(r, t),
                };
                ui.add(egui::Label::new(egui::RichText::new(line).small().color(color)).truncate().selectable(false)).on_hover_text(hover(r));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if r.checking {
                    ui.spinner();
                    return;
                }
                let action = match &r.status {
                    Status::Unknown { installed: None } | Status::NotInstalled { .. } => Some("Install"),
                    Status::UpdateAvailable { .. } => Some("Update"),
                    Status::Unknown { installed: Some(_) } | Status::UpToDate { .. } => Some("Open"),
                    Status::Unsupported { .. } => None,
                };
                if let Some(label) = action {
                    ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text(NOT_YET);
                }
            });
        });
    });
}

/// The status line under an app's name, and its colour.
fn status_text(r: &AppStatus, t: &Tokens) -> (String, egui::Color32) {
    let (line, color) = status_base(r, t);
    match &r.pinned {
        Some(v) => (format!("{line} · pinned to {v}"), color),
        None => (line, color),
    }
}

fn status_base(r: &AppStatus, t: &Tokens) -> (String, egui::Color32) {
    match &r.status {
        Status::Unknown { installed: None } => (r.tagline.clone(), t.text_dim),
        Status::UpdateAvailable { .. } => (r.status.label(), t.accent),
        Status::UpToDate { .. } => (r.status.label(), t.success),
        Status::Unsupported { .. } => (r.status.label(), t.warning),
        other => (other.label(), t.text_dim),
    }
}

/// The row's tooltip: the tagline, and why the last check failed.
fn hover(r: &AppStatus) -> String {
    let mut lines = vec![r.tagline.clone()];
    if let Some(e) = &r.error {
        lines.push(format!("Last check failed: {e}"));
    }
    lines.join("\n")
}
