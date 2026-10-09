//! The CLI's contract: output, exit codes, no panic on bad input, and no side effects from usage
//! errors. Every test runs in its own temp data folder with a fake network.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use artcraft_toolbox_cli::{Env, run};
use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Layout, Target};
use sha2::Digest;

const PHOTOCRAFT: &str = include_str!("../../../crates/feed/tests/fixtures/photocraft-releases.json");
const FEED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/feed/tests/fixtures/photocraft-releases.json");

fn temp() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-cli-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// PhotoCraft's real feed; an empty feed for the rest; a timeout for VectorCraft when `flaky`.
struct Fake {
    flaky: bool,
    requests: Mutex<usize>,
}

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        *self.requests.lock().unwrap() += 1;
        if self.flaky && req.url.contains("/vectorcraft/") {
            return Err(NetError::Timeout);
        }
        let body = if req.url.contains("/photocraft/") { PHOTOCRAFT } else { "[]" };
        Ok(Response::Ok { body: body.as_bytes().to_vec(), etag: Some("W/\"e\"".into()), rate: RateLimit::default() })
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn cli_in(dir: &Path, fake: &Arc<Fake>, args: &[&str]) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (dir, fake) = (dir.to_path_buf(), Arc::clone(fake));
    let layout =
        Layout { apps: dir.join("apps"), desktop_entries: None, icons: None, start_menu: None, downloads: dir.join("downloads"), kept: dir.join("kept") };
    let code = run(args, &mut out, &mut err, move || Env { data_dir: Ok(dir), transport: Some(fake), layout: Some(layout), host: None });
    Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
}

fn fake(flaky: bool) -> Arc<Fake> {
    Arc::new(Fake { flaky, requests: Mutex::new(0) })
}

fn cli(args: &[&str]) -> Run {
    let dir = temp();
    let r = cli_in(&dir, &fake(false), args);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn help_and_version() {
    let r = cli(&[]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("usage: artcraft-toolbox-cli") && r.out.contains("ARTCRAFT_TOOLBOX_GITHUB_TOKEN"));
    let r = cli(&["--version"]);
    assert_eq!(r.code, 0);
    assert!(r.out.starts_with("artcraft-toolbox-cli 0."));
    assert_eq!(cli(&["check", "--help"]).code, 0);
}

#[test]
fn list_and_commands() {
    let r = cli(&["list"]);
    assert_eq!(r.code, 0);
    assert!(r.out.lines().any(|l| l.starts_with("photocraft") && l.contains("PhotoCraft")));
    let r = cli(&["list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(12));
    let r = cli(&["commands", "--json"]);
    assert_eq!(r.code, 0);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    let check = v.as_array().unwrap().iter().find(|c| c["id"] == "updates.check").unwrap();
    assert_eq!(check["background"], true);
}

#[test]
fn check_then_status_reads_the_cache() {
    let dir = temp();
    let net = fake(false);
    let r = cli_in(&dir, &net, &["check"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("Checked just now."), "{}", r.out);
    assert_eq!(*net.requests.lock().unwrap(), 12);
    assert!(dir.join("feeds/photocraft.json").exists());

    // A later run (a new process, in effect) reads the cached feeds without the network.
    let offline = fake(false);
    let r = cli_in(&dir, &offline, &["status", "--json"]);
    assert_eq!(r.code, 0);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    let pc = v.as_array().unwrap().iter().find(|r| r["id"] == "photocraft").unwrap();
    assert_eq!(pc["status"]["latest"], "0.5.0");
    assert!(pc["checkedAt"].is_u64());
    assert_eq!(*offline.requests.lock().unwrap(), 0, "status never touches the network");

    // Checked seconds ago: nothing is requested again unless forced.
    let again = fake(false);
    assert_eq!(cli_in(&dir, &again, &["check", "--app", "photocraft"]).code, 0);
    assert_eq!(*again.requests.lock().unwrap(), 0);
    assert_eq!(cli_in(&dir, &again, &["check", "--app", "photocraft", "--force"]).code, 0);
    assert_eq!(*again.requests.lock().unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_check_exits_1_and_says_why() {
    let dir = temp();
    let r = cli_in(&dir, &fake(true), &["check", "--json"]);
    assert_eq!(r.code, 1, "{}", r.err);
    assert!(r.err.contains("Couldn't check VectorCraft: the server took too long to answer"), "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v["check"]["failed"][0]["app"], "vectorcraft");
    assert_eq!(v["apps"].as_array().map(Vec::len), Some(12));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn status_with_an_offline_feed() {
    let feed = format!("photocraft={FEED}");
    let r = cli(&["status", "--json", "--feed", &feed]);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    let pc = v.as_array().unwrap().iter().find(|r| r["id"] == "photocraft").unwrap();
    // Every CI and dev platform gets a PhotoCraft build.
    assert_eq!(pc["status"], serde_json::json!({"state": "notInstalled", "latest": "0.5.0"}));
    let wc = v.as_array().unwrap().iter().find(|r| r["id"] == "wordcraft").unwrap();
    assert_eq!(wc["status"]["state"], "unknown");
}

#[test]
fn run_dispatches_engine_commands_and_settings_persist() {
    let dir = temp();
    let net = fake(false);
    let r = cli_in(&dir, &net, &["run", "app.status", r#"{"app":"vectorcraft"}"#]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("\"VectorCraft\""));
    let r = cli_in(&dir, &net, &["run", "app.status", r#"{"app":"nope"}"#]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("unknown app"));
    assert_eq!(cli_in(&dir, &net, &["run", "settings.set", r#"{"channel":"prerelease"}"#]).code, 0);
    let r = cli_in(&dir, &net, &["run", "settings.get"]);
    assert!(r.out.contains("\"prerelease\""), "{}", r.out);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn usage_errors_exit_2_and_touch_nothing() {
    for args in [
        vec!["nope"],
        vec!["list", "--nope"],
        vec!["list", "--json", "--json"],
        vec!["status", "--feed"],
        vec!["status", "--feed", "photocraft"],
        vec!["check", "--app"],
        vec!["check", "--app", "a", "--app", "b"],
        vec!["check", "--feed", "x=y"],
        vec!["list", "--force"],
        vec!["run"],
        vec!["run", "app.status", "{not json"],
        vec!["run", "a", "{}", "extra"],
        vec!["install"],
        vec!["install", "--json"],
        vec!["install", "photocraft", "--version"],
        vec!["install", "photocraft", "--force"],
        vec!["uninstall", "photocraft", "--version", "1.0.0"],
        vec!["open"],
        vec!["open", "photocraft", "--json"],
        vec!["update"],
        vec!["update", "--json"],
        vec!["update", "--all", "photocraft"],
        vec!["update", "photocraft", "--version"],
        vec!["rollback"],
        vec!["rollback", "photocraft", "--version"],
        vec!["rollback", "photocraft", "--force"],
        vec!["versions"],
        vec!["versions", "photocraft", "--version", "1.0.0"],
        vec!["adopt"],
        vec!["adopt", "photocraft", "--all"],
    ] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&args, &mut out, &mut err, || panic!("a usage error must not open the session ({args:?})"));
        let err = String::from_utf8(err).unwrap();
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains("usage:"), "{args:?}");
    }
    // A missing feed file is a failure, not a usage error.
    assert_eq!(cli(&["status", "--feed", "photocraft=/definitely/missing.json"]).code, 1);
}

#[test]
fn no_data_folder_is_a_warning() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&["list"], &mut out, &mut err, || Env { data_dir: Err("no HOME".into()), transport: None, layout: None, host: None });
    assert_eq!(code, 0);
    assert!(String::from_utf8(err).unwrap().contains("warning: no HOME; nothing will be saved"));
}

#[test]
fn binary_exit_codes() {
    let exe = env!("CARGO_BIN_EXE_artcraft-toolbox-cli");
    let ok = Command::new(exe).arg("--version").output().unwrap();
    assert!(ok.status.success());
    let bad = Command::new(exe).arg("--bogus").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
}

/// A GitHub with one installable PhotoCraft release for Linux.
struct Installable;

const ASSET_URL: &str = "https://github.com/storytold/photocraft/releases/download/v9.0.0/photocraft-9.0.0-linux-x86_64.AppImage";

fn appimage() -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend(std::iter::repeat_n(7u8, 50_000));
    b
}

impl Transport for Installable {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let body = if req.url.ends_with("SHA256SUMS.txt") {
            let hex: String = sha2::Sha256::digest(appimage()).iter().map(|b| format!("{b:02x}")).collect();
            format!("{hex}  photocraft-9.0.0-linux-x86_64.AppImage\n")
        } else if req.url.contains("/photocraft/") {
            serde_json::json!([{"tag_name": "v9.0.0", "assets": [
                {"name": "photocraft-9.0.0-linux-x86_64.AppImage", "size": appimage().len(), "browser_download_url": ASSET_URL},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": "https://github.com/storytold/photocraft/releases/download/v9.0.0/SHA256SUMS.txt"},
            ]}])
            .to_string()
        } else {
            "[]".into()
        };
        Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, url: &str, from: u64) -> Result<Download, NetError> {
        assert_eq!(url, ASSET_URL);
        let rest = appimage().get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(appimage().len() as u64) })
    }
}

fn cli_installable(dir: &Path, args: &[&str]) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let dir = dir.to_path_buf();
    let layout =
        Layout { apps: dir.join("apps"), desktop_entries: None, icons: None, start_menu: None, downloads: dir.join("downloads"), kept: dir.join("kept") };
    let code = run(args, &mut out, &mut err, move || Env {
        data_dir: Ok(dir),
        transport: Some(Arc::new(Installable)),
        layout: Some(layout),
        host: Target::from_consts("linux", "x86_64"),
    });
    Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
}

#[test]
fn install_open_and_uninstall() {
    let dir = temp();
    // Nothing known yet: install says to check first.
    let r = cli_installable(&dir, &["install", "photocraft"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("check for updates first"), "{}", r.err);
    // The session pretends to be a Linux computer, so this installs the AppImage on any OS.
    assert_eq!(cli_installable(&dir, &["check", "--app", "photocraft"]).code, 0);
    let r = cli_installable(&dir, &["install", "photocraft"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.err.contains("Starting"), "progress goes to stderr: {}", r.err);
    assert!(r.out.starts_with("photocraft 9.0.0 installed: "), "{}", r.out);
    assert!(dir.join("apps/photocraft/9.0.0/photocraft.AppImage").is_file());
    let r = cli_installable(&dir, &["uninstall", "photocraft", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("\"removed\""));
    assert!(!dir.join("apps/photocraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_or_removing_what_isnt_installed_fails_cleanly() {
    for args in [vec!["open", "photocraft"], vec!["uninstall", "photocraft"]] {
        let r = cli(&args);
        assert_eq!(r.code, 1, "{args:?}");
        assert!(r.err.contains("isn't installed"), "{args:?}: {}", r.err);
    }
    assert!(cli(&["open", "nope"]).err.contains("unknown app"));
}

/// A GitHub with PhotoCraft 8.0.0 and 9.0.0 for Linux.
struct TwoReleases;

fn appimage_of(v: &str) -> Vec<u8> {
    let mut b = appimage();
    b.extend(v.as_bytes());
    b
}

fn release_url(v: &str, file: &str) -> String {
    format!("https://github.com/storytold/photocraft/releases/download/v{v}/{file}")
}

impl Transport for TwoReleases {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let v = if req.url.contains("/v8.0.0/") { "8.0.0" } else { "9.0.0" };
        let body = if req.url.ends_with("SHA256SUMS.txt") {
            let hex: String = sha2::Sha256::digest(appimage_of(v)).iter().map(|b| format!("{b:02x}")).collect();
            format!("{hex}  photocraft-{v}-linux-x86_64.AppImage\n")
        } else if req.url.contains("/photocraft/") {
            let rel = |v: &str| {
                let file = format!("photocraft-{v}-linux-x86_64.AppImage");
                serde_json::json!({"tag_name": format!("v{v}"), "assets": [
                    {"name": file, "size": appimage_of(v).len(), "browser_download_url": release_url(v, &file)},
                    {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": release_url(v, "SHA256SUMS.txt")},
                ]})
            };
            serde_json::json!([rel("9.0.0"), rel("8.0.0")]).to_string()
        } else {
            "[]".into()
        };
        Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, url: &str, from: u64) -> Result<Download, NetError> {
        let v = if url.contains("/v8.0.0/") { "8.0.0" } else { "9.0.0" };
        let body = appimage_of(v);
        let rest = body.get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(body.len() as u64) })
    }
}

fn cli_two(dir: &Path, args: &[&str]) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let dir = dir.to_path_buf();
    let layout =
        Layout { apps: dir.join("apps"), desktop_entries: None, icons: None, start_menu: None, downloads: dir.join("downloads"), kept: dir.join("kept") };
    let code = run(args, &mut out, &mut err, move || Env {
        data_dir: Ok(dir),
        transport: Some(Arc::new(TwoReleases)),
        layout: Some(layout),
        host: Target::from_consts("linux", "x86_64"),
    });
    Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
}

#[test]
fn update_versions_rollback_and_update_all() {
    let dir = temp();
    assert_eq!(cli_two(&dir, &["check", "--app", "photocraft"]).code, 0);
    let r = cli_two(&dir, &["update", "--all"]);
    assert_eq!((r.code, r.out.as_str()), (0, "nothing to update\n"), "{}", r.err);
    let r = cli_two(&dir, &["update", "photocraft"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("isn't installed"), "{}", r.err);

    assert_eq!(cli_two(&dir, &["install", "photocraft", "--version", "8.0.0"]).code, 0);
    let r = cli_two(&dir, &["update", "photocraft"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.starts_with("photocraft 9.0.0 updated: "), "{}", r.out);
    let r = cli_two(&dir, &["versions", "photocraft"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let lines: Vec<&str> = r.out.lines().collect();
    assert!(lines.len() == 2 && lines[0].starts_with("9.0.0      in use  No platform signature") && lines[1].starts_with("8.0.0      kept"), "{}", r.out);

    let r = cli_two(&dir, &["rollback", "photocraft"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.starts_with("photocraft 8.0.0 in use: "), "{}", r.out);
    // The newer version is kept: update --all switches back to it.
    let r = cli_two(&dir, &["update", "--all"]);
    assert_eq!((r.code, r.out.as_str()), (0, "photocraft 9.0.0 in use (it was kept)\n"), "{}", r.err);

    let r = cli_two(&dir, &["uninstall", "photocraft", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("\"removedVersions\": [\n    \"8.0.0\"\n  ]"), "{}", r.out);
    assert!(!dir.join("apps/photocraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_copy_installed_by_hand_is_adopted() {
    let dir = temp();
    let by_hand = dir.join("apps/photocraft/8.0.0/photocraft.AppImage");
    std::fs::create_dir_all(by_hand.parent().unwrap()).unwrap();
    std::fs::write(&by_hand, appimage_of("8.0.0")).unwrap();
    let r = cli_two(&dir, &["versions", "photocraft"]);
    assert!(r.out.starts_with("8.0.0      found outside the toolbox"), "{}", r.out);
    let r = cli_two(&dir, &["adopt", "photocraft"]);
    assert_eq!((r.code, r.out.as_str()), (0, "photocraft adopted: 8.0.0\n"), "{}", r.err);
    let r = cli_two(&dir, &["adopt", "photocraft"]);
    assert!(r.code == 1 && r.err.contains("already managed"), "{}", r.err);
    let _ = std::fs::remove_dir_all(&dir);
}
