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
//! the network). `--checking` draws a check in progress.

use std::sync::Arc;

use artcraft_toolbox_engine::net::{NetError, Request, Response, Transport};
use artcraft_toolbox_engine::{Catalog, Session};
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
    let transport: Option<Arc<dyn Transport>> = if checking { Some(Arc::new(Stall)) } else { None };
    // A copy of the data folder's files, so drawing never writes to it.
    let store = match arg(&args, "--data-dir") {
        Some(dir) => {
            let copy = std::env::temp_dir().join(format!("artcraft-toolbox-snapshot-{}", std::process::id()));
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
    for (app, file) in all(&args, "--feed") {
        let json = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
        session.ingest_releases(&app, &json, 0).map_err(|e| e.to_string())?;
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

    let mut app = ToolboxApp::new(session);
    if checking {
        app.start_check();
    }
    app.ui.tab = if arg(&args, "--tab").as_deref() == Some("settings") { Tab::Settings } else { Tab::Apps };
    app.ui.search = arg(&args, "--search").unwrap_or_default();

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
