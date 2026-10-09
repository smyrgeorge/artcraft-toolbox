//! The Apps tab: every Crafting App with its status and its one action (Install, Update, Open,
//! Adopt), and "Update all" when more than one update is waiting.

use artcraft_toolbox_engine::update_cmds::CHECK;
use artcraft_toolbox_engine::versions_cmds::UPDATE_ALL;
use artcraft_toolbox_engine::{AppStatus, Status};

use crate::actions;
use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::widgets::{TILE, app_tile, card, section, section_with};
use crate::{ToolboxApp, wording};

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    toolbar(app, ui, t);
    let query = app.ui.search.trim().to_lowercase();
    let (installed, available): (Vec<AppStatus>, Vec<AppStatus>) =
        app.session.statuses().into_iter().filter(|r| matches(r, &query)).partition(|r| installed_version(&r.status).is_some() || r.found.is_some());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if installed.is_empty() && available.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(fmt(tl!("No app matches “{query}”"), &[("query", app.ui.search.trim())])).color(t.text_dim))
            });
            return;
        }
        if !installed.is_empty() {
            let waiting =
                installed.iter().filter(|r| matches!(r.status, Status::UpdateAvailable { .. }) && actions::installing(&app.session, r).is_none()).count();
            if waiting > 1 {
                section_with(ui, tl!("Installed"), installed.len(), t, |ui| {
                    let label = fmt(tl!("Update all ({n})"), &[("n", &waiting.to_string())]);
                    if actions::command_button(ui, &app.session, UPDATE_ALL, &label, tl!("Update every app that has an update").into()) {
                        app.update_all(false);
                    }
                });
            } else {
                section(ui, tl!("Installed"), installed.len(), t);
            }
            for r in &installed {
                row(app, ui, r, t);
            }
        }
        if !available.is_empty() {
            section(ui, tl!("Available"), available.len(), t);
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
                    ui.label(
                        egui::RichText::new(fmt(
                            tl!("Checking {done} of {total}"),
                            &[("done", &job.done.min(job.total).to_string()), ("total", &job.total.to_string())],
                        ))
                        .small()
                        .color(t.text_dim),
                    );
                    ui.spinner();
                }
                None => {
                    let reason = app.session.disabled_reason(CHECK);
                    let button = ui.add_enabled(reason.is_none(), egui::Button::new(tl!("Check for updates")));
                    let button = match &reason {
                        Some(why) => button.on_disabled_hover_text(why),
                        None => button.on_hover_text(tl!("Look for new versions of every app now")),
                    };
                    if button.clicked() {
                        app.start_check();
                    }
                }
            }
            ui.add(egui::TextEdit::singleline(&mut app.ui.search).hint_text(tl!("Search apps")).desired_width(f32::INFINITY));
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
    let job = actions::installing(&app.session, r);
    let mut clicked = None;
    // The row's own background takes the click; the action, drawn on top, keeps its own.
    let response = ui
        .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| clicked = row_card(ui, &app.session, r, job.as_ref(), icon.as_ref(), t))
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // For screen readers: the app and its status line; for the keyboard: a ring when Tab reaches
    // it (Enter or Space opens the page).
    let spoken = format!("{}, {}", r.name, status_text(r, t).0);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect, egui::CornerRadius::same(t.radius), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
    }
    let opened = response.clicked();
    match clicked {
        Some(c) => actions::perform(app, &r.id, c),
        None if opened => app.ui.selected = Some(r.id.clone()),
        None => {}
    }
    ui.add_space(4.0);
}

fn row_card(
    ui: &mut egui::Ui,
    session: &artcraft_toolbox_engine::Session,
    r: &AppStatus,
    job: Option<&artcraft_toolbox_engine::JobInfo>,
    icon: Option<&egui::TextureHandle>,
    t: &Tokens,
) -> Option<actions::Clicked> {
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            app_tile(ui, &r.id, &r.name, icon, TILE, t);
            // Leave room for the action on the right; long lines end in an ellipsis (the tooltip
            // has the full text).
            let text_width = (ui.available_width() - actions::WIDTH).max(80.0);
            ui.vertical(|ui| {
                ui.set_max_width(text_width);
                // Not selectable: a selectable label would take the click meant for the row.
                ui.add(egui::Label::new(egui::RichText::new(&r.name).strong().color(t.text)).truncate().selectable(false));
                let (line, color) = match (job, &r.status, &r.error) {
                    (Some(j), _, _) => (actions::progress_text(j), t.accent_fg),
                    (None, Status::Unknown { .. }, Some(e)) => (fmt(tl!("Couldn't check: {error}"), &[("error", wording::engine(e))]), t.danger),
                    _ => status_text(r, t),
                };
                ui.add(egui::Label::new(egui::RichText::new(line).small().color(color)).truncate().selectable(false)).on_hover_text(hover(r));
                if let Some(j) = job {
                    ui.add_space(2.0);
                    actions::bar(ui, j, (text_width - 16.0).max(48.0), t);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if r.checking {
                    ui.spinner();
                    return None;
                }
                actions::draw(ui, session, r, t)
            })
            .inner
        })
        .inner
    })
    .inner
}

/// The status line under an app's name, and its colour.
fn status_text(r: &AppStatus, t: &Tokens) -> (String, egui::Color32) {
    if let (Some(v), false) = (&r.found, actions::installed(r)) {
        return (fmt(tl!("{version} installed outside the toolbox"), &[("version", &v.to_string())]), t.warning);
    }
    let (line, color) = status_base(r, t);
    match &r.pinned {
        Some(v) => (fmt(tl!("{status} · pinned to {version}"), &[("status", &line), ("version", &v.to_string())]), color),
        None => (line, color),
    }
}

fn status_base(r: &AppStatus, t: &Tokens) -> (String, egui::Color32) {
    match &r.status {
        Status::Unknown { installed: None } => (wording::tagline(&r.tagline).to_string(), t.text_dim),
        Status::UpdateAvailable { .. } => (wording::status(&r.status), t.accent_fg),
        Status::UpToDate { .. } => (wording::status(&r.status), t.success),
        Status::Unsupported { .. } => (wording::status(&r.status), t.warning),
        other => (wording::status(other), t.text_dim),
    }
}

/// The row's tooltip: the tagline, and why the last check failed.
fn hover(r: &AppStatus) -> String {
    let mut lines = vec![wording::tagline(&r.tagline).to_string()];
    if let Some(e) = &r.error {
        lines.push(fmt(tl!("Last check failed: {error}"), &[("error", wording::engine(e))]));
    }
    lines.join("\n")
}
