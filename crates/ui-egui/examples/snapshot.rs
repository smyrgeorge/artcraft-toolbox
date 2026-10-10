//! Render the UI offscreen (no window, no focus stealing) and save a PNG. Look at it before you
//! ship a UI change (AGENTS.md rule 6).
//!
//! ```sh
//! cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/apps.png \
//!     --size 440x720 --scale 2 --tab apps --search photo \
//!     --feed photocraft=crates/feed/tests/fixtures/photocraft-releases.json --installed photocraft=0.3.0
//! ```
//!
//! `--feed <app>=<file>` loads a saved GitHub "list releases" response for an app, and
//! `--installed <app>=<version>` pretends a version is installed, so every status can be drawn
//! without the network. `--host <os>-<arch>` (`linux-x86_64`) picks releases for another machine.
//! `--data-dir <dir>` draws a real data folder's state (settings, cached feeds; read only, never
//! the network). `--checking` draws a check in progress. `--details <app>` opens an app's page,
//! and `--confirm-uninstall` its uninstall question. `--installing <app>` draws an install
//! stopped at 45% of its download (the app's release from `--feed`; nothing is installed).
//! `--installed` may name several versions of an app (the last is in use, the others kept);
//! `--signed` draws them signed and notarized. `--found <app>=<version>` draws a copy installed
//! outside the toolbox, offered for adoption (as a Linux AppImage: use `--host linux-x86_64`).
//! `--online` draws the buttons that need the network enabled (requests fail at once; no
//! automatic check runs). `--lang <code>` (`de`, `ja`, `auto` …), `--theme light|dark` and
//! `--text-size <percent>` set the appearance settings; without `--lang` it draws in English.
//! `--popover` draws the popover's rounded edge (the desktop app's window under the menu-bar or
//! tray icon); `--fold` folds the "Available apps" panel. `--toolbox-update <version>` draws the
//! toolbox's own update offer (a newer ArtCraft Toolbox published, this copy installed as a
//! package), and `--toolbox-ready <version>` that update downloaded, waiting for a restart.

use std::sync::Arc;

use std::collections::HashMap;
use std::io::Read;

use artcraft_toolbox_engine::install_cmds::INSTALL;
use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Layout, SelfInstall, Session};
use artcraft_toolbox_model::{Installation, Inventory, StagedUpdate, Trust};
use artcraft_toolbox_release::{Os, PackageKind, Target, Version};
use artcraft_toolbox_ui_egui::ToolboxApp;
use artcraft_toolbox_ui_egui::state::Tab;

/// A network that never answers (for `--checking`): requests block until the process exits.
struct Stall;

impl Transport for Stall {
    fn get(&self, _: &Request<'_>) -> Result<Response, NetError> {
        std::thread::sleep(std::time::Duration::from_secs(3600));
        Err(NetError::Timeout)
    }
}

/// A GitHub whose downloads stop at 45% (for `--installing`): it knows the `--feed` assets, lists
/// them all in `SHA256SUMS.txt` (the hashes never get checked) and answers nothing else.
struct Halfway {
    sizes: HashMap<String, (String, u64)>,
}

impl Transport for Halfway {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        if !req.url.ends_with("/SHA256SUMS.txt") {
            return Err(NetError::Connect("offline (snapshot)".into()));
        }
        let body: String = self.sizes.values().map(|(name, _)| format!("{}  {name}\n", "0".repeat(64))).collect();
        Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
    }
    fn download(&self, url: &str, _: u64) -> Result<Download, NetError> {
        let total = self.sizes.get(url).map_or(1 << 20, |(_, size)| *size);
        Ok(Download { reader: Box::new(Stalls { left: total * 45 / 100 }), offset: 0, total: Some(total) })
    }
}

/// Zeros, `left` of them, then nothing until the process exits.
struct Stalls {
    left: u64,
}

impl Read for Stalls {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
        let n = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        buf[..n].fill(0);
        self.left -= n as u64;
        Ok(n)
    }
}

/// `url -> (name, size)` of every asset in a GitHub "list releases" response.
fn asset_sizes(json: &str) -> HashMap<String, (String, u64)> {
    let releases: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let assets = releases.as_array().into_iter().flatten().flat_map(|r| r["assets"].as_array().into_iter().flatten());
    assets.filter_map(|a| Some((a["browser_download_url"].as_str()?.to_string(), (a["name"].as_str()?.to_string(), a["size"].as_u64()?)))).collect()
}

/// A GitHub "list releases" response publishing ArtCraft Toolbox `version` for every host.
fn toolbox_feed(version: &str) -> String {
    let files = [
        format!("artcraft-toolbox-{version}-macos-universal.dmg"),
        format!("artcraft-toolbox-{version}-windows-x64-portable.zip"),
        format!("artcraft-toolbox-{version}-windows-arm64-portable.zip"),
        format!("artcraft-toolbox-{version}-linux-x86_64.AppImage"),
        format!("artcraft-toolbox-{version}-linux-aarch64.AppImage"),
        "SHA256SUMS.txt".to_string(),
    ];
    let assets: Vec<serde_json::Value> = files
        .iter()
        .map(|f| serde_json::json!({"name": f, "size": 1 << 20, "browser_download_url": format!("https://github.com/smyrgeorge/artcraft-toolbox/releases/download/v{version}/{f}")}))
        .collect();
    serde_json::json!([{"tag_name": format!("v{version}"), "assets": assets}]).to_string()
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn all(args: &[String], name: &str) -> Vec<(String, String)> {
    args.windows(2).filter(|w| w[0] == name).filter_map(|w| w[1].split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))).collect()
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").unwrap_or_else(|| "target/snapshots/toolbox.png".into());
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
        .unwrap_or((440.0, 720.0));
    let scale: f32 = arg(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(2.0);

    let checking = args.iter().any(|a| a == "--checking");
    let installing = arg(&args, "--installing");
    let feeds = all(&args, "--feed")
        .into_iter()
        .map(|(app, file)| std::fs::read_to_string(&file).map(|json| (app, json)).map_err(|e| format!("{file}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let online = args.iter().any(|a| a == "--online");
    let transport: Option<Arc<dyn Transport>> = if checking {
        Some(Arc::new(Stall))
    } else if installing.is_some() || online {
        // Answers only checksums for `--installing`; everything else fails at once.
        Some(Arc::new(Halfway { sizes: feeds.iter().flat_map(|(_, json)| asset_sizes(json)).collect() }))
    } else {
        None
    };
    // A copy of the data folder's files, so drawing never writes to it (removed at the end).
    let copy = std::env::temp_dir().join(format!("artcraft-toolbox-snapshot-{}", std::process::id()));
    let toolbox_flags = arg(&args, "--toolbox-update").is_some() || arg(&args, "--toolbox-ready").is_some();
    let store = match arg(&args, "--data-dir") {
        Some(dir) => {
            copy_dir(std::path::Path::new(&dir), &copy).map_err(|e| format!("{dir}: {e}"))?;
            Some(artcraft_toolbox_store::Store::open(&copy).map_err(|e| e.to_string())?)
        }
        // The toolbox's own update needs a data folder to download into: an empty scratch one.
        None if toolbox_flags => Some(artcraft_toolbox_store::Store::open(copy.join("data")).map_err(|e| e.to_string())?),
        None => None,
    };
    let (mut session, warnings) = Session::open(Catalog::builtin().map_err(|e| e.to_string())?, store, transport);
    for w in warnings {
        eprintln!("warning: {w}");
    }
    if let Some(h) = arg(&args, "--host") {
        let (os, arch) = h.split_once('-').ok_or("--host is <os>-<arch>, e.g. linux-x86_64")?;
        session.set_host(Some(Target::from_consts(os, arch).ok_or_else(|| format!("unknown host {h}"))?));
    }
    for (app, json) in &feeds {
        session.ingest_releases(app, json, artcraft_toolbox_engine::time::now_unix()).map_err(|e| e.to_string())?;
    }
    let mut inventory = Inventory::default();
    let signed = args.iter().any(|a| a == "--signed");
    for (n, (app, v)) in all(&args, "--installed").into_iter().enumerate() {
        let version = Version::parse(&v).map_err(|e| e.to_string())?;
        let trust = if signed {
            Trust::Signed { signer: "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)".into(), team: Some("DJ6XS33FX8".into()), notarized: true }
        } else {
            Trust::Unsigned
        };
        inventory
            .record(Installation { app, version, kind: PackageKind::AppImage, path: String::new(), installed_at: n as u64, active: true, trust: Some(trust) })
            .map_err(|e| e.to_string())?;
    }
    if !inventory.installations.is_empty() {
        session.set_inventory(inventory);
    }

    // Draw what the desktop app shows: notifications and a tray icon available.
    let popover = args.iter().any(|a| a == "--popover");
    let services = artcraft_toolbox_ui_egui::Services { notify: Some(Box::new(|_: &str, _: &str| {})), tray: true, popover, transparent: popover };
    let mut app = ToolboxApp::with_services(session, services);
    if checking {
        app.start_check();
    }
    // Installing is on, as in the desktop app, into a scratch folder removed at the end (only
    // `--installing` writes to it).
    let scratch = std::env::temp_dir().join(format!("artcraft-toolbox-snapshot-install-{}", std::process::id()));
    for (id, v) in all(&args, "--found") {
        let file = scratch.join("apps").join(&id).join(&v).join(format!("{id}.AppImage"));
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&file, b"\x7fELF\x02\x01\x01\x00AI\x02").map_err(|e| format!("{}: {e}", file.display()))?;
    }
    app.session.set_layout(Some(Layout {
        apps: scratch.join("apps"),
        desktop_entries: None,
        icons: None,
        start_menu: None,
        downloads: scratch.join("downloads"),
        kept: scratch.join("kept"),
    }));
    app.session.rescan();
    let language = arg(&args, "--lang").unwrap_or_else(|| "en".into());
    app.session.execute("settings.set", serde_json::json!({ "language": language })).map_err(|e| e.to_string())?;
    if let Some(theme) = arg(&args, "--theme") {
        app.session.execute("settings.set", serde_json::json!({ "theme": theme })).map_err(|e| e.to_string())?;
    }
    if let Some(size) = arg(&args, "--text-size").and_then(|s| s.parse::<u32>().ok()) {
        app.session.execute("settings.set", serde_json::json!({ "textSize": size })).map_err(|e| e.to_string())?;
    }
    if online || installing.is_some() {
        app.session.execute("settings.set", serde_json::json!({ "checkIntervalHours": 0 })).map_err(|e| e.to_string())?;
    }
    if let Some(id) = &installing {
        app.session.start(INSTALL, serde_json::json!({ "app": id })).map_err(|e| e.to_string())?;
        // Let the download reach its stopping point.
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    // The toolbox's own update: this copy as a packaged install of this build's version, and a
    // newer version published (`--toolbox-update`), downloaded already (`--toolbox-ready`).
    let ready = arg(&args, "--toolbox-ready");
    if let Some(v) = arg(&args, "--toolbox-update").or_else(|| ready.clone()) {
        let host = app.session.host().ok_or("no host")?;
        let kind = match host.os {
            Os::Macos => PackageKind::Dmg,
            Os::Windows => PackageKind::PortableZip,
            _ => PackageKind::AppImage,
        };
        let running = artcraft_toolbox_engine::toolbox_cmds::running_version();
        app.session.set_self_install(Some(SelfInstall { kind, path: scratch.join("ArtCraft Toolbox.app"), version: running }));
        app.session.ingest_releases("artcraft-toolbox", &toolbox_feed(&v), artcraft_toolbox_engine::time::now_unix()).map_err(|e| e.to_string())?;
        if let Some(v) = &ready {
            let version = Version::parse(v).map_err(|e| e.to_string())?;
            let path = scratch.join("self-update").join("x").to_string_lossy().into_owned();
            app.session.set_staged_update(Some(StagedUpdate { version, kind, path, staged_at: 0, trust: Some(Trust::Unsigned) }));
        }
    }
    app.ui.tab = if arg(&args, "--tab").as_deref() == Some("settings") { Tab::Settings } else { Tab::Apps };
    app.ui.search = arg(&args, "--search").unwrap_or_default();
    app.ui.search_open = !app.ui.search.is_empty();
    app.ui.available_folded = args.iter().any(|a| a == "--fold");
    app.ui.selected = arg(&args, "--details");
    if args.iter().any(|a| a == "--confirm-uninstall") {
        app.ui.confirm_uninstall = app.ui.selected.clone();
    }

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(w, h)).with_pixels_per_point(scale).wgpu().build_eframe(move |cc| {
        ToolboxApp::setup_context(&cc.egui_ctx);
        app
    });
    // A few frames more than layout needs: CJK fallback fonts load one per frame.
    harness.run_steps(8);
    let img = harness.render().map_err(|e| format!("render: {e}"))?;
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    img.save(&out).map_err(|e| format!("{out}: {e}"))?;
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = std::fs::remove_dir_all(&copy);
    println!("wrote {out} ({}×{})", img.width(), img.height());
    Ok(())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
