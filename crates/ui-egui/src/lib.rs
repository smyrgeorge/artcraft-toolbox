//! ArtCraft Toolbox's UI shell, built on egui/eframe.
//!
//! Deliberately thin: everything it shows comes from the [`Session`], and every action it takes
//! is an engine command run through [`Session::execute`]. UI-only state (the open tab, the search
//! text) lives in [`state::UiState`], plain serde data, so automation can read and drive it.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod app_list;
pub mod settings_ui;
pub mod state;
pub mod theme;
pub mod widgets;

use artcraft_toolbox_engine::update_cmds::CHECK;
use artcraft_toolbox_engine::{Session, Started, time};
use serde_json::Value;

use state::{Tab, UiState};
use theme::Tokens;

/// The whole app: one session, one UI state.
pub struct ToolboxApp {
    pub session: Session,
    pub ui: UiState,
    /// The last command error, shown in the status bar until the next action.
    pub notice: Option<String>,
}

impl ToolboxApp {
    pub fn new(session: Session) -> ToolboxApp {
        ToolboxApp { session, ui: UiState::default(), notice: None }
    }

    /// Theme and style; call once on the egui context before the first frame.
    pub fn setup_context(ctx: &egui::Context) {
        theme::apply(ctx);
    }

    /// Run a command from the UI. Errors go to the status bar, never to a panic.
    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.session.execute(id, params) {
            Ok(v) => {
                self.notice = None;
                Some(v)
            }
            Err(e) => {
                log::warn!("{id}: {e}");
                self.notice = Some(e.to_string());
                None
            }
        }
    }

    /// Start a background check for updates (all apps). Disabled states (offline, already
    /// running, GitHub's rate limit) land in the status bar.
    pub fn start_check(&mut self) {
        match self.session.start(CHECK, Value::Null) {
            Ok(Started::Job(_) | Started::Done(_)) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Start a check if one is due (`checkIntervalHours`); the desktop app calls this at start.
    pub fn check_if_due(&mut self) {
        if self.session.check_due() {
            self.start_check();
        }
    }

    /// Apply background job progress; report the jobs that ended.
    fn poll(&mut self, ctx: &egui::Context) {
        for event in self.session.poll_jobs() {
            self.notice = match &event.result {
                Ok(v) if event.command == CHECK => self.session.check_notice(v),
                Ok(_) => None,
                Err(e) => Some(e.clone()),
            };
        }
        if self.session.has_jobs() {
            // Results arrive from worker threads; keep drawing until they are all in.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Draw one frame into `ui` (the eframe root, or a test harness).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        let t = Tokens::get(ui.ctx());
        egui::Panel::top("header").frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(16, 12))).show(ui, |ui| self.header(ui, &t));
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ui, |ui| self.status_bar(ui, &t));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.bg).inner_margin(egui::Margin::symmetric(8, 8))).show(ui, |ui| match self.ui.tab {
            Tab::Apps => app_list::show(self, ui, &t),
            Tab::Settings => settings_ui::show(self, ui, &t),
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("ArtCraft Toolbox").heading().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for tab in [Tab::Settings, Tab::Apps] {
                    if ui.selectable_label(self.ui.tab == tab, tab.label()).clicked() {
                        self.ui.tab = tab;
                    }
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        ui.horizontal(|ui| {
            if let Some(n) = &self.notice {
                ui.label(egui::RichText::new(n).small().color(t.danger)).on_hover_text(n);
                return;
            }
            ui.label(egui::RichText::new(status_line(&self.session)).small().color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(artcraft_toolbox_engine::build_info::long_version()).small().color(t.text_faint));
            });
        });
    }
}

/// `Checked 5 min ago · 12 apps · macos-aarch64`.
pub fn status_line(session: &Session) -> String {
    let checked = match session.last_checked() {
        Some(at) => format!("Checked {}", time::ago(session.now(), at)),
        None => "Not checked yet".into(),
    };
    let host = session.host().map(|h| h.to_string()).unwrap_or_else(|| "unsupported platform".into());
    format!("{checked} · {} apps · {host}", session.catalog().apps.len())
}

impl eframe::App for ToolboxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
