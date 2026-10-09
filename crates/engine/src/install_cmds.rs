//! `app.install` (a background job), `app.uninstall` and `app.launch`.
//!
//! An install is download → verify → install → record, on one worker thread:
//!
//! 1. the release's `SHA256SUMS.txt` is fetched first: no checksum entry, no install;
//! 2. the asset is streamed to `<data>/downloads/<file>.part`, resuming a partial file with an
//!    HTTP range, and must end at exactly the size GitHub listed;
//! 3. its SHA-256 must match, or it is deleted and the install fails;
//! 4. the platform installer (`artcraft_toolbox_install`) puts it in place under a staging name;
//! 5. the session records it in the inventory (and saves it).
//!
//! Cancelling keeps the `.part` file, so the next install resumes it. Nothing already installed
//! is replaced: updating is milestone M3. See docs/architecture.md § 5 for where apps go.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use artcraft_toolbox_install::{AppInfo, Layout};
use artcraft_toolbox_model::Installation;
use artcraft_toolbox_net::{NetError, Request, Transport};
use artcraft_toolbox_release::{Checksums, Os, PackageKind, Version, asset};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::jobs::{JobMsg, JobState, Running};
use crate::{EngineError, JobId, Result, Session, echo, params};

pub const INSTALL: &str = "app.install";
pub const UNINSTALL: &str = "app.uninstall";
pub const LAUNCH: &str = "app.launch";
/// Largest checksum file read (a real one is about 2 KB).
const MAX_SUMS_BYTES: usize = 1 << 20;
/// Progress is reported at most this often (bytes).
const PROGRESS_STEP: u64 = 512 * 1024;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: INSTALL, label: "Install", params: r#"{"app":"<id>","version"?:"x.y.z"}"#, enabled: can_install, run, start: Some(start) },
        CommandSpec { id: UNINSTALL, label: "Uninstall", params: r#"{"app":"<id>"}"#, enabled: can_change_installs, run: uninstall, start: None },
        CommandSpec { id: LAUNCH, label: "Open", params: r#"{"app":"<id>"}"#, enabled: always, run: launch, start: None },
    ]
}

fn can_change_installs(s: &Session) -> std::result::Result<(), String> {
    if s.layout.is_none() {
        return Err("this session has no install location".into());
    }
    if let Some(why) = &s.inventory_locked {
        return Err(why.clone());
    }
    Ok(())
}

fn can_install(s: &Session) -> std::result::Result<(), String> {
    if !s.online() {
        return Err("this session has no network access".into());
    }
    can_change_installs(s)
}

/// The package kind this computer installs (docs/architecture.md § 5).
fn installs_here(kind: PackageKind, os: Os) -> bool {
    matches!((kind, os), (PackageKind::Dmg, Os::Macos) | (PackageKind::PortableZip, Os::Windows) | (PackageKind::AppImage, Os::Linux))
}

/// Everything the worker needs, owned.
struct Plan {
    id: String,
    name: String,
    bundle_id: String,
    tagline: String,
    version: Version,
    kind: PackageKind,
    url: String,
    file: String,
    size: u64,
    sums_url: String,
    icon_png: Option<Vec<u8>>,
}

impl Plan {
    /// `PhotoCraft 0.5.0`.
    fn what(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// What the install job reports.
pub(crate) enum InstallMsg {
    Progress { phase: &'static str, done: u64, total: Option<u64> },
    Installed(Installation),
    Failed(String),
}

/// An install job's bookkeeping.
pub(crate) struct InstallState {
    pub(crate) phase: String,
    done: u64,
    total: Option<u64>,
    /// Set once the job reported its end.
    pub(crate) outcome: Option<std::result::Result<Value, String>>,
}

impl InstallState {
    pub(crate) fn fraction(&self) -> Option<f32> {
        self.total.filter(|t| *t > 0).map(|t| (self.done as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = start(s, p)?;
    s.wait_job(id)
}

fn refused(m: impl Into<String>) -> EngineError {
    EngineError::Refused(m.into())
}

fn start(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["app", "version"])?;
    let app = params::app(s, p)?.clone();
    let wanted = match params::object(p)?.get("version") {
        None => None,
        Some(Value::String(v)) => Some(Version::parse(v).map_err(|e| EngineError::BadParams(e.to_string()))?),
        Some(other) => return Err(EngineError::BadParams(format!("`version` must be a string, got {}", params::kind(other)))),
    };
    if s.job_for(INSTALL, &app.id).is_some() {
        return Err(refused(format!("{} is already being installed", app.name)));
    }
    if let Some(inst) = s.inventory().current(&app.id) {
        return Err(refused(format!("{} {} is already installed (updating comes with milestone M3)", app.name, inst.version)));
    }
    let layout = s.layout.clone().ok_or_else(|| refused("this session has no install location"))?;
    let host = s.host().ok_or_else(|| refused("no Crafting App is built for this computer"))?;
    let feed = s.feed(&app.id).ok_or_else(|| refused(format!("no release information for {} yet: check for updates first", app.name)))?;
    let (release, chosen) = match &wanted {
        Some(v) => {
            let r = feed.releases.iter().find(|r| &r.version == v).ok_or_else(|| refused(format!("{} has no release {v}", app.name)))?;
            let a = asset::select(&r.assets, host, |a| &a.name).ok_or_else(|| refused(format!("{} {v} has no build for this computer", app.name)))?;
            (r, a)
        }
        None => artcraft_toolbox_feed::latest(&feed.releases, s.settings().channel_for(&app.id), host, s.settings().pinned(&app.id))
            .ok_or_else(|| refused(format!("{} has no release for this computer", app.name)))?,
    };
    if !installs_here(chosen.name.kind, host.os) {
        return Err(refused(format!("installing {:?} packages isn't supported on {}", chosen.name.kind, host.os.label())));
    }
    let sums_url = release
        .checksums_url
        .clone()
        .ok_or_else(|| refused(format!("{} {} has no SHA256SUMS.txt; the toolbox only installs what it can verify", app.name, release.version)))?;
    let icon_png = s.store.as_ref().and_then(|st| st.load_icon(&app.id).ok().flatten()).map(|(png, _)| png);
    let plan = Plan {
        id: app.id.clone(),
        name: app.name.clone(),
        bundle_id: app.bundle_id(),
        tagline: app.tagline.clone(),
        version: release.version.clone(),
        kind: chosen.name.kind,
        url: chosen.url.clone(),
        file: chosen.file.clone(),
        size: chosen.size,
        sums_url,
        icon_png,
    };
    let label = format!("Installing {}", plan.what());
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let transport = s.transport.clone().ok_or_else(|| refused("this session has no network access"))?;
    let (worker_cancel, clock) = (Arc::clone(&cancel), s.clock);
    let total = Some(plan.size);
    let spawned = std::thread::Builder::new().name("app-install".into()).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install_worker(&plan, transport.as_ref(), &layout, clock, &worker_cancel, &tx)))
            .unwrap_or_else(|_| Err("internal error (logged)".into()));
        let msg = match outcome {
            Ok(inst) => InstallMsg::Installed(inst),
            Err(e) => InstallMsg::Failed(e),
        };
        let _ = tx.send(JobMsg::Install(msg));
    });
    if let Err(e) = spawned {
        return Err(EngineError::Job(format!("couldn't start the install: {e}")));
    }
    let id = s.jobs.next_id();
    let state = InstallState { phase: "Starting".into(), done: 0, total, outcome: None };
    s.jobs.push(Running { id, command: INSTALL, label, rx, cancel, items: vec![app.id], in_flight: Default::default(), state: JobState::Install(state) });
    Ok(id)
}

fn send(tx: &Sender<JobMsg>, phase: &'static str, done: u64, total: Option<u64>) {
    let _ = tx.send(JobMsg::Install(InstallMsg::Progress { phase, done, total }));
}

/// Download, verify, install (on the worker). Never touches the session.
fn install_worker(
    plan: &Plan,
    transport: &dyn Transport,
    layout: &Layout,
    clock: fn() -> u64,
    cancel: &AtomicBool,
    tx: &Sender<JobMsg>,
) -> std::result::Result<Installation, String> {
    let what = plan.what();
    send(tx, "Verifying the release", 0, Some(plan.size));
    let sums = transport
        .get(&Request { url: &plan.sums_url, accept: "text/plain", etag: None, max_bytes: MAX_SUMS_BYTES })
        .map_err(|e| format!("couldn't get {what}'s checksums: {e}"))
        .and_then(|r| match r {
            artcraft_toolbox_net::Response::Ok { body, .. } => String::from_utf8(body).map_err(|_| "SHA256SUMS.txt is not text".to_string()),
            artcraft_toolbox_net::Response::NotModified { .. } => Err("SHA256SUMS.txt: unexpected 304".to_string()),
        })?;
    let sums = Checksums::parse(&sums).map_err(|e| e.to_string())?;
    let expected = sums.get(&plan.file).ok_or_else(|| format!("SHA256SUMS.txt doesn't list {}; not installing it", plan.file))?;

    let package = download(plan, transport, &layout.downloads, cancel, tx)?;
    send(tx, "Verifying", plan.size, Some(plan.size));
    let actual = artcraft_toolbox_install::sha256_file(&package).map_err(|e| e.to_string())?;
    if actual != expected {
        let _ = std::fs::remove_file(&package);
        return Err(format!("{} doesn't match its checksum (expected {expected}, got {actual}); it was deleted", plan.file));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    send(tx, "Installing", plan.size, Some(plan.size));
    let version = plan.version.to_string();
    let info =
        AppInfo { id: &plan.id, name: &plan.name, bundle_id: &plan.bundle_id, tagline: &plan.tagline, version: &version, icon_png: plan.icon_png.as_deref() };
    let installed = artcraft_toolbox_install::install(layout, &info, plan.kind, &package);
    // The verified package has served its purpose either way.
    let _ = std::fs::remove_file(&package);
    let path = installed.map_err(|e| e.to_string())?;
    let path = path.to_str().ok_or("the install location is not a UTF-8 path")?.to_string();
    Ok(Installation { app: plan.id.clone(), version: plan.version.clone(), kind: plan.kind, path, installed_at: clock(), active: true })
}

/// Stream the asset into `<dir>/<file>.part`, resuming what an earlier attempt left, and rename
/// it to `<file>` once it has exactly the listed size.
fn download(plan: &Plan, transport: &dyn Transport, dir: &Path, cancel: &AtomicBool, tx: &Sender<JobMsg>) -> std::result::Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", plan.file));
    let done = dir.join(&plan.file);
    let _ = std::fs::remove_file(&done);
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > plan.size {
        have = 0;
    }
    for _ in 0..2 {
        let d = match transport.download(&plan.url, have) {
            Err(NetError::Status { status: 416, .. }) if have > 0 => {
                // What we kept doesn't fit the file any more: start over.
                have = 0;
                continue;
            }
            Err(e) => return Err(format!("couldn't download {}: {e}", plan.file)),
            Ok(d) => d,
        };
        let resuming = have > 0 && d.offset == have;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resuming)
            .truncate(!resuming)
            .open(&part)
            .map_err(|e| format!("{}: {e}", part.display()))?;
        if !resuming {
            have = 0;
        }
        send(tx, "Downloading", have, Some(plan.size));
        let mut reader = d.reader;
        let mut buf = vec![0u8; 64 * 1024];
        let mut reported = have;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let n = reader.read(&mut buf).map_err(|e| format!("the download of {} stopped: {e}", plan.file))?;
            let Some(chunk) = buf.get(..n) else { break };
            if n == 0 {
                break;
            }
            have = have.saturating_add(n as u64);
            if have > plan.size {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(format!("{} is larger than its release says ({} bytes); not installing it", plan.file, plan.size));
            }
            file.write_all(chunk).map_err(|e| format!("{}: {e}", part.display()))?;
            if have - reported >= PROGRESS_STEP {
                reported = have;
                send(tx, "Downloading", have, Some(plan.size));
            }
        }
        file.sync_all().map_err(|e| format!("{}: {e}", part.display()))?;
        if have != plan.size {
            return Err(format!("the download of {} ended early ({have} of {} bytes); installing again resumes it", plan.file, plan.size));
        }
        std::fs::rename(&part, &done).map_err(|e| format!("{}: {e}", done.display()))?;
        return Ok(done);
    }
    Err(format!("couldn't download {}", plan.file))
}

pub(crate) fn apply(s: &mut Session, job: &mut Running, msg: InstallMsg) {
    let JobState::Install(state) = &mut job.state else { return };
    match msg {
        InstallMsg::Progress { phase, done, total } => {
            state.phase = phase.to_string();
            state.done = done;
            state.total = total;
        }
        InstallMsg::Installed(inst) => {
            let summary = json!({"app": inst.app, "version": inst.version, "path": inst.path});
            let recorded = s.inventory.record(inst).map_err(EngineError::from).and_then(|()| s.save_inventory());
            state.outcome = Some(match recorded {
                Ok(()) => Ok(summary),
                Err(e) => Err(format!("installed, but the inventory couldn't be saved: {e}")),
            });
        }
        InstallMsg::Failed(e) => state.outcome = Some(Err(e)),
    }
}

pub(crate) fn finish(job: Running) -> std::result::Result<Value, String> {
    match job.state {
        JobState::Install(s) => s.outcome.unwrap_or_else(|| Err("the install stopped unexpectedly".into())),
        _ => Err("not an install".into()),
    }
}

fn uninstall(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?.clone();
    if s.job_for(INSTALL, &app.id).is_some() {
        return Err(refused(format!("{} is being installed", app.name)));
    }
    let inst = s.inventory().current(&app.id).cloned().ok_or_else(|| refused(format!("{} isn't installed", app.name)))?;
    let layout = s.layout.clone().ok_or_else(|| refused("this session has no install location"))?;
    let (version, bundle_id) = (inst.version.to_string(), app.bundle_id());
    let info = AppInfo { id: &app.id, name: &app.name, bundle_id: &bundle_id, tagline: &app.tagline, version: &version, icon_png: None };
    artcraft_toolbox_install::uninstall(&layout, &info, inst.kind, Path::new(&inst.path))?;
    s.inventory.remove(&app.id, &inst.version);
    s.save_inventory()?;
    Ok(json!({"app": app.id, "version": inst.version, "removed": inst.path}))
}

fn launch(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?;
    let inst = s.inventory().current(&app.id).ok_or_else(|| refused(format!("{} isn't installed", echo(&app.name))))?;
    artcraft_toolbox_install::launch(inst.kind, Path::new(&inst.path))?;
    Ok(json!({"app": app.id, "version": inst.version, "path": inst.path}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;
    use artcraft_toolbox_catalog::Catalog;
    use artcraft_toolbox_net::{Download, RateLimit, Response};
    use artcraft_toolbox_release::Target;
    use artcraft_toolbox_store::Store;
    use std::sync::Mutex;

    const URL: &str = "https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-linux-x86_64.AppImage";
    const SUMS: &str = "https://github.com/storytold/photocraft/releases/download/v0.5.0/SHA256SUMS.txt";

    fn appimage() -> Vec<u8> {
        let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
        b.extend((0..200_000u32).map(|i| (i % 251) as u8));
        b
    }

    fn feed(size: usize) -> String {
        json!([{
            "tag_name": "v0.5.0",
            "assets": [
                {"name": "photocraft-0.5.0-linux-x86_64.AppImage", "size": size, "browser_download_url": URL},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": SUMS},
            ]
        }])
        .to_string()
    }

    fn sha(bytes: &[u8]) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A GitHub that serves one release: its checksums and its AppImage (with ranges).
    struct Fake {
        body: Vec<u8>,
        sums: String,
        resumed_from: Mutex<Vec<u64>>,
    }

    impl Transport for Fake {
        fn get(&self, req: &Request<'_>) -> std::result::Result<Response, NetError> {
            assert_eq!(req.url, SUMS);
            Ok(Response::Ok { body: self.sums.as_bytes().to_vec(), etag: None, rate: RateLimit::default() })
        }
        fn download(&self, url: &str, from: u64) -> std::result::Result<Download, NetError> {
            assert_eq!(url, URL);
            self.resumed_from.lock().unwrap().push(from);
            let rest = self.body.get(from as usize..).unwrap_or_default().to_vec();
            Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(self.body.len() as u64) })
        }
    }

    struct Setup {
        root: PathBuf,
        session: Session,
        fake: Arc<Fake>,
    }

    fn setup(body: Vec<u8>, sums: String, listed_size: usize) -> Setup {
        let root = temp("install");
        let store = Store::open(root.join("data")).unwrap();
        let fake = Arc::new(Fake { body, sums, resumed_from: Mutex::new(Vec::new()) });
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(store), Some(fake.clone()));
        s.set_host(Target::from_consts("linux", "x86_64"));
        s.set_layout(Some(Layout {
            apps: root.join("apps"),
            desktop_entries: Some(root.join("share/applications")),
            icons: Some(root.join("share/icons")),
            start_menu: None,
            downloads: root.join("data/downloads"),
        }));
        s.ingest_releases("photocraft", &feed(listed_size), 1).unwrap();
        Setup { root, session: s, fake }
    }

    fn good() -> Setup {
        let body = appimage();
        let sums = format!("{}  photocraft-0.5.0-linux-x86_64.AppImage\n", sha(&body));
        let n = body.len();
        setup(body, sums, n)
    }

    #[test]
    fn install_downloads_verifies_installs_and_records() {
        let Setup { root, mut session, .. } = good();
        let out = session.execute(INSTALL, json!({"app": "photocraft"})).unwrap();
        // Joined part by part: the path is compared as text, with the platform's separators.
        let path = root.join("apps").join("photocraft").join("0.5.0").join("photocraft.AppImage");
        assert_eq!(out["path"], path.to_str().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), appimage());
        assert!(root.join("share/applications/ai.storyteller.photocraft.desktop").is_file());
        let st = session.app_status("photocraft").unwrap();
        assert_eq!(st.status, crate::Status::UpToDate { installed: Version::new(0, 5, 0) });
        // Recorded on disk, and the downloaded package is gone.
        let saved = Store::open(root.join("data")).unwrap().load_inventory().unwrap().unwrap();
        assert_eq!(saved.current("photocraft").map(|i| i.path.clone()), Some(path.to_str().unwrap().to_string()));
        assert_eq!(std::fs::read_dir(root.join("data/downloads")).unwrap().count(), 0);
        // Installed once: a second install is refused.
        assert!(session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string().contains("already installed"));

        session.execute(UNINSTALL, json!({"app": "photocraft"})).unwrap();
        assert!(!path.exists());
        assert!(!root.join("share/applications/ai.storyteller.photocraft.desktop").exists());
        assert!(session.inventory().current("photocraft").is_none());
        assert!(session.execute(UNINSTALL, json!({"app": "photocraft"})).unwrap_err().to_string().contains("isn't installed"));
        assert!(session.execute(LAUNCH, json!({"app": "photocraft"})).unwrap_err().to_string().contains("isn't installed"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_checksum_mismatch_installs_nothing_and_deletes_the_download() {
        let body = appimage();
        let sums = format!("{}  photocraft-0.5.0-linux-x86_64.AppImage\n", sha(b"something else"));
        let n = body.len();
        let Setup { root, mut session, .. } = setup(body, sums, n);
        let e = session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string();
        assert!(e.contains("doesn't match its checksum"), "{e}");
        assert!(!root.join("apps").exists() || std::fs::read_dir(root.join("apps")).unwrap().count() == 0);
        assert_eq!(std::fs::read_dir(root.join("data/downloads")).unwrap().count(), 0);
        assert!(session.inventory().current("photocraft").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_checksum_entry_no_install() {
        let body = appimage();
        let n = body.len();
        let Setup { root, mut session, fake } = setup(body, format!("{}  some-other-file.zip\n", "0".repeat(64)), n);
        let e = session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string();
        assert!(e.contains("doesn't list"), "{e}");
        assert!(fake.resumed_from.lock().unwrap().is_empty(), "nothing is downloaded without a checksum");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_partial_download_is_resumed() {
        let Setup { root, mut session, fake } = good();
        let downloads = root.join("data/downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("photocraft-0.5.0-linux-x86_64.AppImage.part"), &appimage()[..70_000]).unwrap();
        session.execute(INSTALL, json!({"app": "photocraft"})).unwrap();
        assert_eq!(*fake.resumed_from.lock().unwrap(), [70_000]);
        assert_eq!(std::fs::read(root.join("apps/photocraft/0.5.0/photocraft.AppImage")).unwrap(), appimage());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn more_bytes_than_announced_is_refused() {
        let body = appimage();
        let sums = format!("{}  photocraft-0.5.0-linux-x86_64.AppImage\n", sha(&body));
        let Setup { root, mut session, .. } = setup(body, sums, 1000);
        let e = session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string();
        assert!(e.contains("larger than its release says"), "{e}");
        assert_eq!(std::fs::read_dir(root.join("data/downloads")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installing_needs_a_place_a_release_and_a_readable_inventory() {
        let Setup { root, mut session, .. } = good();
        for (p, want) in [
            (json!({"app": "photocraft", "version": "9.9.9"}), "has no release 9.9.9"),
            (json!({"app": "vectorcraft"}), "check for updates first"),
            (json!({"app": "photocraft", "version": 5}), "must be a string"),
        ] {
            let e = session.execute(INSTALL, p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        session.inventory_locked = Some("inventory.json can't be read".into());
        assert!(session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string().contains("can't be read"));
        session.inventory_locked = None;
        session.set_layout(None);
        assert!(session.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string().contains("no install location"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_background_install_reports_progress() {
        let Setup { root, mut session, .. } = good();
        let crate::Started::Job(id) = session.start(INSTALL, json!({"app": "photocraft"})).unwrap() else { panic!("a job") };
        let mut events = Vec::new();
        for _ in 0..500 {
            if let Some(j) = session.job_for(INSTALL, "photocraft") {
                assert!(j.phase.is_some());
            }
            events.extend(session.poll_jobs());
            if !session.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, id);
        assert!(events[0].result.is_ok(), "{:?}", events[0].result);
        assert!(session.inventory().current("photocraft").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
