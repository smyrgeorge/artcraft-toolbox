//! The Apps tab: every Crafting App with its status and its one action (Install, Update, Open).

use artcraft_toolbox_engine::{AppStatus, Status};

use crate::ToolboxApp;
use crate::theme::Tokens;
use crate::widgets::{app_tile, card, section};

/// Why the action buttons are disabled: installing lands with milestone M2 (docs/roadmap.md).
const NOT_YET: &str = "Installing and updating apps is not available yet.";

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.add(egui::TextEdit::singleline(&mut app.ui.search).hint_text("Search apps").desired_width(f32::INFINITY));
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
                row(ui, r, t);
            }
        }
        if !available.is_empty() {
            section(ui, "Available", available.len(), t);
            for r in &available {
                row(ui, r, t);
            }
        }
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

fn row(ui: &mut egui::Ui, r: &AppStatus, t: &Tokens) {
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            app_tile(ui, &r.id, &r.name, t);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&r.name).strong().color(t.text));
                let (line, color) = match &r.status {
                    Status::Unknown { installed: None } => (r.tagline.clone(), t.text_dim),
                    Status::UpdateAvailable { .. } => (r.status.label(), t.accent),
                    Status::UpToDate { .. } => (r.status.label(), t.success),
                    Status::Unsupported { .. } => (r.status.label(), t.warning),
                    other => (other.label(), t.text_dim),
                };
                ui.label(egui::RichText::new(line).small().color(color));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
    ui.add_space(4.0);
}
