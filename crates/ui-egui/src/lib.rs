//! ArtCraft Toolbox's UI shell, built on egui/eframe.
//!
//! Deliberately thin: everything it shows comes from the [`Session`], and every action it takes
//! is an engine command run through [`Session::execute`] or [`Session::start`]. UI-only state
//! (the open tab, the search text, the app whose details are open) lives in [`state::UiState`],
//! plain serde data, so automation can read and drive it. What only the platform can do (OS
//! notifications, a menu-bar or tray icon) comes in through [`Services`], from the app.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod actions;
pub mod app_list;
pub mod details;
pub mod settings_ui;
pub mod state;
pub mod theme;
pub mod widgets;

use std::collections::HashMap;
use std::time::Duration;

use artcraft_toolbox_engine::icons::REFRESH_ICONS;
use artcraft_toolbox_engine::update_cmds::CHECK;
use artcraft_toolbox_engine::{Session, Started, time};
use serde_json::Value;

use state::{Tab, UiState};
use theme::Tokens;

/// How often the app wakes up when nothing happens, to start due checks (also while hidden).
pub const IDLE_TICK: Duration = Duration::from_secs(60);

/// Shows an OS notification: `(title, body)`.
pub type Notify = Box<dyn Fn(&str, &str)>;

/// What the platform provides. All optional: tests use none.
#[derive(Default)]
pub struct Services {
    pub notify: Option<Notify>,
    /// A menu-bar or tray icon exists, so closing the window can keep the toolbox running.
    pub tray: bool,
}

/// The whole app: one session, one UI state.
pub struct ToolboxApp {
    pub session: Session,
    pub ui: UiState,
    /// The last command error, shown in the status bar until the next action.
    pub notice: Option<String>,
    pub services: Services,
    /// Set by an explicit Quit (the tray menu): closing the window then really quits.
    pub quitting: bool,
    pub(crate) markdown: egui_commonmark::CommonMarkCache,
    /// App icons uploaded to the GPU, with the revision they were made from.
    textures: HashMap<String, (u64, egui::TextureHandle)>,
}

impl ToolboxApp {
    pub fn new(session: Session) -> ToolboxApp {
        ToolboxApp::with_services(session, Services::default())
    }

    pub fn with_services(session: Session, services: Services) -> ToolboxApp {
        ToolboxApp {
            session,
            ui: UiState::default(),
            notice: None,
            services,
            quitting: false,
            markdown: egui_commonmark::CommonMarkCache::default(),
            textures: HashMap::new(),
        }
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

    /// Start the background work that is due: an update check (`checkIntervalHours`, with the
    /// engine's retry back-off) and an icon refresh (missing or week-old icons). Runs from
    /// [`eframe::App::logic`], so also while the window is hidden; the desktop app calls it at start.
    pub fn background(&mut self) {
        if self.session.check_due() {
            self.start_check();
        }
        if self.session.icons_due()
            && let Err(e) = self.session.start(REFRESH_ICONS, Value::Null)
        {
            log::warn!("{e}");
        }
    }

    /// Apply background job progress; react to the jobs that ended.
    fn poll(&mut self, ctx: &egui::Context) {
        for event in self.session.poll_jobs() {
            match (&event.result, event.command.as_str()) {
                (Ok(v), CHECK) => {
                    self.notice = self.session.check_notice(v);
                    self.announce_updates();
                }
                // Icons failing is not worth the status bar: the monogram stays.
                (Ok(_), _) => {}
                (Err(e), _) => self.notice = Some(e.clone()),
            }
        }
        if self.session.has_jobs() {
            // Results arrive from worker threads; keep drawing until they are all in.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// An OS notification for updates the user hasn't heard about yet (each version once).
    fn announce_updates(&mut self) {
        if !self.session.settings().notifications {
            return;
        }
        let Some(notify) = &self.services.notify else { return };
        let fresh = self.session.take_new_updates();
        if let Some(text) = updates_text(&fresh) {
            let title = if fresh.len() == 1 { "Update available" } else { "Updates available" };
            notify(title, &text);
        }
    }

    /// Closing the window hides it instead, while there is a tray icon, the setting asks for
    /// it and the user didn't choose Quit.
    fn close_to_tray(&mut self, ctx: &egui::Context) {
        let wants_close = ctx.input(|i| i.viewport().close_requested());
        if wants_close && self.services.tray && self.session.settings().close_to_tray && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    /// Everything that isn't drawing: also runs while the window is hidden.
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        self.background();
        self.close_to_tray(ctx);
        ctx.request_repaint_after(IDLE_TICK);
    }

    /// `app`'s icon as a texture, uploaded once per icon revision.
    pub fn icon_texture(&mut self, ctx: &egui::Context, app: &str) -> Option<egui::TextureHandle> {
        let image = self.session.icon(app)?;
        if let Some((rev, tex)) = self.textures.get(app)
            && *rev == image.revision
        {
            return Some(tex.clone());
        }
        let size = [usize::try_from(image.width).ok()?, usize::try_from(image.height).ok()?];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
        let tex = ctx.load_texture(format!("icon-{app}"), color, egui::TextureOptions::LINEAR);
        self.textures.insert(app.to_string(), (image.revision, tex.clone()));
        Some(tex)
    }

    /// Draw one frame into `ui` (the eframe root, or a test harness).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        let t = Tokens::get(ui.ctx());
        egui::Panel::top("header").frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(16, 12))).show(ui, |ui| self.header(ui, &t));
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ui, |ui| self.status_bar(ui, &t));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.bg).inner_margin(egui::Margin::symmetric(8, 8))).show(ui, |ui| {
            match (self.ui.tab, self.ui.selected.clone()) {
                (Tab::Apps, Some(app)) if self.session.catalog().get(&app).is_some() => details::show(self, ui, &t, &app),
                (Tab::Apps, _) => app_list::show(self, ui, &t),
                (Tab::Settings, _) => settings_ui::show(self, ui, &t),
            }
        });
        // Links in release notes come from each craft's releases: only web links may open.
        ui.ctx().output_mut(|o| o.commands.retain(|c| !matches!(c, egui::OutputCommand::OpenUrl(u) if !safe_url(&u.url))));
    }

    fn header(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("ArtCraft Toolbox").heading().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for tab in [Tab::Settings, Tab::Apps] {
                    if ui.selectable_label(self.ui.tab == tab, tab.label()).clicked() {
                        // The Apps tab always leads back to the list.
                        if tab == Tab::Apps {
                            self.ui.selected = None;
                        }
                        self.ui.tab = tab;
                    }
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        ui.horizontal(|ui| {
            if let Some(n) = &self.notice {
                ui.add(egui::Label::new(egui::RichText::new(n).small().color(t.danger)).truncate()).on_hover_text(n);
                return;
            }
            // The version first, on the right; the status line gets the rest and ends in an
            // ellipsis in a narrow window.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(artcraft_toolbox_engine::build_info::long_version()).small().color(t.text_faint));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let line = status_line(&self.session);
                    ui.add(egui::Label::new(egui::RichText::new(&line).small().color(t.text_dim)).truncate()).on_hover_text(line);
                });
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

/// The notification text for new updates: `PhotoCraft 0.5.0, VectorCraft 0.7.0`.
pub fn updates_text(updates: &[(String, artcraft_toolbox_engine::Version)]) -> Option<String> {
    match updates {
        [] => None,
        list if list.len() <= 3 => Some(list.iter().map(|(name, v)| format!("{name} {v}")).collect::<Vec<_>>().join(", ")),
        [first, second, rest @ ..] => Some(format!("{} {}, {} {} and {} more", first.0, first.1, second.0, second.1, rest.len())),
        _ => None,
    }
}

/// Only `https://` and `http://` links leave the app.
pub fn safe_url(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.chars().any(char::is_control)
}

impl eframe::App for ToolboxApp {
    /// Runs before every frame and, while the window is hidden, whenever a repaint is requested
    /// (by [`ToolboxApp::tick`]'s idle timer, a job, or the tray).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artcraft_toolbox_engine::Version;

    #[test]
    fn notification_text() {
        let v = |n: &str, x| (n.to_string(), Version::new(0, x, 0));
        assert_eq!(updates_text(&[]), None);
        assert_eq!(updates_text(&[v("PhotoCraft", 5)]).as_deref(), Some("PhotoCraft 0.5.0"));
        assert_eq!(updates_text(&[v("A", 1), v("B", 2), v("C", 3)]).as_deref(), Some("A 0.1.0, B 0.2.0, C 0.3.0"));
        assert_eq!(updates_text(&[v("A", 1), v("B", 2), v("C", 3), v("D", 4)]).as_deref(), Some("A 0.1.0, B 0.2.0 and 2 more"));
    }

    #[test]
    fn only_web_links_open() {
        for ok in ["https://github.com/storytold/photocraft/pull/434", "http://example.com", "HTTPS://GitHub.com"] {
            assert!(safe_url(ok), "{ok}");
        }
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "ssh://x", "x-apple-helpviewer://", "/relative", "https://a\nb", ""] {
            assert!(!safe_url(bad), "{bad}");
        }
    }
}
