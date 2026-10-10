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
//!
//! `--app artcraft-toolbox` checks the toolbox's own releases the same way (it updates itself
//! through them, docs/architecture.md § 7), and `--dir <folder>` checks a folder of freshly built
//! release artifacts offline, as the release workflow does before it uploads them.

use artcraft_toolbox_catalog::{App, Catalog};
use artcraft_toolbox_feed::Release;
use artcraft_toolbox_model::Channel;
use artcraft_toolbox_release::{Arch, Checksums, Os, PackageKind, Target};
use std::path::Path;
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
    let mut dir = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--app" => only.push(it.next().ok_or("--app needs an id")?),
            "--dir" => dir = Some(it.next().ok_or("--dir needs a folder of release artifacts")?),
            other => return Err(format!("unexpected argument `{other}` (usage: cargo xtask contract [--app <id>]... | --dir <artifacts>)")),
        }
    }
    let catalog = Catalog::builtin().map_err(|e| e.to_string())?;
    if let Some(dir) = dir {
        return check_dir(&catalog, Path::new(dir));
    }
    for id in &only {
        if catalog.feed_app(id).is_none() {
            return Err(format!("`{id}` is not in crates/catalog/catalog.toml"));
        }
    }
    let apps: Vec<&App> = catalog
        .apps
        .iter()
        .chain(&catalog.toolbox)
        .filter(|a| if only.is_empty() { catalog.get(&a.id).is_some() } else { only.contains(&a.id.as_str()) })
        .collect();
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

/// A folder of the toolbox's own freshly built release artifacts (`dist/release`, or the release
/// workflow's merged artifacts): the same checks as a live release, without the network. Every
/// file must be a contract asset of one version, `SHA256SUMS.txt` must list each of them (and
/// nothing else), and every host must get its package.
fn check_dir(catalog: &Catalog, dir: &Path) -> Result<(), String> {
    let toolbox = catalog.toolbox.as_ref().ok_or("crates/catalog/catalog.toml has no [toolbox] entry")?;
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    let sums_text = std::fs::read_to_string(dir.join("SHA256SUMS.txt")).map_err(|e| format!("{}: SHA256SUMS.txt: {e}", dir.display()))?;
    let sums = Checksums::parse(&sums_text).map_err(|e| e.to_string())?;
    let (rel, mut problems) = release_of_files(toolbox, &names);
    for name in names.iter().filter(|n| *n != "SHA256SUMS.txt") {
        if sums.get(name).is_none() {
            problems.push(format!("{name} is not in SHA256SUMS.txt"));
        }
    }
    for (listed, _) in sums.iter() {
        if !names.iter().any(|n| n == listed) {
            problems.push(format!("SHA256SUMS.txt lists {listed}, which isn't here"));
        }
    }
    if let Some(rel) = &rel {
        problems.extend(check_release(rel));
    }
    match (rel, problems.is_empty()) {
        (Some(rel), true) => {
            println!(
                "OK: {} release artifacts in {} follow the release contract ({} assets, {} hosts).",
                rel.tag,
                dir.display(),
                rel.assets.len(),
                HOSTS.len()
            );
            Ok(())
        }
        (_, _) => {
            for p in &problems {
                println!("FAIL  {p}");
            }
            Err(format!("{} problem(s) in {} (docs/release-contract.md)", problems.len(), dir.display()))
        }
    }
}

/// The release a folder of files amounts to: every file as an asset at its would-be download
/// URL, tagged with the one version they share. Files that aren't assets are problems.
fn release_of_files(app: &App, names: &[String]) -> (Option<Release>, Vec<String>) {
    let slugs = app.slugs();
    let mut versions: Vec<String> = Vec::new();
    let mut problems = Vec::new();
    for n in names.iter().filter(|n| *n != "SHA256SUMS.txt") {
        if let Some(a) = artcraft_toolbox_release::asset::parse(n, &slugs) {
            let v = a.version.to_string();
            if !versions.contains(&v) {
                versions.push(v);
            }
        }
        // A file that isn't an asset is reported by `check_release` (the parser lists it as
        // unrecognized).
    }
    let version = match versions.as_slice() {
        [v] => v.clone(),
        [] => {
            problems.push("no release asset found".into());
            return (None, problems);
        }
        many => {
            problems.push(format!("assets of several versions: {}", many.join(", ")));
            return (None, problems);
        }
    };
    let assets: Vec<serde_json::Value> = names
        .iter()
        .map(|n| {
            let size = 1;
            serde_json::json!({"name": n, "size": size, "browser_download_url": format!("https://github.com/{}/releases/download/v{version}/{n}", app.repo)})
        })
        .collect();
    let json = serde_json::json!([{"tag_name": format!("v{version}"), "assets": assets}]).to_string();
    match artcraft_toolbox_feed::parse_releases(&json, &slugs) {
        Ok(mut rels) if !rels.is_empty() => (Some(rels.remove(0)), problems),
        Ok(_) => {
            problems.push("the files don't make a release".into());
            (None, problems)
        }
        Err(e) => {
            problems.push(e.to_string());
            (None, problems)
        }
    }
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

    /// What the release workflow builds for the toolbox, as file names (docs/releasing.md).
    const TOOLBOX_FILES: &[&str] = &[
        "artcraft-toolbox-0.1.0-linux-aarch64.AppImage",
        "artcraft-toolbox-0.1.0-linux-aarch64.AppImage.zsync",
        "artcraft-toolbox-0.1.0-linux-aarch64.deb",
        "artcraft-toolbox-0.1.0-linux-aarch64.rpm",
        "artcraft-toolbox-0.1.0-linux-aarch64.tar.gz",
        "artcraft-toolbox-0.1.0-linux-x86_64.AppImage",
        "artcraft-toolbox-0.1.0-linux-x86_64.AppImage.zsync",
        "artcraft-toolbox-0.1.0-linux-x86_64.deb",
        "artcraft-toolbox-0.1.0-linux-x86_64.rpm",
        "artcraft-toolbox-0.1.0-linux-x86_64.tar.gz",
        "artcraft-toolbox-0.1.0-macos-universal.dmg",
        "artcraft-toolbox-0.1.0-windows-arm64-portable.zip",
        "artcraft-toolbox-0.1.0-windows-arm64.msi",
        "artcraft-toolbox-0.1.0-windows-x64-portable.zip",
        "artcraft-toolbox-0.1.0-windows-x64.msi",
        "artcraft-toolbox-0.1.0-windows-x86-portable.zip",
        "artcraft-toolbox-0.1.0-windows-x86.msi",
        "artcraft-toolbox-cli-0.1.0-macos-universal.zip",
    ];

    #[test]
    fn the_toolboxs_own_artifacts_follow_the_contract() {
        let catalog = Catalog::builtin().unwrap();
        let dir = std::env::temp_dir().join(format!("artcraft-toolbox-contract-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut sums = String::new();
        for name in TOOLBOX_FILES {
            std::fs::write(dir.join(name), name.as_bytes()).unwrap();
            sums.push_str(&format!("{}  {name}\n", "0".repeat(64)));
        }
        std::fs::write(dir.join("SHA256SUMS.txt"), &sums).unwrap();
        check_dir(&catalog, &dir).unwrap();
        // A file outside the contract, one missing from the checksums, and a missing host.
        std::fs::write(dir.join("notes.pdf"), b"x").unwrap();
        let e = check_dir(&catalog, &dir).unwrap_err();
        assert!(e.contains("2 problem"), "{e}");
        std::fs::remove_file(dir.join("notes.pdf")).unwrap();
        std::fs::remove_file(dir.join("artcraft-toolbox-0.1.0-macos-universal.dmg")).unwrap();
        let e = check_dir(&catalog, &dir).unwrap_err();
        assert!(e.contains("3 problem"), "the listed-but-missing dmg and two macOS hosts: {e}");
        let _ = std::fs::remove_dir_all(&dir);
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
