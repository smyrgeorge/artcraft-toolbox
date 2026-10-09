//! ArtCraft Toolbox's UI shell, built on egui/eframe.
//!
//! Deliberately thin: everything it shows comes from the [`Session`], and every action it takes
//! is an engine command run through [`Session::execute`] or [`Session::start`]. UI-only state
//! (the open tab, the search text, the app whose details are open) lives in [`state::UiState`],
//! plain serde data, so automation can read and drive it. What only the platform can do (OS
//! notifications, a menu-bar or tray icon) comes in through [`Services`], from the app.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Translate a string literal into the current UI language: `tl!("Check for updates")`
/// (PhotoCraft's pattern; see [`i18n`]).
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod actions;
pub mod app_list;
pub mod cjk;
pub mod cjk_fonts;
pub mod details;
pub mod i18n;
pub mod settings_ui;
pub mod state;
pub mod theme;
pub mod widgets;
pub mod wording;

use std::collections::HashMap;
use std::time::Duration;

use artcraft_toolbox_engine::icons::REFRESH_ICONS;
use artcraft_toolbox_engine::update_cmds::CHECK;
use artcraft_toolbox_engine::versions_cmds::UPDATE_ALL;
use artcraft_toolbox_engine::{JobId, Session, Started};
use serde_json::{Value, json};

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
    /// Updates started automatically (`autoUpdate`): announced when they finish.
    auto_updates: std::collections::HashSet<JobId>,
    /// The `textSize` setting last applied as the zoom factor.
    applied_text_size: Option<u32>,
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
            auto_updates: std::collections::HashSet::new(),
            applied_text_size: None,
        }
    }

    /// Follow the appearance settings: the language (also used for notifications, so it runs
    /// while the window is hidden too), the theme and the text size.
    pub fn sync_appearance(&mut self, ctx: &egui::Context) {
        let s = self.session.settings();
        i18n::sync_context(ctx, &s.language);
        theme::set_preference(ctx, s.theme);
        if self.applied_text_size != Some(s.text_size) {
            self.applied_text_size = Some(s.text_size);
            ctx.set_zoom_factor(s.text_size as f32 / 100.0);
        }
    }

    /// Cmd/Ctrl with +, - and 0 step the `textSize` setting (saved, unlike egui's own zoom).
    fn text_size_keys(&mut self, ctx: &egui::Context) {
        use artcraft_toolbox_model::settings::TEXT_SIZES;
        use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
        let current = self.session.settings().text_size;
        let at = TEXT_SIZES.iter().position(|n| *n == current).unwrap_or(1);
        let wanted = ctx.input_mut(|i| {
            if i.consume_shortcut(&ZOOM_IN) || i.consume_shortcut(&ZOOM_IN_SECONDARY) {
                TEXT_SIZES.get(at + 1).copied()
            } else if i.consume_shortcut(&ZOOM_OUT) {
                at.checked_sub(1).and_then(|i| TEXT_SIZES.get(i)).copied()
            } else if i.consume_shortcut(&ZOOM_RESET) {
                Some(100)
            } else {
                None
            }
        });
        if let Some(n) = wanted.filter(|n| *n != current) {
            self.run("settings.set", json!({ "textSize": n }));
        }
    }

    /// Update every app that has an update (`only_automatic`: those set to update automatically,
    /// after a check). Reports why an app was skipped in the status bar, for a request the user
    /// made; automatic updates are announced when they finish.
    pub fn update_all(&mut self, only_automatic: bool) {
        let out = if only_automatic {
            // Quietly: a session that can't update (or nothing to do) leaves the status bar alone.
            if self.session.disabled_reason(UPDATE_ALL).is_some() {
                return;
            }
            match self.session.execute(UPDATE_ALL, json!({ "onlyAutomatic": true })) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("automatic updates: {e}");
                    return;
                }
            }
        } else {
            let Some(out) = self.run(UPDATE_ALL, json!({})) else { return };
            out
        };
        let jobs = out["started"].as_array().into_iter().flatten().filter_map(|j| j["job"].as_u64()).map(JobId);
        if only_automatic {
            self.auto_updates.extend(jobs);
            for done in out["switched"].as_array().into_iter().flatten() {
                self.announce_updated(done["app"].as_str().unwrap_or_default(), done["version"].as_str().unwrap_or_default());
            }
            for skipped in out["skipped"].as_array().into_iter().flatten() {
                log::info!("automatic update of {} skipped: {}", skipped["app"], skipped["reason"]);
            }
        } else if let Some(first) = out["skipped"].as_array().and_then(|a| a.first()) {
            let name = self.session.catalog().get(first["app"].as_str().unwrap_or_default()).map(|a| a.name.clone()).unwrap_or_default();
            self.notice = Some(format!("{name}: {}", first["reason"].as_str().unwrap_or_default()));
        }
    }

    /// "PhotoCraft updated to 0.5.0", when notifications are on.
    fn announce_updated(&self, app: &str, version: &str) {
        if !self.session.settings().notifications {
            return;
        }
        let (Some(notify), Some(entry)) = (&self.services.notify, self.session.catalog().get(app)) else { return };
        notify(tl!("Updated"), &format!("{} {version}", entry.name));
    }

    /// Themes, style and the built-in fonts; call once on the egui context before the first
    /// frame. Text size has its own shortcuts ([`ToolboxApp::text_size_keys`]), not egui's zoom.
    pub fn setup_context(ctx: &egui::Context) {
        theme::apply(ctx);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
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
            if self.auto_updates.remove(&event.id) {
                match &event.result {
                    Ok(v) => self.announce_updated(v["app"].as_str().unwrap_or_default(), v["version"].as_str().unwrap_or_default()),
                    Err(e) => self.notice = Some(i18n::fmt(tl!("An automatic update failed: {error}"), &[("error", wording::engine(e))])),
                }
                continue;
            }
            match (&event.result, event.command.as_str()) {
                (Ok(v), CHECK) => {
                    self.notice = wording::check_notice(&self.session, v);
                    self.announce_updates();
                    self.update_all(true);
                }
                // Icons failing is not worth the status bar: the monogram stays.
                (Ok(_), _) => {}
                (Err(e), _) => self.notice = Some(wording::engine(e).to_string()),
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
        if let Some(text) = wording::updates_text(&fresh) {
            let title = if fresh.len() == 1 { tl!("Update available") } else { tl!("Updates available") };
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
        self.sync_appearance(ctx);
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
        self.sync_appearance(ui.ctx());
        self.text_size_keys(ui.ctx());
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
        // The tabs first, on the right; the title gets what is left (long tab names in some
        // languages leave it less in a narrow window).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (tab, label) in [(Tab::Settings, tl!("Settings")), (Tab::Apps, tl!("Apps"))] {
                if ui.selectable_label(self.ui.tab == tab, label).clicked() {
                    // The Apps tab always leads back to the list.
                    if tab == Tab::Apps {
                        self.ui.selected = None;
                    }
                    self.ui.tab = tab;
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new("ArtCraft Toolbox").heading().color(t.text)).truncate());
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        ui.horizontal(|ui| {
            if let Some(n) = &self.notice {
                let n = wording::engine(n);
                ui.add(egui::Label::new(egui::RichText::new(n).small().color(t.danger)).truncate()).on_hover_text(n);
                return;
            }
            // The version first, on the right; the status line gets the rest and ends in an
            // ellipsis in a narrow window.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(artcraft_toolbox_engine::build_info::long_version()).small().color(t.text_faint));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let line = wording::status_line(&self.session);
                    ui.add(egui::Label::new(egui::RichText::new(&line).small().color(t.text_dim)).truncate()).on_hover_text(line);
                });
            });
        });
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
        use wording::updates_text;
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
