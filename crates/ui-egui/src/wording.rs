//! UI text made from data, in the current language: an app's status, "5 min ago", the status
//! line, notifications, a check's outcome, a signature. The engine keeps its own English wording
//! (`Status::label`, `time::ago`) for the CLI, logs and JSON; the UI builds its text here.
//!
//! Messages that come from the engine as text (errors, why a button is disabled) are shown in
//! English, like PhotoCraft's, except the fixed ones in [`ENGINE_STRINGS`], which are translated
//! by exact match ([`engine`]).

use artcraft_toolbox_engine::update_cmds::CheckSummary;
use artcraft_toolbox_engine::{SelfStatus, Session, Status, Trust, Version};
use serde_json::Value;

use crate::theme::Tokens;

use crate::i18n::{fmt, tn};

/// Fixed engine and network messages the UI shows and the catalogs translate. Each one is
/// checked to still exist in the engine's or net crate's source (`engine_strings_exist`).
pub const ENGINE_STRINGS: &[&str] = &[
    "this session has no network access",
    "this session has no install location",
    "a check is already running",
    "the icons are already being refreshed",
    "the toolbox is already updating",
    "the server took too long to answer",
    "GitHub's request limit is used up",
    "too many redirects",
    "cancelled",
    "the install stopped unexpectedly",
    // The phases of an install or update job.
    "Starting",
    "Verifying the release",
    "Downloading",
    "Verifying",
    "Installing",
    "Checking the signature",
];

/// An engine message in the current language when it is one of [`ENGINE_STRINGS`], else as is.
pub fn engine(s: &str) -> &str {
    crate::i18n::t(s)
}

/// The tray menu: Open, Check for Updates, Quit (in the current language).
pub fn tray_labels() -> [&'static str; 3] {
    [tl!("Open ArtCraft Toolbox"), tl!("Check for Updates"), tl!("Quit ArtCraft Toolbox")]
}

/// An app's one-line description from the catalog (`catalog.toml`), in the current language.
pub fn tagline(s: &str) -> &str {
    crate::i18n::t(s)
}

/// An app's status line.
pub fn status(s: &Status) -> String {
    match s {
        Status::Unknown { installed: Some(v) } => fmt(tl!("{version} installed"), &[("version", &v.to_string())]),
        Status::Unknown { installed: None } => tl!("Not installed").to_string(),
        Status::NotInstalled { latest } => fmt(tl!("{version} available"), &[("version", &latest.to_string())]),
        Status::UpToDate { installed } => fmt(tl!("{version} · up to date"), &[("version", &installed.to_string())]),
        Status::UpdateAvailable { installed, latest } => {
            fmt(tl!("{latest} available · {installed} installed"), &[("latest", &latest.to_string()), ("installed", &installed.to_string())])
        }
        Status::Unsupported { .. } => tl!("No build for this computer").to_string(),
    }
}

/// The toolbox's own update state, and its colour: `0.2.0 is available`,
/// `… is downloaded · restart to update`, `Up to date`, `Not checked yet`. The feed's
/// "no release for this computer" shows as not checked: the toolbox doesn't nag about its own
/// releases the way it does about an app's.
pub fn self_status(st: &SelfStatus, t: &Tokens) -> (String, egui::Color32) {
    if let Some(v) = &st.staged {
        return (fmt(tl!("{version} is downloaded · restart to update"), &[("version", &v.to_string())]), t.accent_fg);
    }
    match &st.status {
        Status::UpdateAvailable { latest, .. } => (fmt(tl!("{version} is available"), &[("version", &latest.to_string())]), t.accent_fg),
        Status::UpToDate { .. } => (tl!("Up to date").to_string(), t.text_dim),
        Status::Unknown { .. } | Status::Unsupported { .. } | Status::NotInstalled { .. } => (tl!("Not checked yet").to_string(), t.text_dim),
    }
}

/// How long ago `then` was: `just now`, `5 min ago`, `3 h ago`, `2 d ago`.
pub fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        d if d < 60 => tl!("just now").to_string(),
        d if d < 3600 => tn(d / 60, "{n} min ago", "{n} min ago"),
        d if d < 86_400 => tn(d / 3600, "{n} h ago", "{n} h ago"),
        d => tn(d / 86_400, "{n} day ago", "{n} days ago"),
    }
}

/// How long until `then`, rounded up: `in under a minute`, `in 20 min`, `in 2 h`.
pub fn until(now: u64, then: u64) -> String {
    match then.saturating_sub(now) {
        d if d < 60 => tl!("in under a minute").to_string(),
        d if d < 3600 => tn(d.div_ceil(60), "in {n} min", "in {n} min"),
        d => tn(d.div_ceil(3600), "in {n} h", "in {n} h"),
    }
}

/// `Checked 5 min ago · 12 apps · macos-aarch64`.
pub fn status_line(session: &Session) -> String {
    let checked = match session.last_checked() {
        Some(at) => fmt(tl!("Checked {when}"), &[("when", &ago(session.now(), at))]),
        None => tl!("Not checked yet").to_string(),
    };
    let host = session.host().map(|h| h.to_string()).unwrap_or_else(|| tl!("unsupported platform").to_string());
    let apps = tn(session.catalog().apps.len() as u64, "{n} app", "{n} apps");
    format!("{checked} · {apps} · {host}")
}

/// GitHub's rate limit, and when checking can resume.
pub fn rate_limited(now: u64, until_at: u64) -> String {
    fmt(
        tl!("GitHub's request limit for this computer is used up; checking can resume {when}. A GitHub token in {variable} raises the limit."),
        &[("when", &until(now, until_at)), ("variable", artcraft_toolbox_engine::setup::ENV_GITHUB_TOKEN)],
    )
}

/// What a finished check has to say (rate limit, failures), or nothing when all went well.
pub fn check_notice(session: &Session, result: &Value) -> Option<String> {
    let summary: CheckSummary = serde_json::from_value(result.clone()).ok()?;
    if let Some(until_at) = summary.rate_limited_until {
        return Some(rate_limited(session.now(), until_at));
    }
    let name = |id: &str| session.feed_app(id).map_or_else(|| id.to_string(), |a| a.name.clone());
    match summary.failed.as_slice() {
        [] => None,
        [f] => Some(fmt(tl!("Couldn't check {app}: {error}"), &[("app", &name(&f.app)), ("error", engine(&f.error))])),
        [f, ..] => {
            Some(fmt(&tn(summary.failed.len() as u64, "Couldn't check {n} app: {error}", "Couldn't check {n} apps: {error}"), &[("error", engine(&f.error))]))
        }
    }
}

/// The notification for new updates: `PhotoCraft 0.5.0, VectorCraft 0.7.0`, or the first two and
/// "and 3 more".
pub fn updates_text(updates: &[(String, Version)]) -> Option<String> {
    let each = |(name, v): &(String, Version)| format!("{name} {v}");
    match updates {
        [] => None,
        list if list.len() <= 3 => Some(list.iter().map(each).collect::<Vec<_>>().join(", ")),
        [first, second, rest @ ..] => Some(fmt(
            &tn(rest.len() as u64, "{first}, {second} and {n} more", "{first}, {second} and {n} more"),
            &[("first", &each(first)), ("second", &each(second))],
        )),
        _ => None,
    }
}

/// A version's platform signature.
pub fn trust(t: &Trust) -> String {
    match t {
        Trust::Signed { signer, notarized, .. } => {
            let who = signer.strip_prefix("Developer ID Application: ").unwrap_or(signer);
            let line = fmt(tl!("Signed by {who}"), &[("who", who)]);
            if *notarized { format!("{line} · {}", tl!("notarized")) } else { line }
        }
        Trust::Unsigned => tl!("No platform signature").to_string(),
        Trust::Unchecked { reason } => fmt(tl!("Signature not checked: {reason}"), &[("reason", reason)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_language};

    #[test]
    fn english_wording_matches_the_engine() {
        with_language(Lang::EN, || {
            let (a, b) = (Version::new(0, 3, 0), Version::new(0, 5, 0));
            for s in [
                Status::Unknown { installed: None },
                Status::Unknown { installed: Some(a.clone()) },
                Status::NotInstalled { latest: b.clone() },
                Status::UpToDate { installed: a.clone() },
                Status::UpdateAvailable { installed: a.clone(), latest: b.clone() },
                Status::Unsupported { installed: None },
            ] {
                assert_eq!(status(&s), s.label());
            }
            for d in [0, 59, 60, 61, 3599, 3600, 86_399, 86_400, 864_000] {
                assert_eq!(ago(d, 0).replace("1 day", "1 d").replace(" days", " d").replace(" day", " d"), artcraft_toolbox_engine::time::ago(d, 0));
                assert_eq!(until(0, d), artcraft_toolbox_engine::time::until(0, d));
            }
            let signed = Trust::Signed {
                signer: "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)".into(),
                team: Some("DJ6XS33FX8".into()),
                notarized: true,
            };
            for t in [signed, Trust::Unsigned, Trust::Unchecked { reason: "x".into() }] {
                assert_eq!(trust(&t), t.label());
            }
            let v = |n: &str| (n.to_string(), Version::new(1, 0, 0));
            assert_eq!(updates_text(&[v("A"), v("B"), v("C"), v("D"), v("E")]).unwrap(), "A 1.0.0, B 1.0.0 and 3 more");
            assert_eq!(updates_text(&[v("A")]).unwrap(), "A 1.0.0");
            assert_eq!(updates_text(&[]), None);
        });
    }

    #[test]
    fn engine_strings_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut source = String::new();
        for dir in ["engine/src", "net/src"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
                source.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
            }
        }
        for s in ENGINE_STRINGS {
            assert!(source.contains(&format!("\"{s}\"")), "{s:?} is no longer in the engine or net crate: update ENGINE_STRINGS");
        }
    }
}
