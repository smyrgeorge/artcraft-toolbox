//! An app's page: what it is, its status, its own settings (channel, auto-update, a pinned
//! version) and its versions with their release notes. Settings changes are `app.settings.set`
//! commands, like every other action.

use artcraft_toolbox_engine::{AppStatus, Status};
use artcraft_toolbox_release::asset;
use serde_json::{Value, json};

use crate::actions::{self, Clicked};
use crate::i18n::{fmt, tn};
use crate::theme::Tokens;
use crate::widgets::{self, app_tile, card, panel, panel_title, subhead};
use crate::{ToolboxApp, wording};

/// Versions listed on the page (the feed asks GitHub for 20).
const MAX_VERSIONS: usize = 20;

pub fn show(app: &mut ToolboxApp, ui: &mut egui::Ui, t: &Tokens, id: &str) {
    let Ok(status) = app.session.app_status(id) else {
        app.ui.selected = None;
        return;
    };
    let back = widgets::icon_button(ui, "arrow-left", tl!("Back"), false, t).on_hover_text(tl!("All apps"));
    if back.clicked() {
        app.ui.selected = None;
        return;
    }
    ui.add_space(2.0);
    let icon = app.icon_texture(ui.ctx(), id);
    let mut change: Option<(&'static str, Value)> = None;
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        clicked = header(app, ui, &status, icon.as_ref(), t);
        ui.add_space(8.0);
        change = settings(app, ui, &status, t);
        ui.add_space(8.0);
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
}

enum HeaderClick {
    Action(Clicked),
    Uninstall,
}

/// "Uninstall PhotoCraft 0.5.0?", over everything else (asked from an app's page or its menu
/// in the list).
pub(crate) fn confirm_uninstall(app: &mut ToolboxApp, ui: &mut egui::Ui, id: &str) {
    let Ok(st) = app.session.app_status(id) else {
        app.ui.confirm_uninstall = None;
        return;
    };
    let st = &st;
    let version = app.session.inventory().current(&st.id).map(|i| i.version.to_string()).unwrap_or_default();
    let kept = app.session.inventory().previous(&st.id).len();
    let removed = match kept {
        0 => tl!("The app is removed.").to_string(),
        n => {
            tn(n as u64, "The app is removed, with {n} earlier version kept for rollback.", "The app is removed, with {n} earlier versions kept for rollback.")
        }
    };
    let t = Tokens::get(ui.ctx());
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new(("confirm-uninstall", &st.id))).show(ui.ctx(), |ui| {
        // 320 wide, or less in a narrow window (the frame adds its margins around it).
        ui.set_max_width((ui.ctx().content_rect().width() - 64.0).clamp(160.0, 320.0));
        ui.label(egui::RichText::new(fmt(tl!("Uninstall {app} {version}?"), &[("app", &st.name), ("version", &version)])).strong());
        ui.label(format!("{removed} {}", tl!("Your documents and its settings are kept.")));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new(tl!("Uninstall")).color(t.danger)).clicked() {
                answer = Some(true);
            }
            if ui.button(tl!("Cancel")).clicked() {
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
            app_tile(ui, &st.id, &st.name, icon, 56.0, None);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&st.name).heading().color(t.text));
                ui.add(egui::Label::new(egui::RichText::new(wording::tagline(&st.tagline)).color(t.text_dim)).wrap());
                let mut line = match (&job, &st.found) {
                    (Some(j), _) => actions::progress_text(j),
                    (None, Some(v)) if !actions::installed(st) => fmt(tl!("{version} installed outside the toolbox"), &[("version", &v.to_string())]),
                    (None, _) => wording::status(&st.status),
                };
                if let Some(pin) = &st.pinned {
                    line = fmt(tl!("{status} · pinned to {version}"), &[("status", &line), ("version", &pin.to_string())]);
                }
                if let Some(at) = st.checked_at {
                    line = fmt(tl!("{status} · checked {when}"), &[("status", &line), ("when", &wording::ago(app.session.now(), at))]);
                }
                ui.label(egui::RichText::new(line).small().color(t.text_dim));
                if let Some(trust) = app.session.inventory().current(&st.id).and_then(|i| i.trust.as_ref()) {
                    ui.label(egui::RichText::new(wording::trust(trust)).small().color(t.text_dim));
                }
                if let Some(j) = &job {
                    actions::bar(ui, j, ui.available_width().min(240.0), t);
                }
                if let Some(e) = &st.error {
                    ui.label(egui::RichText::new(fmt(tl!("Last check failed: {error}"), &[("error", wording::engine(e))])).small().color(t.danger));
                }
                ui.horizontal(|ui| {
                    ui.hyperlink_to("GitHub", entry.repo_url());
                    ui.hyperlink_to(tl!("Releases"), format!("{}/releases", entry.repo_url()));
                    if let Some(w) = &entry.website {
                        ui.hyperlink_to(tl!("Website"), w);
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
            if matches!(st.status, Status::UpdateAvailable { .. })
                && job.is_none()
                && ui.button(tl!("Open")).on_hover_text(fmt(tl!("Open {app}"), &[("app", &st.name)])).clicked()
            {
                clicked = Some(HeaderClick::Action(Clicked::Open));
            }
            if actions::installed(st) && job.is_none() {
                let reason = app.session.disabled_reason(artcraft_toolbox_engine::install_cmds::UNINSTALL);
                let b = ui.add_enabled(reason.is_none(), egui::Button::new(tl!("Uninstall")));
                if b.clicked() {
                    clicked = Some(HeaderClick::Uninstall);
                }
                if let Some(why) = reason {
                    b.on_disabled_hover_text(wording::engine(&why));
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
    card(ui, t, |ui| {
        panel_title(ui, &fmt(tl!("Settings for {app}"), &[("app", &st.name)]), t);
        ui.add_space(4.0);
        egui::Grid::new(("app-settings", &st.id)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(tl!("Channel"));
            let default = fmt(tl!("Default ({value})"), &[("value", channel_name(global.channel))]);
            let current = own.channel.map_or(default.clone(), |c| channel_name(c).to_string());
            egui::ComboBox::from_id_salt(("channel", &st.id)).selected_text(current).show_ui(ui, |ui| {
                for (label, value) in [(default.as_str(), Value::Null), (tl!("Stable"), json!("stable")), (tl!("Pre-release"), json!("prerelease"))] {
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

            ui.label(tl!("Updates"));
            let default = fmt(tl!("Default ({value})"), &[("value", if global.auto_update { tl!("automatic") } else { tl!("ask first") })]);
            let current = own.auto_update.map_or(default.clone(), |b| if b { tl!("Automatic").into() } else { tl!("Ask first").into() });
            egui::ComboBox::from_id_salt(("auto", &st.id)).selected_text(current).show_ui(ui, |ui| {
                for (label, value) in [(default.as_str(), Value::Null), (tl!("Automatic"), json!(true)), (tl!("Ask first"), json!(false))] {
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

            ui.label(tl!("Version"));
            let current = own.pinned.as_ref().map_or(tl!("Latest").to_string(), |v| fmt(tl!("Pinned to {version}"), &[("version", &v.to_string())]));
            egui::ComboBox::from_id_salt(("pin", &st.id)).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(own.pinned.is_none(), tl!("Latest")).clicked() && own.pinned.is_some() {
                    change = Some(("pinned", Value::Null));
                }
                for v in &versions {
                    let selected = own.pinned.as_ref().is_some_and(|p| &p.to_string() == v);
                    if ui.selectable_label(selected, fmt(tl!("Pin to {version}"), &[("version", v)])).clicked() && !selected {
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
        artcraft_toolbox_model::Channel::Stable => tl!("Stable"),
        artcraft_toolbox_model::Channel::Prerelease => tl!("Pre-release"),
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
        card(ui, t, |ui| {
            panel_title(ui, tl!("Versions"), t);
            ui.label(egui::RichText::new(tl!("Not checked yet: check for updates to see versions and release notes.")).color(t.text_dim));
        });
        return None;
    };
    let installed = session.inventory().current(&st.id).filter(|i| i.active).map(|i| i.version.clone());
    let busy = actions::installing(session, st).is_some();
    let mut clicked = None;
    let host = session.host();
    subhead(ui, tl!("Versions"), t);
    ui.add_space(2.0);
    panel(ui, t, |ui| {
        for (i, r) in feed.releases.iter().take(MAX_VERSIONS).enumerate() {
            let mut title = r.version.to_string();
            if let Some(d) = r.published_at.as_deref().and_then(date) {
                title.push_str(&format!(" · {d}"));
            }
            if r.is_prerelease() {
                title.push_str(&format!(" · {}", tl!("pre-release")));
            }
            let kept = installed.as_ref() != Some(&r.version) && session.inventory().get(&st.id, &r.version).is_some();
            if installed.as_ref() == Some(&r.version) {
                title.push_str(&format!(" · {}", tl!("in use")));
            } else if kept {
                title.push_str(&format!(" · {}", tl!("kept")));
            }
            if st.pinned.as_ref() == Some(&r.version) {
                title.push_str(&format!(" · {}", tl!("pinned")));
            }
            let buildable = host.is_some_and(|h| asset::select(&r.assets, h, |a| &a.name).is_some());
            if !buildable {
                title.push_str(&format!(" · {}", tl!("no build for this computer")));
            }
            if i > 0 {
                ui.separator();
            }
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::CollapsingHeader::new(egui::RichText::new(title).font(crate::theme::medium(13.0)))
                    .id_salt(("release", &st.id, i))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        let label = if kept { tl!("Switch to this version") } else { tl!("Install this version") };
                        if installed.as_ref() != Some(&r.version) && (kept || (buildable && st.found.is_none())) {
                            let hover = if kept {
                                fmt(
                                    tl!("Make {app} {version} the version in use, without downloading it"),
                                    &[("app", &st.name), ("version", &r.version.to_string())],
                                )
                            } else {
                                fmt(tl!("Download and install {app} {version}"), &[("app", &st.name), ("version", &r.version.to_string())])
                            };
                            if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(hover).clicked() {
                                clicked = Some(Clicked::UseVersion(r.version.clone()));
                            }
                        }
                        if r.notes.trim().is_empty() {
                            ui.label(egui::RichText::new(tl!("No release notes.")).color(t.text_dim));
                        } else {
                            egui_commonmark::CommonMarkViewer::new().show(ui, markdown, &autolink(&r.notes));
                        }
                        if let Some(url) = &r.page_url {
                            ui.hyperlink_to(tl!("Open on GitHub"), url);
                        }
                    });
            });
        }
    });
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
