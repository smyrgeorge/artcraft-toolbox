//! An app's one action, shared by its row and its page: Install, Open, or Cancel while it
//! installs (the progress bar goes under the status line, [`bar`]). Each is an engine command
//! (`app.install` in the background, `app.launch`).

use artcraft_toolbox_engine::install_cmds::{INSTALL, LAUNCH};
use artcraft_toolbox_engine::{AppStatus, JobId, JobInfo, Session, Status};
use serde_json::json;

use crate::ToolboxApp;
use crate::theme::Tokens;

/// What the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clicked {
    Install,
    Open,
    Cancel(JobId),
}

/// The install job running for `r`, if any.
pub fn installing(session: &Session, r: &AppStatus) -> Option<JobInfo> {
    session.job_for(INSTALL, &r.id)
}

/// Is the app installed (whatever its update state)?
pub fn installed(r: &AppStatus) -> bool {
    matches!(
        r.status,
        Status::UpToDate { .. } | Status::UpdateAvailable { .. } | Status::Unknown { installed: Some(_) } | Status::Unsupported { installed: Some(_) }
    )
}

/// Room the action needs on the right of a row.
pub const WIDTH: f32 = 84.0;

/// `Downloading 45%`, while an install runs.
pub fn progress_text(job: &JobInfo) -> String {
    let phase = job.phase.clone().unwrap_or_else(|| "Installing".into());
    match job.fraction {
        Some(f) if phase == "Downloading" => format!("{phase} {:.0}%", f * 100.0),
        _ => phase,
    }
}

/// A thin progress bar, `width` wide. Phases without a fraction (verifying, unpacking) slide a
/// segment along it.
pub fn bar(ui: &mut egui::Ui, job: &JobInfo, width: f32, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, t.border);
    let fill = match job.fraction {
        Some(f) => egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * f.clamp(0.0, 1.0), rect.height())),
        None => {
            let seg = rect.width() * 0.25;
            let x = rect.min.x - seg + (rect.width() + seg) * (ui.input(|i| i.time) * 0.8).fract() as f32;
            egui::Rect::from_x_y_ranges(x.max(rect.min.x)..=(x + seg).min(rect.max.x), rect.y_range())
        }
    };
    if fill.width() > 0.0 {
        ui.painter().rect_filled(fill, 2.0, t.accent);
    }
}

/// Draw the action and return what was clicked.
pub fn draw(ui: &mut egui::Ui, session: &Session, r: &AppStatus, _t: &Tokens) -> Option<Clicked> {
    if let Some(job) = installing(session, r) {
        let cancel = ui.button("Cancel").on_hover_text("Stop; installing again resumes the download");
        return cancel.clicked().then_some(Clicked::Cancel(job.id));
    }
    if installed(r) {
        return ui.button("Open").on_hover_text(format!("Open {}", r.name)).clicked().then_some(Clicked::Open);
    }
    match &r.status {
        Status::NotInstalled { latest } => {
            let reason = session.disabled_reason(INSTALL);
            let b = ui.add_enabled(reason.is_none(), egui::Button::new("Install"));
            let b = match &reason {
                Some(why) => b.on_disabled_hover_text(why),
                None => b.on_hover_text(format!("Install {} {latest}", r.name)),
            };
            b.clicked().then_some(Clicked::Install)
        }
        Status::Unknown { installed: None } => {
            ui.add_enabled(false, egui::Button::new("Install")).on_disabled_hover_text("Check for updates first");
            None
        }
        _ => None,
    }
}

/// Run what was clicked. Errors land in the status bar.
pub fn perform(app: &mut ToolboxApp, id: &str, clicked: Clicked) {
    match clicked {
        Clicked::Install => match app.session.start(INSTALL, json!({ "app": id })) {
            Ok(_) => app.notice = None,
            Err(e) => app.notice = Some(e.to_string()),
        },
        Clicked::Open => {
            app.run(LAUNCH, json!({ "app": id }));
        }
        Clicked::Cancel(job) => {
            app.session.cancel_job(job);
        }
    }
}
