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

use std::sync::Arc;

use std::collections::HashMap;
use std::io::Read;

use artcraft_toolbox_engine::install_cmds::INSTALL;
use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Layout, Session};
use artcraft_toolbox_model::{Installation, Inventory};
use artcraft_toolbox_release::{PackageKind, Target, Version};
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
    let transport: Option<Arc<dyn Transport>> = if checking {
        Some(Arc::new(Stall))
    } else if installing.is_some() {
        Some(Arc::new(Halfway { sizes: feeds.iter().flat_map(|(_, json)| asset_sizes(json)).collect() }))
    } else {
        None
    };
    // A copy of the data folder's files, so drawing never writes to it (removed at the end).
    let copy = std::env::temp_dir().join(format!("artcraft-toolbox-snapshot-{}", std::process::id()));
    let store = match arg(&args, "--data-dir") {
        Some(dir) => {
            copy_dir(std::path::Path::new(&dir), &copy).map_err(|e| format!("{dir}: {e}"))?;
            Some(artcraft_toolbox_store::Store::open(&copy).map_err(|e| e.to_string())?)
        }
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
    for (app, v) in all(&args, "--installed") {
        let version = Version::parse(&v).map_err(|e| e.to_string())?;
        inventory
            .record(Installation { app, version, kind: PackageKind::AppImage, path: String::new(), installed_at: 0, active: true })
            .map_err(|e| e.to_string())?;
    }
    if !inventory.installations.is_empty() {
        session.set_inventory(inventory);
    }

    // Draw what the desktop app shows: notifications and a tray icon available.
    let services = artcraft_toolbox_ui_egui::Services { notify: Some(Box::new(|_: &str, _: &str| {})), tray: true };
    let mut app = ToolboxApp::with_services(session, services);
    if checking {
        app.start_check();
    }
    // Installing is on, as in the desktop app, into a scratch folder removed at the end (only
    // `--installing` writes to it).
    let scratch = std::env::temp_dir().join(format!("artcraft-toolbox-snapshot-install-{}", std::process::id()));
    app.session.set_layout(Some(Layout {
        apps: scratch.join("apps"),
        desktop_entries: None,
        icons: None,
        start_menu: None,
        downloads: scratch.join("downloads"),
    }));
    if let Some(id) = &installing {
        app.session.execute("settings.set", serde_json::json!({ "checkIntervalHours": 0 })).map_err(|e| e.to_string())?;
        app.session.start(INSTALL, serde_json::json!({ "app": id })).map_err(|e| e.to_string())?;
        // Let the download reach its stopping point.
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    app.ui.tab = if arg(&args, "--tab").as_deref() == Some("settings") { Tab::Settings } else { Tab::Apps };
    app.ui.search = arg(&args, "--search").unwrap_or_default();
    app.ui.selected = arg(&args, "--details");
    if args.iter().any(|a| a == "--confirm-uninstall") {
        app.ui.confirm_uninstall = app.ui.selected.clone();
    }

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(w, h)).with_pixels_per_point(scale).wgpu().build_eframe(move |cc| {
        ToolboxApp::setup_context(&cc.egui_ctx);
        app
    });
    harness.run_steps(4);
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
