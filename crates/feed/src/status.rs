//! An app's update status: what is installed here against what its feed offers this machine.

use artcraft_toolbox_model::Channel;
use artcraft_toolbox_release::{Target, Version, asset};
use serde::{Deserialize, Serialize};

use crate::{Asset, Release};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Status {
    /// No release information yet: never checked, or the last check failed.
    Unknown {
        installed: Option<Version>,
    },
    /// Not installed; `latest` can be installed here.
    NotInstalled {
        latest: Version,
    },
    UpToDate {
        installed: Version,
    },
    UpdateAvailable {
        installed: Version,
        latest: Version,
    },
    /// Releases exist, but none has a build for this machine on this channel.
    Unsupported {
        installed: Option<Version>,
    },
}

impl Status {
    /// Short user-facing text for the app list.
    pub fn label(&self) -> String {
        match self {
            Status::Unknown { installed: Some(v) } => format!("{v} installed"),
            Status::Unknown { installed: None } => "Not installed".into(),
            Status::NotInstalled { latest } => format!("{latest} available"),
            Status::UpToDate { installed } => format!("{installed} · up to date"),
            Status::UpdateAvailable { installed, latest } => format!("{latest} available · {installed} installed"),
            Status::Unsupported { .. } => "No build for this computer".into(),
        }
    }
}

/// The newest release `channel` offers that has an installable build for `host`, with that build.
/// With a `pin`, nothing newer than the pinned version is offered.
pub fn latest<'a>(releases: &'a [Release], channel: Channel, host: Target, pin: Option<&Version>) -> Option<(&'a Release, &'a Asset)> {
    let mut offered: Vec<&Release> = releases.iter().filter(|r| channel.offers(&r.version, r.prerelease) && pin.is_none_or(|p| r.version <= *p)).collect();
    offered.sort_by(|a, b| b.version.cmp(&a.version));
    offered.into_iter().find_map(|r| asset::select(&r.assets, host, |a| &a.name).map(|a| (r, a)))
}

/// Status of one app. `releases` is `None` until its feed has been fetched; `host` is `None` on a
/// platform no craft ships for; `pin` caps the version offered (an installed version above the pin
/// reads as up to date: pinning never offers a downgrade).
pub fn status(installed: Option<&Version>, releases: Option<&[Release]>, channel: Channel, host: Option<Target>, pin: Option<&Version>) -> Status {
    let installed = installed.cloned();
    let Some(releases) = releases else { return Status::Unknown { installed } };
    let Some((latest, _)) = host.and_then(|h| latest(releases, channel, h, pin)) else { return Status::Unsupported { installed } };
    let latest = latest.version.clone();
    match installed {
        None => Status::NotInstalled { latest },
        // Newer than the feed (e.g. a pre-release kept after switching to stable) is up to date.
        Some(i) if i >= latest => Status::UpToDate { installed: i },
        Some(i) => Status::UpdateAvailable { installed: i, latest },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_releases;
    use artcraft_toolbox_release::{Arch, Os};
    use serde_json::json;

    fn feed(tags: &[(&str, bool)]) -> Vec<Release> {
        let rels: Vec<serde_json::Value> = tags
            .iter()
            .map(|(tag, pre)| {
                let v = tag.trim_start_matches('v');
                let name = format!("photocraft-{v}-linux-x86_64.AppImage");
                json!({"tag_name": tag, "prerelease": pre, "assets": [{"name": name, "size": 1, "browser_download_url": format!("https://github.com/o/r/releases/download/{tag}/{name}")}]})
            })
            .collect();
        parse_releases(&json!(rels).to_string(), &["photocraft"]).unwrap()
    }

    const LINUX: Option<Target> = Some(Target::new(Os::Linux, Arch::X86_64));
    const MAC: Option<Target> = Some(Target::new(Os::Macos, Arch::Aarch64));

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn statuses() {
        let f = feed(&[("v0.5.0", false), ("v0.6.0-rc.1", true), ("v0.4.0", false)]);
        let st = |inst: Option<&str>, ch| status(inst.map(v).as_ref(), Some(&f), ch, LINUX, None);
        assert_eq!(st(None, Channel::Stable), Status::NotInstalled { latest: v("0.5.0") });
        assert_eq!(st(Some("0.4.0"), Channel::Stable), Status::UpdateAvailable { installed: v("0.4.0"), latest: v("0.5.0") });
        assert_eq!(st(Some("0.5.0"), Channel::Stable), Status::UpToDate { installed: v("0.5.0") });
        assert_eq!(st(Some("0.5.0"), Channel::Prerelease), Status::UpdateAvailable { installed: v("0.5.0"), latest: v("0.6.0-rc.1") });
        assert_eq!(st(Some("0.6.0-rc.1"), Channel::Stable), Status::UpToDate { installed: v("0.6.0-rc.1") });
    }

    #[test]
    fn unknown_and_unsupported() {
        assert_eq!(status(None, None, Channel::Stable, LINUX, None), Status::Unknown { installed: None });
        let f = feed(&[("v0.5.0", false)]);
        // Only Linux builds in this feed.
        assert_eq!(status(None, Some(&f), Channel::Stable, MAC, None), Status::Unsupported { installed: None });
        assert_eq!(status(None, Some(&f), Channel::Stable, None, None), Status::Unsupported { installed: None });
        assert_eq!(status(None, Some(&[]), Channel::Stable, LINUX, None), Status::Unsupported { installed: None });
    }

    #[test]
    fn latest_skips_releases_without_a_build_for_the_host() {
        let mut f = feed(&[("v0.5.0", false), ("v0.4.0", false)]);
        f[0].assets.clear();
        let (r, a) = latest(&f, Channel::Stable, Target::new(Os::Linux, Arch::X86_64), None).unwrap();
        assert_eq!(r.version, v("0.4.0"));
        assert_eq!(a.file, "photocraft-0.4.0-linux-x86_64.AppImage");
    }

    #[test]
    fn a_pin_caps_what_is_offered() {
        let f = feed(&[("v0.5.0", false), ("v0.4.0", false), ("v0.3.0", false)]);
        let pin = v("0.4.0");
        let st = |inst: Option<&str>| status(inst.map(v).as_ref(), Some(&f), Channel::Stable, LINUX, Some(&pin));
        assert_eq!(st(None), Status::NotInstalled { latest: v("0.4.0") });
        assert_eq!(st(Some("0.3.0")), Status::UpdateAvailable { installed: v("0.3.0"), latest: v("0.4.0") });
        assert_eq!(st(Some("0.4.0")), Status::UpToDate { installed: v("0.4.0") });
        assert_eq!(st(Some("0.5.0")), Status::UpToDate { installed: v("0.5.0") }, "never a downgrade");
        // A pin below every release offers nothing.
        let old = v("0.1.0");
        assert_eq!(status(None, Some(&f), Channel::Stable, LINUX, Some(&old)), Status::Unsupported { installed: None });
    }

    #[test]
    fn serializes_for_agents() {
        let s = Status::UpdateAvailable { installed: v("0.4.0"), latest: v("0.5.0") };
        assert_eq!(serde_json::to_value(&s).unwrap(), json!({"state": "updateAvailable", "installed": "0.4.0", "latest": "0.5.0"}));
        assert_eq!(s.label(), "0.5.0 available · 0.4.0 installed");
    }
}
