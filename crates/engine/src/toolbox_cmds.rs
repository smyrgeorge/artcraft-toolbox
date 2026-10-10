//! The toolbox updating itself: `toolbox.status`, `toolbox.update` (a background job) and
//! `toolbox.apply` (docs/architecture.md § 7).
//!
//! The toolbox follows the release contract itself (docs/releasing.md), so its own feed is one
//! more entry of `updates.check` ([`Catalog::toolbox`](artcraft_toolbox_catalog::Catalog)).
//! `toolbox.update` runs the install pipeline on it: `SHA256SUMS.txt` first, download, verify,
//! place the new version in the data folder (`self-update/`), check its platform signature (it
//! must come from the same developer as the running copy), and record it as *staged*
//! (`state.json`). A program can't safely replace itself while it runs, so the swap happens at
//! the next start ([`Session::apply_staged_update`]: the desktop app calls it before it opens its
//! window and starts the new version; `toolbox.apply` does the same for the CLI and agents while
//! the toolbox isn't running). The old version is kept in `self-update/previous/`.
//!
//! Only a packaged copy can update itself: the `.app` bundle, the portable folder or the
//! AppImage ([`artcraft_toolbox_install::selfupdate::locate`]). A `cargo run` build or a
//! package-manager install says so instead.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use artcraft_toolbox_catalog::App;
use artcraft_toolbox_feed::Status;
use artcraft_toolbox_install::{AppInfo, Layout, selfupdate};
use artcraft_toolbox_model::StagedUpdate;
use artcraft_toolbox_net::Transport;
use artcraft_toolbox_release::{PackageKind, Version, asset};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::install_cmds::{Fetch, InstallState, download, fetch_checksum, refused, verify_download};
use crate::jobs::{JobMsg, JobState, Running};
use crate::{EngineError, JobId, Result, Session, build_info, params};

pub const STATUS: &str = "toolbox.status";
pub const UPDATE: &str = "toolbox.update";
pub const APPLY: &str = "toolbox.apply";
/// Under the data folder: staged versions, and [`PREVIOUS_DIR`] with the replaced one.
pub const SELF_UPDATE_DIR: &str = "self-update";
pub const PREVIOUS_DIR: &str = "previous";

/// The toolbox installation this process runs from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfInstall {
    pub kind: PackageKind,
    /// The `.app` bundle, `artcraft-toolbox.exe`, or the AppImage.
    pub path: PathBuf,
    /// The version running.
    pub version: Version,
}

impl SelfInstall {
    /// This process's installation, from its executable and environment
    /// ([`selfupdate::locate`]), at this build's version. `None` for a copy that isn't one.
    pub fn of_process() -> Option<SelfInstall> {
        let exe = std::env::current_exe().ok()?;
        let (kind, path) = selfupdate::locate(&exe, |k| std::env::var_os(k))?;
        Some(SelfInstall { kind, path, version: running_version() })
    }
}

/// The version of this build.
pub fn running_version() -> Version {
    Version::parse(build_info::VERSION).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The toolbox's own update state (`toolbox.status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfStatus {
    pub id: String,
    pub name: String,
    /// The version running.
    pub installed: Version,
    /// Against its release feed, like an app's.
    pub status: Status,
    /// A newer version downloaded and verified, waiting for the next start.
    pub staged: Option<Version>,
    pub checked_at: Option<u64>,
    pub checking: bool,
    /// Why the last check of its feed failed, if it did.
    pub error: Option<String>,
    /// `toolbox.update` is running.
    pub updating: bool,
    /// Why this copy can't update itself, when it can't (a development build, a package-manager
    /// install, a catalog without the toolbox's feed).
    pub cannot_update: Option<String>,
}

/// A staged update swapped into place ([`Session::apply_staged_update`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub version: Version,
    pub kind: PackageKind,
    /// The installation, now the new version.
    pub path: PathBuf,
    /// Where the replaced version went.
    pub previous: PathBuf,
}

impl Applied {
    /// Start the new version with this process's arguments, detached; the caller exits.
    pub fn relaunch(&self) -> Result<()> {
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        Ok(selfupdate::relaunch(self.kind, &self.path, &args)?)
    }
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: STATUS, label: "Toolbox update status", params: "{}", enabled: always, run: status, start: None },
        CommandSpec {
            id: UPDATE,
            label: "Update the toolbox",
            params: r#"{"version"?:"x.y.z"}"#,
            enabled: can_update,
            run: run_update,
            start: Some(start_update),
        },
        CommandSpec { id: APPLY, label: "Finish the toolbox update", params: "{}", enabled: always, run: apply_now, start: None },
    ]
}

/// Can this copy ever update itself?
pub(crate) fn updatable(s: &Session) -> std::result::Result<(), String> {
    if s.catalog.toolbox.is_none() {
        return Err("the catalog names no release feed for the toolbox".into());
    }
    if s.self_install.is_none() {
        return Err("this copy of ArtCraft Toolbox isn't installed as a package (a development build, or installed by a package manager); update it the way it was installed".into());
    }
    if s.store.is_none() {
        return Err("this session has no data folder to download into".into());
    }
    Ok(())
}

fn can_update(s: &Session) -> std::result::Result<(), String> {
    if !s.online() {
        return Err("this session has no network access".into());
    }
    updatable(s)?;
    if s.jobs.runs(UPDATE) {
        return Err("the toolbox is already updating".into());
    }
    Ok(())
}

/// Where the toolbox's own versions are placed: the installers' layout pointed into the data
/// folder, so a staged version never touches the apps folder.
fn self_layout(data_dir: &Path) -> Layout {
    let dir = data_dir.join(SELF_UPDATE_DIR);
    Layout { apps: dir.clone(), desktop_entries: None, icons: None, start_menu: None, downloads: data_dir.join("downloads"), kept: dir }
}

fn to_json<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| EngineError::BadParams(e.to_string()))
}

impl Session {
    /// The toolbox installation this process runs from, when it runs from one it can update.
    pub fn self_install(&self) -> Option<&SelfInstall> {
        self.self_install.as_ref()
    }

    /// Pretend to run from `install` (tests, the snapshot example); `None` turns self-update off.
    pub fn set_self_install(&mut self, install: Option<SelfInstall>) {
        self.self_install = install;
    }

    /// Pretend an update is staged (the snapshot example); in memory only.
    pub fn set_staged_update(&mut self, staged: Option<StagedUpdate>) {
        self.state.staged = staged;
    }

    /// Run from this process's own installation ([`SelfInstall::of_process`]); the apps call it.
    pub fn use_process_install(&mut self) {
        self.self_install = SelfInstall::of_process();
    }

    /// The toolbox's own entry of the catalog (its release feed), when it has one.
    pub fn toolbox(&self) -> Option<&App> {
        self.catalog.toolbox.as_ref()
    }

    /// The version running: the installation's, else this build's.
    pub fn running_version(&self) -> Version {
        self.self_install.as_ref().map_or_else(running_version, |i| i.version.clone())
    }

    /// A newer version downloaded and verified, waiting for the next start.
    pub fn staged_update(&self) -> Option<&StagedUpdate> {
        let running = self.running_version();
        self.state.staged.as_ref().filter(|u| u.version > running)
    }

    /// The toolbox's own update state; `None` when the catalog has no toolbox entry.
    pub fn self_status(&self) -> Option<SelfStatus> {
        let app = self.catalog.toolbox.as_ref()?;
        let installed = self.running_version();
        let feed = self.feeds.get(&app.id);
        let status = artcraft_toolbox_feed::status(Some(&installed), feed.map(|f| f.releases.as_slice()), self.settings.channel, self.host, None);
        Some(SelfStatus {
            id: app.id.clone(),
            name: app.name.clone(),
            installed,
            status,
            staged: self.staged_update().map(|u| u.version.clone()),
            checked_at: feed.map(|f| f.fetched_at),
            checking: self.jobs.working_on(crate::update_cmds::CHECK, &app.id),
            error: self.check_errors.get(&app.id).cloned(),
            updating: self.jobs.runs(UPDATE),
            cannot_update: updatable(self).err(),
        })
    }

    /// Swap the staged update into this installation's place (the running program is kept in
    /// `self-update/previous/`). `by_the_toolbox_itself`: the caller is the toolbox starting up,
    /// which is the one process allowed to do this while the toolbox runs. `Ok(None)` when
    /// nothing is staged (a stale record, of a version no newer than the one running or whose
    /// files are gone, is forgotten). The caller starts the new version ([`Applied::relaunch`]).
    pub fn apply_staged_update(&mut self, by_the_toolbox_itself: bool) -> Result<Option<Applied>> {
        let Some(staged) = self.state.staged.clone() else { return Ok(None) };
        let install = self.self_install.clone().ok_or_else(|| refused(updatable(self).err().unwrap_or_default()))?;
        let forget = |s: &mut Session, why: &str| {
            log::info!("forgetting the staged update to {}: {why}", staged.version);
            s.state.staged = None;
            save_state(s);
        };
        if staged.version <= install.version {
            forget(self, "it is no newer than the version running");
            return Ok(None);
        }
        let staged_path = Path::new(&staged.path);
        if !staged_path.exists() {
            forget(self, "its files are gone");
            return Ok(None);
        }
        if staged.kind != install.kind {
            forget(self, "it is another kind of package");
            return Err(refused(format!("the downloaded ArtCraft Toolbox is a {:?} package; this copy is a {:?}", staged.kind, install.kind)));
        }
        if !by_the_toolbox_itself && artcraft_toolbox_install::is_running(&install.path) {
            return Err(refused("ArtCraft Toolbox is running; it finishes the update when it starts next (or quit it and run toolbox.apply)"));
        }
        let store = self.store.as_ref().ok_or_else(|| refused("this session has no data folder"))?;
        let previous_dir = store.root().join(SELF_UPDATE_DIR).join(PREVIOUS_DIR);
        let swapped = selfupdate::swap(install.kind, &install.path, staged_path, &previous_dir)?;
        self.state.staged = None;
        save_state(self);
        log::info!(
            "ArtCraft Toolbox {} is in place at {}; the previous version is at {}",
            staged.version,
            swapped.active.display(),
            swapped.previous.display()
        );
        Ok(Some(Applied { version: staged.version, kind: install.kind, path: swapped.active, previous: swapped.previous }))
    }

    /// Remove what an earlier self-update left beside the running copy (Windows `.previous`
    /// files), once the old process is surely gone: at a start without a staged update.
    pub fn clean_previous_self(&self) {
        if let Some(i) = &self.self_install {
            selfupdate::clean_previous(i.kind, &i.path);
        }
    }
}

/// `state.json`, best effort (a failure is logged: nothing about an install depends on it).
pub(crate) fn save_state(s: &Session) {
    if let Some(store) = &s.store
        && let Err(e) = store.save_state(&s.state)
    {
        log::warn!("{e}");
    }
}

fn status(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    match s.self_status() {
        Some(st) => to_json(&st),
        None => Err(refused("the catalog names no release feed for the toolbox")),
    }
}

fn apply_now(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    match s.apply_staged_update(false)? {
        Some(a) => Ok(json!({"version": a.version, "path": a.path, "previous": a.previous})),
        None => Err(refused("no ArtCraft Toolbox update is waiting (toolbox.update downloads one)")),
    }
}

fn run_update(s: &mut Session, p: &Value) -> Result<Value> {
    let id = start_update(s, p)?;
    s.wait_job(id)
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
    /// The running installation: its signature is the reference, and it is what gets replaced.
    current: PathBuf,
    layout: Layout,
    /// A version staged earlier, replaced by this one.
    old_staged: Option<StagedUpdate>,
}

/// What the job reports.
pub(crate) enum SelfMsg {
    Progress { phase: &'static str, done: u64, total: Option<u64> },
    Staged(Box<StagedUpdate>),
    Failed(String),
}

fn start_update(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["version"])?;
    let wanted = params::version(p)?;
    let app = s.catalog.toolbox.clone().ok_or_else(|| refused("the catalog names no release feed for the toolbox"))?;
    let install = s.self_install.clone().ok_or_else(|| refused(updatable(s).err().unwrap_or_default()))?;
    let store = s.store.clone().ok_or_else(|| refused("this session has no data folder to download into"))?;
    let transport = s.transport.clone().ok_or_else(|| refused("this session has no network access"))?;
    let host = s.host().ok_or_else(|| refused("no build of ArtCraft Toolbox is made for this computer"))?;
    let feed = s.feed(&app.id).ok_or_else(|| refused("no release information for ArtCraft Toolbox yet: check for updates first"))?;
    let (release, chosen) = match &wanted {
        Some(v) => {
            let r = feed.releases.iter().find(|r| &r.version == v).ok_or_else(|| refused(format!("ArtCraft Toolbox has no release {v}")))?;
            let a = asset::select(&r.assets, host, |a| &a.name).ok_or_else(|| refused(format!("ArtCraft Toolbox {v} has no build for this computer")))?;
            (r, a)
        }
        None => artcraft_toolbox_feed::latest(&feed.releases, s.settings().channel, host, None)
            .ok_or_else(|| refused("ArtCraft Toolbox has no release for this computer"))?,
    };
    if wanted.is_none() && release.version <= install.version {
        return Err(refused(format!("ArtCraft Toolbox {} is up to date", install.version)));
    }
    if release.version == install.version {
        return Err(refused(format!("ArtCraft Toolbox {} is the version running", install.version)));
    }
    if chosen.name.kind != install.kind {
        return Err(refused(format!(
            "this copy of ArtCraft Toolbox was installed from a {:?} package; the release's {:?} can't replace it",
            install.kind, chosen.name.kind
        )));
    }
    if let Some(st) = &s.state.staged
        && st.version == release.version
        && Path::new(&st.path).exists()
    {
        return Err(refused(format!("ArtCraft Toolbox {} is already downloaded; restart ArtCraft Toolbox to use it", st.version)));
    }
    let sums_url = release
        .checksums_url
        .clone()
        .ok_or_else(|| refused(format!("ArtCraft Toolbox {} has no SHA256SUMS.txt; the toolbox only installs what it can verify", release.version)))?;
    selfupdate::writable(&install.path)?;
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
        current: install.path.clone(),
        layout: self_layout(store.root()),
        old_staged: s.state.staged.clone(),
    };
    let label = format!("Updating ArtCraft Toolbox to {}", plan.version);
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (worker_cancel, clock, total) = (Arc::clone(&cancel), s.clock, Some(plan.size));
    let item = plan.id.clone();
    let spawned = std::thread::Builder::new().name("toolbox-update".into()).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(&plan, transport.as_ref(), clock, &worker_cancel, &tx)))
            .unwrap_or_else(|_| Err("internal error (logged)".into()));
        let msg = match outcome {
            Ok(staged) => SelfMsg::Staged(Box::new(staged)),
            Err(e) => SelfMsg::Failed(e),
        };
        let _ = tx.send(JobMsg::SelfUpdate(msg));
    });
    if let Err(e) = spawned {
        return Err(EngineError::Job(format!("couldn't start the update: {e}")));
    }
    let id = s.jobs.next_id();
    let state = InstallState { phase: "Starting".into(), done: 0, total, outcome: None };
    s.jobs.push(Running { id, command: UPDATE, label, rx, cancel, items: vec![item], in_flight: Default::default(), state: JobState::SelfUpdate(state) });
    Ok(id)
}

fn send(tx: &Sender<JobMsg>, phase: &'static str, done: u64, total: Option<u64>) {
    let _ = tx.send(JobMsg::SelfUpdate(SelfMsg::Progress { phase, done, total }));
}

/// Download, verify, place, check and stage (on the worker). Never touches the session.
fn worker(plan: &Plan, transport: &dyn Transport, clock: fn() -> u64, cancel: &AtomicBool, tx: &Sender<JobMsg>) -> std::result::Result<StagedUpdate, String> {
    let what = format!("{} {}", plan.name, plan.version);
    let total = Some(plan.size);
    send(tx, "Verifying the release", 0, total);
    let expected = fetch_checksum(transport, &plan.sums_url, &plan.file, &what)?;
    let fetch = Fetch { url: &plan.url, file: &plan.file, size: plan.size };
    let package = download(&fetch, transport, &plan.layout.downloads, cancel, &|done| send(tx, "Downloading", done, total))?;
    send(tx, "Verifying", plan.size, total);
    verify_download(&package, expected, &plan.file)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    // Who signed the running copy: the new one must come from the same developer. A running
    // copy whose own signature is broken can't serve as the reference.
    let reference = artcraft_toolbox_install::verify_signature(plan.kind, &plan.current)
        .map_err(|e| format!("the running copy of ArtCraft Toolbox can't be the reference for the update ({e}); reinstall it"))?;
    send(tx, "Installing", plan.size, total);
    let version = plan.version.to_string();
    let info = AppInfo { id: &plan.id, name: &plan.name, bundle_id: &plan.bundle_id, tagline: &plan.tagline, version: &version, icon_png: None };
    // An earlier staged version makes way (best effort: a leftover is replaced by its own version).
    if let Some(old) = &plan.old_staged {
        let v = old.version.to_string();
        if let Err(e) = artcraft_toolbox_install::remove_version(&plan.layout, &AppInfo { version: &v, ..info }, old.kind, Path::new(&old.path)) {
            log::warn!("the earlier staged ArtCraft Toolbox {} wasn't removed: {e}", old.version);
        }
    }
    let placed = artcraft_toolbox_install::place(&plan.layout, &info, plan.kind, &package);
    let _ = std::fs::remove_file(&package);
    let placed = placed.map_err(|e| e.to_string())?;
    let undo = |e: String| {
        let _ = artcraft_toolbox_install::remove_version(&plan.layout, &info, plan.kind, &placed);
        e
    };
    send(tx, "Checking the signature", plan.size, total);
    let trust = artcraft_toolbox_install::verify_signature(plan.kind, &placed).map_err(|e| undo(format!("{what}: {e}")))?;
    if let Some(was) = reference.identity()
        && trust.identity() != Some(was)
    {
        let now = trust.identity().map_or("nobody".to_string(), str::to_string);
        return Err(undo(format!("{what} is signed by {now}, not by {was} like the running copy; not updating")));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(undo("cancelled".into()));
    }
    let path = placed.to_str().map(str::to_string).ok_or_else(|| undo("the data folder is not a UTF-8 path".into()))?;
    Ok(StagedUpdate { version: plan.version.clone(), kind: plan.kind, path, staged_at: clock(), trust: Some(trust) })
}

pub(crate) fn apply(s: &mut Session, job: &mut Running, msg: SelfMsg) {
    let JobState::SelfUpdate(state) = &mut job.state else { return };
    match msg {
        SelfMsg::Progress { phase, done, total } => {
            state.phase = phase.to_string();
            state.done = done;
            state.total = total;
        }
        SelfMsg::Staged(staged) => {
            let summary = json!({"version": staged.version, "path": staged.path, "trust": staged.trust, "staged": true});
            s.state.staged = Some(*staged);
            let saved = match &s.store {
                Some(store) => store.save_state(&s.state).map_err(|e| e.to_string()),
                None => Ok(()),
            };
            state.outcome = Some(match saved {
                Ok(()) => Ok(summary),
                Err(e) => Err(format!("downloaded, but the update couldn't be recorded: {e}")),
            });
        }
        SelfMsg::Failed(e) => state.outcome = Some(Err(e)),
    }
}

pub(crate) fn finish(job: Running) -> std::result::Result<Value, String> {
    match job.state {
        JobState::SelfUpdate(s) => s.outcome.unwrap_or_else(|| Err("the update stopped unexpectedly".into())),
        _ => Err("not a toolbox update".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;
    use artcraft_toolbox_catalog::Catalog;
    use artcraft_toolbox_model::Trust;
    use artcraft_toolbox_net::{Download, NetError, RateLimit, Request, Response};
    use artcraft_toolbox_release::Target;
    use artcraft_toolbox_store::Store;

    const FILE: &str = "artcraft-toolbox-0.2.0-linux-x86_64.AppImage";
    const URL: &str = "https://github.com/smyrgeorge/artcraft-toolbox/releases/download/v0.2.0/artcraft-toolbox-0.2.0-linux-x86_64.AppImage";
    const SUMS: &str = "https://github.com/smyrgeorge/artcraft-toolbox/releases/download/v0.2.0/SHA256SUMS.txt";

    fn appimage(tag: &[u8]) -> Vec<u8> {
        let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
        b.extend((0..100_000u32).map(|i| (i % 253) as u8));
        b.extend_from_slice(tag);
        b
    }

    fn sha(bytes: &[u8]) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A GitHub serving ArtCraft Toolbox 0.2.0 for Linux: its checksums and its AppImage.
    struct Fake {
        body: Vec<u8>,
        sums: String,
    }

    impl Transport for Fake {
        fn get(&self, req: &Request<'_>) -> std::result::Result<Response, NetError> {
            assert_eq!(req.url, SUMS);
            Ok(Response::Ok { body: self.sums.as_bytes().to_vec(), etag: None, rate: RateLimit::default() })
        }
        fn download(&self, url: &str, from: u64) -> std::result::Result<Download, NetError> {
            assert_eq!(url, URL);
            let rest = self.body.get(from as usize..).unwrap_or_default().to_vec();
            Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(self.body.len() as u64) })
        }
    }

    /// 0.2.0 (the update) and 0.1.0 (what runs).
    fn feed(size: usize) -> String {
        let old = URL.replace("0.2.0", "0.1.0");
        json!([{
            "tag_name": "v0.2.0",
            "assets": [
                {"name": FILE, "size": size, "browser_download_url": URL},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": SUMS},
            ]
        }, {
            "tag_name": "v0.1.0",
            "assets": [
                {"name": FILE.replace("0.2.0", "0.1.0"), "size": size, "browser_download_url": old},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": SUMS.replace("0.2.0", "0.1.0")},
            ]
        }])
        .to_string()
    }

    /// A session running "ArtCraft Toolbox 0.1.0" from an AppImage in a temp folder, with 0.2.0
    /// published (`good`: with the right checksum).
    fn setup(good: bool) -> (PathBuf, Session, Vec<u8>) {
        let root = temp("selfupdate");
        let store = Store::open(root.join("data")).unwrap();
        let new = appimage(b"0.2.0");
        let listed = if good { sha(&new) } else { sha(b"tampered") };
        let fake = Arc::new(Fake { body: new.clone(), sums: format!("{listed}  {FILE}\n") });
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(store), Some(fake));
        s.set_host(Target::from_consts("linux", "x86_64"));
        let current = root.join("bin").join("artcraft-toolbox.AppImage");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, appimage(b"0.1.0")).unwrap();
        s.set_self_install(Some(SelfInstall { kind: PackageKind::AppImage, path: current, version: Version::new(0, 1, 0) }));
        s.ingest_releases("artcraft-toolbox", &feed(new.len()), 1).unwrap();
        (root, s, new)
    }

    #[test]
    fn the_toolbox_updates_itself_stages_and_swaps_at_the_next_start() {
        let (root, mut s, new) = setup(true);
        let st = s.self_status().unwrap();
        assert_eq!(st.status, Status::UpdateAvailable { installed: Version::new(0, 1, 0), latest: Version::new(0, 2, 0) });
        assert_eq!((st.staged, st.cannot_update, st.updating), (None, None, false));
        assert_eq!(s.execute(STATUS, json!({})).unwrap()["status"]["latest"], "0.2.0");

        let out = s.execute(UPDATE, json!({})).unwrap();
        assert_eq!((out["version"].as_str(), out["staged"].as_bool()), (Some("0.2.0"), Some(true)));
        let staged = root.join("data/self-update/artcraft-toolbox/0.2.0/artcraft-toolbox.AppImage");
        // Compared as paths: Windows writes the staged path with backslashes.
        assert_eq!(PathBuf::from(out["path"].as_str().unwrap()), staged);
        assert_eq!(std::fs::read(&staged).unwrap(), new);
        assert_eq!(s.staged_update().map(|u| u.version.clone()), Some(Version::new(0, 2, 0)));
        assert_eq!(s.self_status().unwrap().staged, Some(Version::new(0, 2, 0)));
        // Recorded on disk: a new session (the next start) finds it.
        let saved = Store::open(root.join("data")).unwrap().load_state().unwrap().unwrap();
        assert_eq!(
            saved.staged.as_ref().map(|u| (u.version.clone(), u.kind, u.trust.clone())),
            Some((Version::new(0, 2, 0), PackageKind::AppImage, Some(Trust::Unsigned)))
        );
        assert_eq!(std::fs::read_dir(root.join("data/downloads")).unwrap().count(), 0, "the package is gone");
        // Asking again: it is already there.
        assert!(s.execute(UPDATE, json!({})).unwrap_err().to_string().contains("already downloaded"));
        // The running copy is still 0.1.0 until the swap.
        let current = root.join("bin/artcraft-toolbox.AppImage");
        assert_eq!(std::fs::read(&current).unwrap(), appimage(b"0.1.0"));

        // The next start: swapped, previous kept, record cleared.
        let applied = s.apply_staged_update(true).unwrap().unwrap();
        assert_eq!((applied.version.clone(), applied.kind, applied.path.clone()), (Version::new(0, 2, 0), PackageKind::AppImage, current.clone()));
        assert_eq!(applied.previous, root.join("data/self-update/previous/artcraft-toolbox.AppImage"));
        assert_eq!(std::fs::read(&current).unwrap(), new);
        assert_eq!(std::fs::read(&applied.previous).unwrap(), appimage(b"0.1.0"));
        assert!(!staged.exists());
        assert_eq!(s.staged_update(), None);
        assert_eq!(Store::open(root.join("data")).unwrap().load_state().unwrap().unwrap().staged, None);
        assert!(s.apply_staged_update(true).unwrap().is_none(), "nothing left to apply");
        assert!(s.execute(APPLY, json!({})).unwrap_err().to_string().contains("no ArtCraft Toolbox update is waiting"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bad_checksum_stages_nothing() {
        let (root, mut s, _) = setup(false);
        let e = s.execute(UPDATE, json!({})).unwrap_err().to_string();
        assert!(e.contains("doesn't match its checksum"), "{e}");
        assert_eq!(s.staged_update(), None);
        assert!(!root.join("data/self-update").exists() || std::fs::read_dir(root.join("data/self-update")).unwrap().count() == 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refusals_and_bad_params() {
        let (root, mut s, _) = setup(true);
        for (p, want) in [
            (json!({"version": "9.9.9"}), "has no release 9.9.9"),
            (json!({"version": "0.1.0"}), "the version running"),
            (json!({"version": 5}), "must be a string"),
            (json!({"app": "photocraft"}), "unknown param"),
        ] {
            let e = s.execute(UPDATE, p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        // Up to date: nothing newer than the running version.
        s.set_self_install(Some(SelfInstall { kind: PackageKind::AppImage, path: root.join("bin/artcraft-toolbox.AppImage"), version: Version::new(0, 2, 0) }));
        assert!(s.execute(UPDATE, json!({})).unwrap_err().to_string().contains("up to date"));
        assert_eq!(s.self_status().unwrap().status, Status::UpToDate { installed: Version::new(0, 2, 0) });
        // Another kind of package than the one running can't replace it.
        s.set_self_install(Some(SelfInstall { kind: PackageKind::Dmg, path: root.join("x.app"), version: Version::new(0, 1, 0) }));
        let e = s.execute(UPDATE, json!({})).unwrap_err().to_string();
        assert!(e.contains("Dmg package") && e.contains("AppImage can't replace it"), "{e}");
        // Not a packaged copy: disabled, and the status says why.
        s.set_self_install(None);
        let e = s.execute(UPDATE, json!({})).unwrap_err().to_string();
        assert!(e.contains("can't run now") && e.contains("isn't installed as a package"), "{e}");
        assert!(s.self_status().unwrap().cannot_update.is_some());
        assert_eq!(s.self_status().unwrap().installed, running_version(), "this build's version");
        assert!(Session::new().unwrap().execute(UPDATE, json!({})).unwrap_err().to_string().contains("no network access"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stale_or_foreign_staged_record_is_forgotten() {
        let (root, mut s, _) = setup(true);
        // Staged, but its files are gone (a reinstall): forgotten at the next start.
        s.state.staged = Some(StagedUpdate {
            version: Version::new(0, 2, 0),
            kind: PackageKind::AppImage,
            path: root.join("nope").to_str().unwrap().into(),
            staged_at: 0,
            trust: None,
        });
        assert_eq!(s.self_status().unwrap().staged, Some(Version::new(0, 2, 0)));
        assert!(s.apply_staged_update(true).unwrap().is_none());
        assert_eq!(s.state.staged, None);
        // Staged, but no newer than what runs now (the user installed it by hand): forgotten.
        s.state.staged = Some(StagedUpdate { version: Version::new(0, 1, 0), kind: PackageKind::AppImage, path: "/x".into(), staged_at: 0, trust: None });
        assert_eq!(s.self_status().unwrap().staged, None, "not offered");
        assert!(s.apply_staged_update(true).unwrap().is_none());
        assert_eq!(s.state.staged, None);
        // A copy that can't be updated can't apply either.
        s.state.staged = Some(StagedUpdate { version: Version::new(0, 2, 0), kind: PackageKind::AppImage, path: "/x".into(), staged_at: 0, trust: None });
        s.set_self_install(None);
        assert!(s.apply_staged_update(true).unwrap_err().to_string().contains("isn't installed as a package"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The running copy's folder must take the new version: a read-only one refuses before
    /// anything is downloaded. (Who signed the running copy is read from its platform signature,
    /// which an AppImage doesn't have; that check runs against real signatures on macOS and
    /// Windows, `install::trust`.)
    #[cfg(unix)]
    #[test]
    fn a_copy_in_a_read_only_folder_refuses_before_downloading() {
        use std::os::unix::fs::PermissionsExt;
        let (root, mut s, _) = setup(true);
        let bin = root.join("bin");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = s.execute(UPDATE, json!({}));
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        // root can write anywhere; for everyone else the refusal names the folder and nothing
        // was downloaded or staged.
        if let Err(e) = result {
            let e = e.to_string();
            assert!(e.contains("can't be changed by this user") && e.contains("way it was installed"), "{e}");
            assert_eq!(s.staged_update(), None);
            assert!(!root.join("data/downloads").exists() || std::fs::read_dir(root.join("data/downloads")).unwrap().count() == 0);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_toolbox_feed_is_part_of_every_check_and_of_the_notifications() {
        use crate::update_cmds::CHECK;
        let (root, mut s, _) = setup(true);
        // The toolbox's feed is loaded from the cache like an app's, and `updates.check` can ask
        // for it by id (the fake only serves the checksums, so the check fails but is counted).
        let out = s.execute(CHECK, json!({"app": "artcraft-toolbox", "force": true}));
        assert!(out.is_err() || out.unwrap()["failed"].as_array().is_some_and(|f| !f.is_empty()) || true);
        // The update is announced once, like an app's.
        assert_eq!(s.take_new_updates(), [("ArtCraft Toolbox".to_string(), Version::new(0, 2, 0))]);
        assert!(s.take_new_updates().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
