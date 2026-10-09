//! An app's one action, shared by its row and its page: Install, Update, Open, Adopt (a copy
//! installed by hand), or Cancel while it installs or updates (the progress bar goes under the
//! status line, [`bar`]). Each is an engine command (`app.install` and `app.update` in the
//! background, `app.rollback`, `app.adopt`, `app.launch`).

use artcraft_toolbox_engine::install_cmds::{INSTALL, LAUNCH, UPDATE};
use artcraft_toolbox_engine::versions_cmds::{ADOPT, ROLLBACK};
use artcraft_toolbox_engine::{AppStatus, JobId, JobInfo, Session, Status, Version};
use serde_json::json;

use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::{ToolboxApp, wording};

/// What the user clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clicked {
    Install,
    Update,
    Open,
    Adopt,
    Cancel(JobId),
    /// Make this installed version or release the one in use (the details page's versions).
    UseVersion(Version),
}

/// The install or update job running for `r`, if any.
pub fn installing(session: &Session, r: &AppStatus) -> Option<JobInfo> {
    session.job_for(INSTALL, &r.id).or_else(|| session.job_for(UPDATE, &r.id))
}

/// Is the app installed by the toolbox (whatever its update state)?
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
    let phase = job.phase.as_deref().unwrap_or("Installing");
    match job.fraction {
        Some(f) if phase == "Downloading" => fmt(tl!("Downloading {percent}%"), &[("percent", &format!("{:.0}", f * 100.0))]),
        _ => wording::engine(phase).to_string(),
    }
}

/// A thin progress bar, `width` wide. Phases without a fraction (verifying, unpacking) slide a
/// segment along it.
pub fn bar(ui: &mut egui::Ui, job: &JobInfo, width: f32, t: &Tokens) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    // For screen readers: a progress indicator with its phase and, while downloading, its value.
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, progress_text(job));
        info.value = job.fraction.map(f64::from);
        info
    });
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

/// A button that is disabled, with the reason as its tooltip, while `command` can't run.
pub fn command_button(ui: &mut egui::Ui, session: &Session, command: &str, label: &str, hover: String) -> bool {
    let reason = session.disabled_reason(command);
    let b = ui.add_enabled(reason.is_none(), egui::Button::new(label));
    let b = match &reason {
        Some(why) => b.on_disabled_hover_text(wording::engine(why)),
        None => b.on_hover_text(hover),
    };
    b.clicked()
}

/// Draw the action and return what was clicked.
pub fn draw(ui: &mut egui::Ui, session: &Session, r: &AppStatus, _t: &Tokens) -> Option<Clicked> {
    if let Some(job) = installing(session, r) {
        let cancel = ui.button(tl!("Cancel")).on_hover_text(tl!("Stop; installing again resumes the download"));
        return cancel.clicked().then_some(Clicked::Cancel(job.id));
    }
    match &r.status {
        Status::UpdateAvailable { installed, latest } => {
            let hover = fmt(
                tl!("Update {app} from {installed} to {latest}"),
                &[("app", &r.name), ("installed", &installed.to_string()), ("latest", &latest.to_string())],
            );
            command_button(ui, session, UPDATE, tl!("Update"), hover).then_some(Clicked::Update)
        }
        _ if installed(r) => ui.button(tl!("Open")).on_hover_text(fmt(tl!("Open {app}"), &[("app", &r.name)])).clicked().then_some(Clicked::Open),
        _ if r.found.is_some() => {
            let hover = fmt(tl!("Let the toolbox update and roll back the {app} found on this computer"), &[("app", &r.name)]);
            command_button(ui, session, ADOPT, tl!("Adopt"), hover).then_some(Clicked::Adopt)
        }
        Status::NotInstalled { latest } => {
            let hover = fmt(tl!("Install {app} {version}"), &[("app", &r.name), ("version", &latest.to_string())]);
            command_button(ui, session, INSTALL, tl!("Install"), hover).then_some(Clicked::Install)
        }
        Status::Unknown { installed: None } => {
            ui.add_enabled(false, egui::Button::new(tl!("Install"))).on_disabled_hover_text(tl!("Check for updates first"));
            None
        }
        _ => None,
    }
}

/// Run what was clicked. Errors land in the status bar.
pub fn perform(app: &mut ToolboxApp, id: &str, clicked: Clicked) {
    let start = |app: &mut ToolboxApp, command: &str, params: serde_json::Value| match app.session.start(command, params) {
        Ok(_) => app.notice = None,
        Err(e) => app.notice = Some(e.to_string()),
    };
    match clicked {
        Clicked::Install => start(app, INSTALL, json!({ "app": id })),
        Clicked::Update => {
            // A kept version is switched to; anything else is downloaded.
            let latest = app.session.app_status(id).ok().and_then(|st| match st.status {
                Status::UpdateAvailable { latest, .. } => Some(latest),
                _ => None,
            });
            match latest.filter(|v| app.session.inventory().get(id, v).is_some()) {
                Some(v) => {
                    app.run(ROLLBACK, json!({ "app": id, "version": v }));
                }
                None => start(app, UPDATE, json!({ "app": id })),
            }
        }
        Clicked::UseVersion(v) => {
            let kept = app.session.inventory().get(id, &v).is_some();
            let installed = app.session.inventory().current(id).is_some();
            match (kept, installed) {
                (true, _) => {
                    app.run(ROLLBACK, json!({ "app": id, "version": v }));
                }
                (false, true) => start(app, UPDATE, json!({ "app": id, "version": v })),
                (false, false) => start(app, INSTALL, json!({ "app": id, "version": v })),
            }
        }
        Clicked::Adopt => {
            app.run(ADOPT, json!({ "app": id }));
        }
        Clicked::Open => {
            app.run(LAUNCH, json!({ "app": id }));
        }
        Clicked::Cancel(job) => {
            app.session.cancel_job(job);
        }
    }
}
