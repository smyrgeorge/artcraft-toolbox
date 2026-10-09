//! An app's page: what it is, its status, its own settings (channel, auto-update, a pinned
//! version) and its versions with their release notes. Settings changes are `app.settings.set`
//! commands, like every other action.

use artcraft_toolbox_engine::{AppStatus, Status};
use artcraft_toolbox_release::asset;
use serde_json::{Value, json};

use crate::ToolboxApp;
use crate::actions::{self, Clicked};
use crate::theme::Tokens;
use crate::widgets::{app_tile, card, section, subhead};

/// Versions listed on the page (the feed asks GitHub for 20).
const MAX_VERSIONS: usize = 20;

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens, id: &str) {
    let Ok(status) = app.session.app_status(id) else {
        app.ui.selected = None;
        return;
    };
    if ui.button("Back").on_hover_text("All apps").clicked() {
        app.ui.selected = None;
        return;
    }
    ui.add_space(4.0);
    let icon = app.icon_texture(ui.ctx(), id);
    let mut change: Option<(&'static str, Value)> = None;
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        clicked = header(app, ui, &status, icon.as_ref(), t);
        ui.add_space(8.0);
        change = settings(app, ui, &status, t);
        ui.add_space(4.0);
        if let Some(c) = versions(app, ui, &status, t) {
            clicked = Some(HeaderClick::Action(c));
        }
    });
    if let Some((key, value)) = change {
        let mut params = json!({ "app": id });
        params[key] = value;
        app.run("app.settings.set", params);
    }
    match clicked {
        Some(HeaderClick::Action(c)) => actions::perform(app, id, c),
        Some(HeaderClick::Uninstall) => app.ui.confirm_uninstall = Some(id.to_string()),
        None => {}
    }
    if app.ui.confirm_uninstall.as_deref() == Some(id) {
        confirm_uninstall(app, ui, &status);
    }
}

enum HeaderClick {
    Action(Clicked),
    Uninstall,
}

/// "Uninstall PhotoCraft 0.5.0?", over everything else.
fn confirm_uninstall(app: &mut ToolboxApp, ui: &mut egui::Ui, st: &AppStatus) {
    let version = app.session.inventory().current(&st.id).map(|i| i.version.to_string()).unwrap_or_default();
    let kept = app.session.inventory().previous(&st.id).len();
    let removed = match kept {
        0 => "The app is removed.".to_string(),
        1 => "The app is removed, with the earlier version kept for rollback.".to_string(),
        n => format!("The app is removed, with the {n} earlier versions kept for rollback."),
    };
    let t = Tokens::get(ui.ctx());
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new(("confirm-uninstall", &st.id))).show(ui.ctx(), |ui| {
        // 320 wide, or less in a narrow window (the frame adds its margins around it).
        ui.set_max_width((ui.ctx().content_rect().width() - 64.0).clamp(160.0, 320.0));
        ui.label(egui::RichText::new(format!("Uninstall {} {version}?", st.name)).strong());
        ui.label(format!("{removed} Your documents and its settings are kept."));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("Uninstall").color(t.danger)).clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    // Escape or a click outside is a Cancel.
    if answer.is_none() && modal.should_close() {
        answer = Some(false);
    }
    if let Some(yes) = answer {
        app.ui.confirm_uninstall = None;
        if yes {
            app.run(artcraft_toolbox_engine::install_cmds::UNINSTALL, json!({ "app": st.id }));
        }
    }
}

fn header(app: &ToolboxApp, ui: &mut egui::Ui, st: &AppStatus, icon: Option<&egui::TextureHandle>, t: &Tokens) -> Option<HeaderClick> {
    let entry = app.session.catalog().get(&st.id)?;
    let job = actions::installing(&app.session, st);
    let mut clicked = None;
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            app_tile(ui, &st.id, &st.name, icon, 56.0, t);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&st.name).heading().color(t.text));
                ui.add(egui::Label::new(egui::RichText::new(&st.tagline).color(t.text_dim)).wrap());
                let mut line = match (&job, &st.found) {
                    (Some(j), _) => actions::progress_text(j),
                    (None, Some(v)) if !actions::installed(st) => format!("{v} installed outside the toolbox"),
                    (None, _) => st.status.label(),
                };
                if let Some(pin) = &st.pinned {
                    line.push_str(&format!(" · pinned to {pin}"));
                }
                if let Some(at) = st.checked_at {
                    line.push_str(&format!(" · checked {}", artcraft_toolbox_engine::time::ago(app.session.now(), at)));
                }
                ui.label(egui::RichText::new(line).small().color(t.text_dim));
                if let Some(trust) = app.session.inventory().current(&st.id).and_then(|i| i.trust.as_ref()) {
                    ui.label(egui::RichText::new(trust.label()).small().color(t.text_dim));
                }
                if let Some(j) = &job {
                    actions::bar(ui, j, ui.available_width().min(240.0), t);
                }
                if let Some(e) = &st.error {
                    ui.label(egui::RichText::new(format!("Last check failed: {e}")).small().color(t.danger));
                }
                ui.horizontal(|ui| {
                    ui.hyperlink_to("GitHub", entry.repo_url());
                    ui.hyperlink_to("Releases", format!("{}/releases", entry.repo_url()));
                    if let Some(w) = &entry.website {
                        ui.hyperlink_to("Website", w);
                    }
                });
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if let Some(c) = actions::draw(ui, &app.session, st, t) {
                clicked = Some(HeaderClick::Action(c));
            }
            // The action is Update then; the app can still be opened.
            if matches!(st.status, Status::UpdateAvailable { .. }) && job.is_none() && ui.button("Open").on_hover_text(format!("Open {}", st.name)).clicked() {
                clicked = Some(HeaderClick::Action(Clicked::Open));
            }
            if actions::installed(st) && job.is_none() {
                let reason = app.session.disabled_reason(artcraft_toolbox_engine::install_cmds::UNINSTALL);
                let b = ui.add_enabled(reason.is_none(), egui::Button::new("Uninstall"));
                if b.clicked() {
                    clicked = Some(HeaderClick::Uninstall);
                }
                if let Some(why) = reason {
                    b.on_disabled_hover_text(why);
                }
            }
        });
    });
    clicked
}

/// The app's own settings. Returns the one change the user made this frame, if any.
fn settings(app: &ToolboxApp, ui: &mut egui::Ui, st: &AppStatus, t: &Tokens) -> Option<(&'static str, Value)> {
    let global = app.session.settings();
    let own = global.apps.get(&st.id).cloned().unwrap_or_default();
    let versions: Vec<String> =
        app.session.feed(&st.id).map(|f| f.releases.iter().take(MAX_VERSIONS).map(|r| r.version.to_string()).collect()).unwrap_or_default();
    let mut change = None;
    subhead(ui, &format!("Settings for {}", st.name), t);
    card(ui, t, |ui| {
        egui::Grid::new(("app-settings", &st.id)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Channel");
            let default = format!("Default ({})", channel_name(global.channel));
            let current = own.channel.map_or(default.clone(), |c| channel_name(c).to_string());
            egui::ComboBox::from_id_salt(("channel", &st.id)).selected_text(current).show_ui(ui, |ui| {
                for (label, value) in [(default.as_str(), Value::Null), ("Stable", json!("stable")), ("Pre-release", json!("prerelease"))] {
                    let selected = match (&own.channel, &value) {
                        (None, Value::Null) => true,
                        (Some(c), Value::String(v)) => channel_key(*c) == v,
                        _ => false,
                    };
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        change = Some(("channel", value));
                    }
                }
            });
            ui.end_row();

            ui.label("Updates");
            let default = format!("Default ({})", if global.auto_update { "automatic" } else { "ask first" });
            let current = own.auto_update.map_or(default.clone(), |b| if b { "Automatic".into() } else { "Ask first".into() });
            egui::ComboBox::from_id_salt(("auto", &st.id)).selected_text(current).show_ui(ui, |ui| {
                for (label, value) in [(default.as_str(), Value::Null), ("Automatic", json!(true)), ("Ask first", json!(false))] {
                    let selected = match (&own.auto_update, &value) {
                        (None, Value::Null) => true,
                        (Some(b), Value::Bool(v)) => b == v,
                        _ => false,
                    };
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        change = Some(("autoUpdate", value));
                    }
                }
            });
            ui.end_row();

            ui.label("Version");
            let current = own.pinned.as_ref().map_or("Latest".to_string(), |v| format!("Pinned to {v}"));
            egui::ComboBox::from_id_salt(("pin", &st.id)).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(own.pinned.is_none(), "Latest").clicked() && own.pinned.is_some() {
                    change = Some(("pinned", Value::Null));
                }
                for v in &versions {
                    let selected = own.pinned.as_ref().is_some_and(|p| &p.to_string() == v);
                    if ui.selectable_label(selected, format!("Pin to {v}")).clicked() && !selected {
                        change = Some(("pinned", json!(v)));
                    }
                }
            });
            ui.end_row();
        });
    });
    change
}

fn channel_name(c: artcraft_toolbox_model::Channel) -> &'static str {
    match c {
        artcraft_toolbox_model::Channel::Stable => "Stable",
        artcraft_toolbox_model::Channel::Prerelease => "Pre-release",
    }
}

fn channel_key(c: artcraft_toolbox_model::Channel) -> &'static str {
    match c {
        artcraft_toolbox_model::Channel::Stable => "stable",
        artcraft_toolbox_model::Channel::Prerelease => "prerelease",
    }
}

/// GitHub links bare URLs in release notes; CommonMark doesn't. Wrap each bare `http(s)://`
/// URL that starts a word as an autolink (`<https://…>`), leaving trailing punctuation outside.
/// Code (fenced blocks and inline spans), existing links and anything not a bare word are left
/// alone.
pub fn autolink(notes: &str) -> String {
    let mut out = String::with_capacity(notes.len() + 64);
    let mut fenced = false;
    for (i, line) in notes.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced || line.contains('`') {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        let mut at_word_start = true;
        while !rest.is_empty() {
            let lower = rest.get(..8).unwrap_or(rest).to_ascii_lowercase();
            if at_word_start && (lower.starts_with("https://") || lower.starts_with("http://")) {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let word = rest.get(..end).unwrap_or(rest);
                let url = word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '"']);
                if url.contains(['<', '>']) || url.len() <= "https://".len() {
                    out.push_str(word);
                } else {
                    out.push('<');
                    out.push_str(url);
                    out.push('>');
                    out.push_str(word.get(url.len()..).unwrap_or(""));
                }
                rest = rest.get(end..).unwrap_or("");
                at_word_start = false;
                continue;
            }
            let Some(c) = rest.chars().next() else { break };
            out.push(c);
            at_word_start = c.is_whitespace();
            rest = rest.get(c.len_utf8()..).unwrap_or("");
        }
    }
    out
}

/// `2026-10-08` from GitHub's `2026-10-08T14:52:50Z`.
pub fn date(published_at: &str) -> Option<&str> {
    published_at.get(..10).filter(|d| d.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() }))
}

/// Every release, newest first, with its notes; a version that isn't in use can be switched to
/// (kept on this computer) or installed. Returns that click.
fn versions(app: &mut ToolboxApp, ui: &mut egui::Ui, st: &AppStatus, t: &Tokens) -> Option<Clicked> {
    let ToolboxApp { session, markdown, .. } = app;
    let Some(feed) = session.feed(&st.id) else {
        section(ui, "Versions", 0, t);
        ui.label(egui::RichText::new("Not checked yet: check for updates to see versions and release notes.").color(t.text_dim));
        return None;
    };
    let installed = session.inventory().current(&st.id).filter(|i| i.active).map(|i| i.version.clone());
    let busy = actions::installing(session, st).is_some();
    let mut clicked = None;
    let host = session.host();
    let shown = feed.releases.len().min(MAX_VERSIONS);
    section(ui, "Versions", shown, t);
    for (i, r) in feed.releases.iter().take(MAX_VERSIONS).enumerate() {
        let mut title = r.version.to_string();
        if let Some(d) = r.published_at.as_deref().and_then(date) {
            title.push_str(&format!(" · {d}"));
        }
        if r.is_prerelease() {
            title.push_str(" · pre-release");
        }
        let kept = installed.as_ref() != Some(&r.version) && session.inventory().get(&st.id, &r.version).is_some();
        if installed.as_ref() == Some(&r.version) {
            title.push_str(" · in use");
        } else if kept {
            title.push_str(" · kept");
        }
        if st.pinned.as_ref() == Some(&r.version) {
            title.push_str(" · pinned");
        }
        let buildable = host.is_some_and(|h| asset::select(&r.assets, h, |a| &a.name).is_some());
        if !buildable {
            title.push_str(" · no build for this computer");
        }
        card(ui, t, |ui| {
            egui::CollapsingHeader::new(egui::RichText::new(title).strong()).id_salt(("release", &st.id, i)).default_open(i == 0).show(ui, |ui| {
                let label = if kept { "Switch to this version" } else { "Install this version" };
                if installed.as_ref() != Some(&r.version) && (kept || (buildable && st.found.is_none())) {
                    let hover = if kept {
                        format!("Make {} {} the version in use, without downloading it", st.name, r.version)
                    } else {
                        format!("Download and install {} {}", st.name, r.version)
                    };
                    if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(hover).clicked() {
                        clicked = Some(Clicked::UseVersion(r.version.clone()));
                    }
                }
                if r.notes.trim().is_empty() {
                    ui.label(egui::RichText::new("No release notes.").color(t.text_dim));
                } else {
                    egui_commonmark::CommonMarkViewer::new().show(ui, markdown, &autolink(&r.notes));
                }
                if let Some(url) = &r.page_url {
                    ui.hyperlink_to("Open on GitHub", url);
                }
            });
        });
        ui.add_space(4.0);
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::{autolink, date};

    #[test]
    fn bare_urls_become_links_and_nothing_else_changes() {
        assert_eq!(autolink("* Fix by @x in https://github.com/o/r/pull/1"), "* Fix by @x in <https://github.com/o/r/pull/1>");
        assert_eq!(autolink("See https://example.com/a."), "See <https://example.com/a>.");
        assert_eq!(autolink("(https://example.com)"), "(https://example.com)", "not at a word start: left alone");
        assert_eq!(autolink("[text](https://example.com)"), "[text](https://example.com)");
        assert_eq!(autolink("<https://example.com>"), "<https://example.com>");
        assert_eq!(autolink("`https://example.com`"), "`https://example.com`");
        assert_eq!(autolink("```\nhttps://example.com\n```"), "```\nhttps://example.com\n```");
        assert_eq!(autolink("javascript:alert(1) file:///etc"), "javascript:alert(1) file:///etc");
        assert_eq!(autolink("https:// x"), "https:// x");
        assert_eq!(autolink("é https://a.b/ü!\nnext"), "é <https://a.b/ü>!\nnext");
        assert_eq!(autolink(""), "");
    }

    #[test]
    fn dates() {
        assert_eq!(date("2026-10-08T14:52:50Z"), Some("2026-10-08"));
        for bad in ["", "2026", "yesterday at noon", "2026/10/08T00:00", "é2026-10-08"] {
            assert_eq!(date(bad), None, "{bad}");
        }
    }
}
