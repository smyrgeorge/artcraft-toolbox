//! The Apps tab: every Crafting App with its status and its one action, in two panels. Installed
//! apps show their version line, the action (Update, Open, Adopt) and a menu (⋮) with the rest;
//! available apps show their tagline with Install under it. "Update all" when more than one update
//! is waiting; a faint footer says when the toolbox last checked.

use artcraft_toolbox_engine::install_cmds::{UNINSTALL, UPDATE};
use artcraft_toolbox_engine::versions_cmds::UPDATE_ALL;
use artcraft_toolbox_engine::{AppStatus, Status};

use crate::actions::{self, Clicked};
use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::widgets::{self, TILE, app_tile, panel, panel_title, panel_title_with};
use crate::{ToolboxApp, icons, self_update, wording};

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens) {
    let query = app.ui.search.trim().to_lowercase();
    let (installed, available): (Vec<AppStatus>, Vec<AppStatus>) =
        app.session.statuses().into_iter().filter(|r| matches(r, &query)).partition(|r| installed_version(&r.status).is_some() || r.found.is_some());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // The toolbox's own update comes first: nothing else is as easy to miss.
        if query.is_empty() {
            self_update::offer(app, ui, t);
        }
        if installed.is_empty() && available.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(fmt(tl!("No app matches “{query}”"), &[("query", app.ui.search.trim())])).color(t.text_dim))
            });
            return;
        }
        if !installed.is_empty() {
            panel(ui, t, |ui| {
                let waiting =
                    installed.iter().filter(|r| matches!(r.status, Status::UpdateAvailable { .. }) && actions::installing(&app.session, r).is_none()).count();
                ui.add_space(2.0);
                indent(ui, |ui| {
                    if waiting > 1 {
                        panel_title_with(ui, tl!("Installed"), t, |ui| {
                            let label = fmt(tl!("Update all ({n})"), &[("n", &waiting.to_string())]);
                            if actions::command_button(ui, &app.session, UPDATE_ALL, &label, tl!("Update every app that has an update").into()) {
                                app.update_all(false);
                            }
                        });
                    } else {
                        panel_title(ui, tl!("Installed"), t);
                    }
                });
                for r in &installed {
                    row(app, ui, r, t);
                }
            });
            ui.add_space(8.0);
        }
        if !available.is_empty() {
            panel(ui, t, |ui| {
                ui.add_space(2.0);
                // Searching unfolds it: a match must show.
                let folded = app.ui.available_folded && query.is_empty();
                if fold_header(ui, tl!("Available apps"), folded, t).clicked() {
                    app.ui.available_folded = !folded;
                }
                if !folded {
                    for r in &available {
                        row(app, ui, r, t);
                    }
                }
            });
        }
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            let line = wording::status_line(&app.session);
            ui.add(egui::Label::new(egui::RichText::new(line).small().color(t.text_faint)).truncate());
        });
        ui.add_space(4.0);
    });
}

/// Content lined up with the rows' text (the panel's own margin plus a row's).
fn indent<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 4, top: 2, bottom: 2 }).show(ui, add).inner
}

/// A panel title that folds its panel: a chevron, then the title.
fn fold_header(ui: &mut egui::Ui, title: &str, folded: bool, t: &Tokens) -> egui::Response {
    let height = 26.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, !folded, title));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(t.radius), t.card_hover);
        }
        widgets::focus_ring(ui, &response, t.radius, t);
        let chevron = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 0.0), egui::vec2(18.0, height));
        icons::paint(ui, chevron, if folded { "chevron-right" } else { "chevron-down" }, 14.0, t.text_dim);
        let galley = ui.painter().layout(title.to_string(), crate::theme::medium(12.5), t.text_dim, rect.width() - 30.0);
        ui.painter().galley(egui::pos2(chevron.right() + 2.0, rect.center().y - galley.size().y / 2.0), galley, t.text_dim);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// What a row's menu asked for (beyond the row's own action).
enum MenuChoice {
    Action(Clicked),
    Details,
    Uninstall,
}

/// One app: click anywhere but its buttons to open its details.
fn row(app: &mut ToolboxApp, ui: &mut egui::Ui, r: &AppStatus, t: &Tokens) {
    let icon = app.icon_texture(ui.ctx(), &r.id);
    let job = actions::installing(&app.session, r);
    let installed_row = installed_version(&r.status).is_some() || r.found.is_some();
    // The hover fill goes under the row, so its place is kept before the row is drawn.
    let background = ui.painter().add(egui::Shape::Noop);
    let mut choice = None;
    let response = ui
        .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                choice = if installed_row {
                    installed_row_content(ui, &app.session, r, job.as_ref(), icon.as_ref(), t)
                } else {
                    available_row_content(ui, &app.session, r, job.as_ref(), icon.as_ref(), t)
                };
            });
        })
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() || response.has_focus() {
        ui.painter().set(background, egui::Shape::rect_filled(response.rect, egui::CornerRadius::same(t.radius), t.card_hover));
    }
    // For screen readers: the app and its status (for an app that isn't installed, the version
    // it would get rather than its tagline); for the keyboard: a ring when Tab reaches it (Enter
    // or Space opens the page).
    let status = match (&r.status, &r.pinned) {
        (Status::NotInstalled { .. } | Status::Unknown { installed: None }, pin) if r.found.is_none() => {
            let line = wording::status(&r.status);
            match pin {
                Some(v) => fmt(tl!("{status} · pinned to {version}"), &[("status", &line), ("version", &v.to_string())]),
                None => line,
            }
        }
        _ => status_text(r, t).0,
    };
    let spoken = format!("{}, {status}", r.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
    widgets::focus_ring(ui, &response, t.radius, t);
    match choice {
        Some(MenuChoice::Action(c)) => actions::perform(app, &r.id, c),
        Some(MenuChoice::Details) => app.ui.selected = Some(r.id.clone()),
        Some(MenuChoice::Uninstall) => app.ui.confirm_uninstall = Some(r.id.clone()),
        None if response.clicked() => app.ui.selected = Some(r.id.clone()),
        None => {}
    }
}

/// Icon, name and version line; the action and the menu on the right.
fn installed_row_content(
    ui: &mut egui::Ui,
    session: &artcraft_toolbox_engine::Session,
    r: &AppStatus,
    job: Option<&artcraft_toolbox_engine::JobInfo>,
    icon: Option<&egui::TextureHandle>,
    t: &Tokens,
) -> Option<MenuChoice> {
    ui.horizontal(|ui| {
        let badge = matches!(r.status, Status::UpdateAvailable { .. }).then_some((t.accent, t.card));
        app_tile(ui, &r.id, &r.name, icon, TILE, badge);
        ui.add_space(4.0);
        let mut choice = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if r.checking {
                ui.spinner();
            } else {
                choice = menu(ui, session, r, job.is_some(), t);
                if let Some(c) = actions::draw(ui, session, r, t) {
                    choice = Some(MenuChoice::Action(c));
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    name_and_status(ui, r, job, t);
                });
            });
        });
        choice
    })
    .inner
}

/// Icon, name, tagline, and Install (or the install's progress) under them.
fn available_row_content(
    ui: &mut egui::Ui,
    session: &artcraft_toolbox_engine::Session,
    r: &AppStatus,
    job: Option<&artcraft_toolbox_engine::JobInfo>,
    icon: Option<&egui::TextureHandle>,
    t: &Tokens,
) -> Option<MenuChoice> {
    ui.horizontal_top(|ui| {
        app_tile(ui, &r.id, &r.name, icon, TILE, None);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            name_and_status(ui, r, job, t);
            ui.add_space(4.0);
            if r.checking {
                ui.spinner();
                return None;
            }
            ui.horizontal(|ui| {
                let clicked = actions::draw(ui, session, r, t);
                if job.is_none()
                    && let Status::NotInstalled { latest } = &r.status
                {
                    let version = match &r.pinned {
                        Some(_) => format!("{latest} · {}", tl!("pinned")),
                        None => latest.to_string(),
                    };
                    ui.label(egui::RichText::new(version).small().color(t.text_faint));
                }
                clicked.map(MenuChoice::Action)
            })
            .inner
        })
        .inner
    })
    .inner
}

/// The name, the status line (or the install's phase and bar) under it. Long lines end in an
/// ellipsis; the tooltip has the full text.
fn name_and_status(ui: &mut egui::Ui, r: &AppStatus, job: Option<&artcraft_toolbox_engine::JobInfo>, t: &Tokens) {
    // Not selectable: a selectable label would take the click meant for the row.
    ui.add(egui::Label::new(egui::RichText::new(&r.name).font(widgets::name_font()).color(t.text)).truncate().selectable(false));
    let (line, color) = match (job, &r.status, &r.error) {
        (Some(j), _, _) => (actions::progress_text(j), t.accent_fg),
        (None, Status::Unknown { .. }, Some(e)) => (fmt(tl!("Couldn't check: {error}"), &[("error", wording::engine(e))]), t.danger),
        _ => status_text(r, t),
    };
    ui.add(egui::Label::new(egui::RichText::new(line).small().color(color)).truncate().selectable(false)).on_hover_text(hover(r));
    if let Some(j) = job {
        ui.add_space(2.0);
        actions::bar(ui, j, ui.available_width().clamp(48.0, 260.0), t);
    }
}

/// The ⋮ menu of an installed app: Open, Update, Details, Uninstall….
fn menu(ui: &mut egui::Ui, session: &artcraft_toolbox_engine::Session, r: &AppStatus, busy: bool, t: &Tokens) -> Option<MenuChoice> {
    let button = widgets::icon_button(ui, "ellipsis-vertical", &fmt(tl!("More actions for {app}"), &[("app", &r.name)]), false, t);
    let mut choice = None;
    egui::Popup::menu(&button).show(|ui| {
        ui.set_min_width(150.0);
        if actions::installed(r) && ui.button(tl!("Open")).clicked() {
            choice = Some(MenuChoice::Action(Clicked::Open));
        }
        if matches!(r.status, Status::UpdateAvailable { .. })
            && !busy
            && ui.add_enabled(session.disabled_reason(UPDATE).is_none(), egui::Button::new(tl!("Update"))).clicked()
        {
            choice = Some(MenuChoice::Action(Clicked::Update));
        }
        if ui.button(tl!("Details")).clicked() {
            choice = Some(MenuChoice::Details);
        }
        if actions::installed(r) && !busy {
            ui.separator();
            let enabled = session.disabled_reason(UNINSTALL).is_none();
            if ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(tl!("Uninstall…")).color(t.danger))).clicked() {
                choice = Some(MenuChoice::Uninstall);
            }
        }
    });
    choice
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
        // Not installed: what the app is (its version sits beside Install).
        Status::Unknown { installed: None } | Status::NotInstalled { .. } => (wording::tagline(&r.tagline).to_string(), t.text_dim),
        Status::UpdateAvailable { .. } => (wording::status(&r.status), t.accent_fg),
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
