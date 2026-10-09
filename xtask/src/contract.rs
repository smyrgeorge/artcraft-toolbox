//! `cargo xtask contract`: check every catalog app's latest live release against the release
//! contract (docs/release-contract.md) with the toolbox's own parsers.
//!
//! The toolbox only works while every craft keeps publishing the same way. This turns "do they
//! still?" into a command (and a scheduled CI job, `.github/workflows/contract.yml`), so a change
//! in a craft's release pipeline shows up here before users hit it.
//!
//! Per app, the newest stable release must have: a `SHA256SUMS.txt` listing every asset, no asset
//! names outside the contract, and an installable desktop build for every host in [`HOSTS`].
//! Network goes through `curl` (anonymous GitHub API: 60 requests/hour; `GITHUB_TOKEN` raises it).

use artcraft_toolbox_catalog::{App, Catalog};
use artcraft_toolbox_feed::Release;
use artcraft_toolbox_model::Channel;
use artcraft_toolbox_release::{Arch, Checksums, Os, PackageKind, Target};
use std::process::Command;

/// Every host the toolbox supports, with the package it should install there.
pub const HOSTS: &[(Os, Arch, PackageKind)] = &[
    (Os::Macos, Arch::Aarch64, PackageKind::Dmg),
    (Os::Macos, Arch::X86_64, PackageKind::Dmg),
    (Os::Windows, Arch::X86_64, PackageKind::PortableZip),
    (Os::Windows, Arch::Aarch64, PackageKind::PortableZip),
    (Os::Linux, Arch::X86_64, PackageKind::AppImage),
    (Os::Linux, Arch::Aarch64, PackageKind::AppImage),
];

/// Generic, like every craft's dev tooling: never a person's name or email in requests.
const USER_AGENT: &str = "ArtCraft-Toolbox-dev";

pub fn run(args: &[&str]) -> Result<(), String> {
    let mut only: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--app" => only.push(it.next().ok_or("--app needs an id")?),
            other => return Err(format!("unexpected argument `{other}` (usage: cargo xtask contract [--app <id>]...)")),
        }
    }
    let catalog = Catalog::builtin().map_err(|e| e.to_string())?;
    for id in &only {
        if catalog.get(id).is_none() {
            return Err(format!("`{id}` is not in crates/catalog/catalog.toml"));
        }
    }
    let apps: Vec<&App> = catalog.apps.iter().filter(|a| only.is_empty() || only.contains(&a.id.as_str())).collect();
    let mut failed = 0;
    for app in &apps {
        match check_app(app) {
            Ok(summary) => println!("ok    {:<12} {summary}", app.id),
            Err(problems) => {
                failed += 1;
                println!("FAIL  {:<12} {}", app.id, problems.first().map(String::as_str).unwrap_or(""));
                for p in problems.iter().skip(1) {
                    println!("      {:<12} {p}", "");
                }
            }
        }
    }
    println!();
    if failed == 0 {
        println!("OK: {} app(s) follow the release contract.", apps.len());
        Ok(())
    } else {
        Err(format!("{failed} of {} app(s) break the release contract (docs/release-contract.md)", apps.len()))
    }
}

/// `Ok(summary)` or every problem found.
fn check_app(app: &App) -> Result<String, Vec<String>> {
    let json = fetch(&format!("{}?per_page=10", app.releases_api_url())).map_err(|e| vec![e])?;
    let releases = artcraft_toolbox_feed::parse_releases(&json, &app.slugs()).map_err(|e| vec![e.to_string()])?;
    let Some(rel) = releases.iter().find(|r| Channel::Stable.offers(&r.version, r.prerelease)) else {
        return Err(vec!["no stable release".into()]);
    };
    let mut problems = check_release(rel);
    match &rel.checksums_url {
        None => problems.push(format!("{}: no SHA256SUMS.txt", rel.tag)),
        Some(url) => match fetch(url).map_err(|e| format!("SHA256SUMS.txt: {e}")).and_then(|t| Checksums::parse(&t).map_err(|e| e.to_string())) {
            Err(e) => problems.push(format!("{}: {e}", rel.tag)),
            Ok(sums) => {
                for a in rel.assets.iter().filter(|a| sums.get(&a.file).is_none()) {
                    problems.push(format!("{}: {} is not in SHA256SUMS.txt", rel.tag, a.file));
                }
            }
        },
    }
    if problems.is_empty() { Ok(format!("{} ({} assets, {} hosts)", rel.tag, rel.assets.len(), HOSTS.len())) } else { Err(problems) }
}

/// Contract checks that need no further download.
fn check_release(rel: &Release) -> Vec<String> {
    let mut problems: Vec<String> = rel.unrecognized.iter().map(|n| format!("{}: asset `{n}` doesn't follow the naming contract", rel.tag)).collect();
    for (os, arch, kind) in HOSTS {
        let host = Target::new(*os, *arch);
        match artcraft_toolbox_release::asset::select(&rel.assets, host, |a| &a.name) {
            None => problems.push(format!("{}: no installable build for {host}", rel.tag)),
            Some(a) if a.name.kind != *kind => problems.push(format!("{}: {host} would get {:?}, expected {kind:?}", rel.tag, a.name.kind)),
            Some(_) => {}
        }
    }
    problems
}

fn fetch(url: &str) -> Result<String, String> {
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--proto", "=https", "--retry", "2", "-A", USER_AGENT, "-H", "Accept: application/vnd.github+json"]);
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.trim().is_empty()
        && url.starts_with("https://api.github.com/")
    {
        c.args(["-H", &format!("Authorization: Bearer {}", token.trim())]);
    }
    let out = c.arg(url).output().map_err(|e| format!("curl: {e} (is curl installed?)"))?;
    if !out.status.success() {
        return Err(format!("GET {url}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    String::from_utf8(out.stdout).map_err(|_| format!("GET {url}: response is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_photocraft_release_passes_the_offline_checks() {
        let json = include_str!("../../crates/feed/tests/fixtures/photocraft-releases.json");
        let releases = artcraft_toolbox_feed::parse_releases(json, &["photocraft"]).unwrap();
        assert_eq!(check_release(&releases[0]), Vec::<String>::new());
    }

    #[test]
    fn missing_builds_are_reported_per_host() {
        let json = include_str!("../../crates/feed/tests/fixtures/photocraft-releases.json");
        let mut rel = artcraft_toolbox_feed::parse_releases(json, &["photocraft"]).unwrap().remove(0);
        rel.assets.retain(|a| a.name.target.map(|t| t.os) != Some(Os::Windows));
        let problems = check_release(&rel);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().all(|p| p.contains("windows")));
    }
}
