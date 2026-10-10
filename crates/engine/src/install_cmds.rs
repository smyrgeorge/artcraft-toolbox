//! `app.install` and `app.update` (background jobs), `app.uninstall` and `app.launch`.
//!
//! Installing and updating are one pipeline, download → verify → place → check → activate →
//! record, on one worker thread:
//!
//! 1. the release's `SHA256SUMS.txt` is fetched first: no checksum entry, no install;
//! 2. the asset is streamed to `<data>/downloads/<file>.part`, resuming a partial file with an
//!    HTTP range, and must end at exactly the size GitHub listed;
//! 3. its SHA-256 must match, or it is deleted and nothing changes;
//! 4. the platform installer (`artcraft_toolbox_install`) places the version beside the others;
//! 5. its platform signature is checked: a broken one is refused, and an update must be signed by
//!    the same developer as the version it replaces (the identity recorded when that was
//!    installed);
//! 6. it is activated (macOS: swapped into the apps folder, the previous bundle kept), versions
//!    beyond `keepPrevious` are removed, and the session records it all.
//!
//! A failure at any step leaves the active version as it was. Cancelling keeps the `.part` file,
//! so the next attempt resumes it. See docs/architecture.md § 5 for where apps go.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use artcraft_toolbox_catalog::App;
use artcraft_toolbox_install::{AppInfo, Current, Layout};
use artcraft_toolbox_model::{Installation, Trust};
use artcraft_toolbox_net::{NetError, Request, Transport};
use artcraft_toolbox_release::{Checksums, Os, PackageKind, Sha256, Version, asset};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::jobs::{JobMsg, JobState, Running};
use crate::{EngineError, JobId, Result, Session, echo, params};

pub const INSTALL: &str = "app.install";
pub const UPDATE: &str = "app.update";
pub const UNINSTALL: &str = "app.uninstall";
pub const LAUNCH: &str = "app.launch";
/// Largest checksum file read (a real one is about 2 KB).
const MAX_SUMS_BYTES: usize = 1 << 20;
/// Progress is reported at most this often (bytes).
const PROGRESS_STEP: u64 = 512 * 1024;

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: INSTALL, label: "Install", params: r#"{"app":"<id>","version"?:"x.y.z"}"#, enabled: can_install, run, start: Some(start) },
        CommandSpec {
            id: UPDATE,
            label: "Update",
            params: r#"{"app":"<id>","version"?:"x.y.z"}"#,
            enabled: can_install,
            run: run_update,
            start: Some(start_update),
        },
        CommandSpec { id: UNINSTALL, label: "Uninstall", params: r#"{"app":"<id>"}"#, enabled: can_change_installs, run: uninstall, start: None },
        CommandSpec { id: LAUNCH, label: "Open", params: r#"{"app":"<id>"}"#, enabled: always, run: launch, start: None },
    ]
}

pub(crate) fn can_change_installs(s: &Session) -> std::result::Result<(), String> {
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
pub(crate) fn kind_for(os: Os) -> Option<PackageKind> {
    match os {
        Os::Macos => Some(PackageKind::Dmg),
        Os::Windows => Some(PackageKind::PortableZip),
        Os::Linux => Some(PackageKind::AppImage),
        _ => None,
    }
}

/// What the installer needs to know about `app` at `version`.
pub(crate) fn info<'a>(app: &'a App, bundle_id: &'a str, version: &'a str, icon_png: Option<&'a [u8]>) -> AppInfo<'a> {
    AppInfo { id: &app.id, name: &app.name, bundle_id, tagline: &app.tagline, version, icon_png }
}

/// The version an update replaces.
struct Replaced {
    version: Version,
    path: PathBuf,
    /// Who signed it: the new version must be signed by the same developer.
    identity: Option<String>,
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
    verify: Verify,
    icon_png: Option<Vec<u8>>,
    /// The active version, for an update.
    replaces: Option<Replaced>,
    /// Inactive versions to remove once the new one is active (beyond `keepPrevious`).
    prune: Vec<(Version, PathBuf, PackageKind)>,
}

/// Where the expected digest of a build comes from. A build with neither is never installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verify {
    /// The release's `SHA256SUMS.txt` (the contract).
    Sums(String),
    /// A digest the signed aggregated feed carries (apps outside the contract).
    Digest(Sha256),
}

/// How a release's build will be verified, or why it can't be.
pub(crate) fn verification(name: &str, version: &Version, checksums_url: Option<&str>, asset: &artcraft_toolbox_feed::Asset) -> Result<Verify> {
    if let Some(url) = checksums_url {
        return Ok(Verify::Sums(url.to_string()));
    }
    match asset.sha256.as_deref().and_then(Sha256::from_hex) {
        Some(digest) => Ok(Verify::Digest(digest)),
        None => Err(refused(format!(
            "{name} {version} has no SHA256SUMS.txt and the release feed carries no digest for {}; the toolbox only installs what it can verify (an app outside the contract needs the publisher's signed feed)",
            asset.file
        ))),
    }
}

impl Plan {
    /// `PhotoCraft 0.5.0`.
    fn what(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// What a finished install or update leaves to record.
pub(crate) struct Done {
    inst: Installation,
    /// Where the replaced version went, when it moved (macOS keeps it outside the apps folder).
    moved: Option<(Version, String)>,
    /// Versions removed for `keepPrevious`.
    pruned: Vec<Version>,
    /// What didn't work but doesn't undo the result (a version that couldn't be removed).
    warnings: Vec<String>,
}

/// What the install job reports.
pub(crate) enum InstallMsg {
    Progress { phase: &'static str, done: u64, total: Option<u64> },
    Installed(Box<Done>),
    Failed(String),
}

/// An install job's bookkeeping (also a toolbox update's, `toolbox_cmds`).
pub(crate) struct InstallState {
    pub(crate) phase: String,
    pub(crate) done: u64,
    pub(crate) total: Option<u64>,
    /// Set once the job reported its end.
    pub(crate) outcome: Option<std::result::Result<Value, String>>,
}

/// What to download: an asset and the size its release lists.
pub(crate) struct Fetch<'a> {
    pub(crate) url: &'a str,
    pub(crate) file: &'a str,
    pub(crate) size: u64,
}

/// The digest `SHA256SUMS.txt` at `sums_url` lists for `file`. No entry, no install.
pub(crate) fn fetch_checksum(transport: &dyn Transport, sums_url: &str, file: &str, what: &str) -> std::result::Result<Sha256, String> {
    let sums = transport
        .get(&Request { url: sums_url, accept: "text/plain", etag: None, max_bytes: MAX_SUMS_BYTES })
        .map_err(|e| format!("couldn't get {what}'s checksums: {e}"))
        .and_then(|r| match r {
            artcraft_toolbox_net::Response::Ok { body, .. } => String::from_utf8(body).map_err(|_| "SHA256SUMS.txt is not text".to_string()),
            artcraft_toolbox_net::Response::NotModified { .. } => Err("SHA256SUMS.txt: unexpected 304".to_string()),
        })?;
    let sums = Checksums::parse(&sums).map_err(|e| e.to_string())?;
    sums.get(file).ok_or_else(|| format!("SHA256SUMS.txt doesn't list {file}; not installing it"))
}

/// The downloaded `package` must hash to `expected`, or it is deleted.
pub(crate) fn verify_download(package: &Path, expected: Sha256, file: &str) -> std::result::Result<(), String> {
    let actual = artcraft_toolbox_install::sha256_file(package).map_err(|e| e.to_string())?;
    if actual != expected {
        let _ = std::fs::remove_file(package);
        return Err(format!("{file} doesn't match its checksum (expected {expected}, got {actual}); it was deleted"));
    }
    Ok(())
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

fn run_update(s: &mut Session, p: &Value) -> Result<Value> {
    let id = start_update(s, p)?;
    s.wait_job(id)
}

pub(crate) fn refused(m: impl Into<String>) -> EngineError {
    EngineError::Refused(m.into())
}

/// The install or update job running for `app`, if any.
pub(crate) fn busy(s: &Session, app: &App) -> Result<()> {
    if s.job_for(INSTALL, &app.id).is_some() || s.job_for(UPDATE, &app.id).is_some() {
        return Err(refused(format!("{} is being installed or updated", app.name)));
    }
    Ok(())
}

fn start(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["app", "version"])?;
    let app = params::app(s, p)?.clone();
    let wanted = params::version(p)?;
    busy(s, &app)?;
    if let Some(inst) = s.inventory().current(&app.id) {
        return Err(refused(format!("{} {} is already installed; update it instead", app.name, inst.version)));
    }
    let plan = plan(s, &app, wanted.as_ref(), None, Vec::new())?;
    spawn(s, plan, INSTALL)
}

/// `app.update`: the newest version the app's channel and pin offer, or `version` (any release,
/// older ones too). A version that is kept is switched to without a download.
fn start_update(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["app", "version"])?;
    let app = params::app(s, p)?.clone();
    let wanted = params::version(p)?;
    start_update_of(s, &app, wanted)
}

pub(crate) fn start_update_of(s: &mut Session, app: &App, wanted: Option<Version>) -> Result<JobId> {
    busy(s, app)?;
    let current = s.inventory().current(&app.id).cloned().ok_or_else(|| refused(format!("{} isn't installed; install it first", app.name)))?;
    if let Some(v) = &wanted
        && *v == current.version
    {
        return Err(refused(format!("{} {v} is the version in use", app.name)));
    }
    if current.kind == PackageKind::Dmg && artcraft_toolbox_install::is_running(Path::new(&current.path)) {
        return Err(refused(format!("{} is running; quit it to update", app.name)));
    }
    // Versions kept afterwards: the one in use now first, then the most recently installed.
    let keep = usize::try_from(s.settings().keep_previous).unwrap_or(usize::MAX);
    let mut previous: Vec<&Installation> = vec![&current];
    previous.extend(s.inventory().previous(&app.id).into_iter().filter(|i| Some(&i.version) != wanted.as_ref()));
    let prune = previous.iter().skip(keep).map(|i| (i.version.clone(), PathBuf::from(&i.path), i.kind)).collect();
    let replaces = Replaced {
        version: current.version.clone(),
        path: PathBuf::from(&current.path),
        identity: current.trust.as_ref().and_then(Trust::identity).map(str::to_string),
    };
    let plan = plan(s, app, wanted.as_ref(), Some(replaces), prune)?;
    if wanted.is_none() && plan.version <= current.version {
        return Err(refused(format!("{} {} is up to date", app.name, current.version)));
    }
    if s.inventory().get(&app.id, &plan.version).is_some() {
        return Err(refused(format!("{} {} is kept on this computer: switch to it instead (app.rollback)", app.name, plan.version)));
    }
    spawn(s, plan, UPDATE)
}

/// Choose the release and its build for this computer.
fn plan(s: &Session, app: &App, wanted: Option<&Version>, replaces: Option<Replaced>, prune: Vec<(Version, PathBuf, PackageKind)>) -> Result<Plan> {
    if s.layout.is_none() {
        return Err(refused("this session has no install location"));
    }
    let host = s.host().ok_or_else(|| refused("no Crafting App is built for this computer"))?;
    let feed = s.feed(&app.id).ok_or_else(|| refused(format!("no release information for {} yet: check for updates first", app.name)))?;
    let (release, chosen) = match wanted {
        Some(v) => {
            let r = feed.releases.iter().find(|r| &r.version == v).ok_or_else(|| refused(format!("{} has no release {v}", app.name)))?;
            let a = asset::select(&r.assets, host, |a| &a.name).ok_or_else(|| refused(format!("{} {v} has no build for this computer", app.name)))?;
            (r, a)
        }
        None => artcraft_toolbox_feed::latest(&feed.releases, s.settings().channel_for(&app.id), host, s.settings().pinned(&app.id))
            .ok_or_else(|| refused(format!("{} has no release for this computer", app.name)))?,
    };
    if kind_for(host.os) != Some(chosen.name.kind) {
        return Err(refused(format!("installing {:?} packages isn't supported on {}", chosen.name.kind, host.os.label())));
    }
    let verify = verification(&app.name, &release.version, release.checksums_url.as_deref(), chosen)?;
    let icon_png = s.store.as_ref().and_then(|st| st.load_icon(&app.id).ok().flatten()).map(|(png, _)| png);
    Ok(Plan {
        id: app.id.clone(),
        name: app.name.clone(),
        bundle_id: app.bundle_id(),
        tagline: app.tagline.clone(),
        version: release.version.clone(),
        kind: chosen.name.kind,
        url: chosen.url.clone(),
        file: chosen.file.clone(),
        size: chosen.size,
        verify,
        icon_png,
        replaces,
        prune,
    })
}

fn spawn(s: &mut Session, plan: Plan, command: &'static str) -> Result<JobId> {
    let layout = s.layout.clone().ok_or_else(|| refused("this session has no install location"))?;
    let label = format!("{} {}", if command == UPDATE { "Updating to" } else { "Installing" }, plan.what());
    let app_id = plan.id.clone();
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let transport = s.transport.clone().ok_or_else(|| refused("this session has no network access"))?;
    let (worker_cancel, clock) = (Arc::clone(&cancel), s.clock);
    let total = Some(plan.size);
    let spawned = std::thread::Builder::new().name("app-install".into()).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install_worker(&plan, transport.as_ref(), &layout, clock, &worker_cancel, &tx)))
            .unwrap_or_else(|_| Err("internal error (logged)".into()));
        let msg = match outcome {
            Ok(done) => InstallMsg::Installed(Box::new(done)),
            Err(e) => InstallMsg::Failed(e),
        };
        let _ = tx.send(JobMsg::Install(msg));
    });
    if let Err(e) = spawned {
        return Err(EngineError::Job(format!("couldn't start the install: {e}")));
    }
    let id = s.jobs.next_id();
    let state = InstallState { phase: "Starting".into(), done: 0, total, outcome: None };
    s.jobs.push(Running { id, command, label, rx, cancel, items: vec![app_id], in_flight: Default::default(), state: JobState::Install(state) });
    Ok(id)
}

fn send(tx: &Sender<JobMsg>, phase: &'static str, done: u64, total: Option<u64>) {
    let _ = tx.send(JobMsg::Install(InstallMsg::Progress { phase, done, total }));
}

/// Download, verify, place, check, activate, prune (on the worker). Never touches the session.
fn install_worker(
    plan: &Plan,
    transport: &dyn Transport,
    layout: &Layout,
    clock: fn() -> u64,
    cancel: &AtomicBool,
    tx: &Sender<JobMsg>,
) -> std::result::Result<Done, String> {
    let what = plan.what();
    send(tx, "Verifying the release", 0, Some(plan.size));
    let expected = match &plan.verify {
        Verify::Sums(url) => fetch_checksum(transport, url, &plan.file, &what)?,
        Verify::Digest(digest) => *digest,
    };

    let fetch = Fetch { url: &plan.url, file: &plan.file, size: plan.size };
    let package = download(&fetch, transport, &layout.downloads, cancel, &|done| send(tx, "Downloading", done, Some(plan.size)))?;
    send(tx, "Verifying", plan.size, Some(plan.size));
    verify_download(&package, expected, &plan.file)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    send(tx, "Installing", plan.size, Some(plan.size));
    let version = plan.version.to_string();
    let info =
        AppInfo { id: &plan.id, name: &plan.name, bundle_id: &plan.bundle_id, tagline: &plan.tagline, version: &version, icon_png: plan.icon_png.as_deref() };
    let placed = artcraft_toolbox_install::place(layout, &info, plan.kind, &package);
    // The verified package has served its purpose either way.
    let _ = std::fs::remove_file(&package);
    let placed = placed.map_err(|e| e.to_string())?;
    // From here a failure removes the placed version again; the active one is untouched.
    let undo = |e: String| {
        let _ = artcraft_toolbox_install::remove_version(layout, &info, plan.kind, &placed);
        e
    };
    send(tx, "Checking the signature", plan.size, Some(plan.size));
    let trust = artcraft_toolbox_install::verify_signature(plan.kind, &placed).map_err(|e| undo(format!("{what}: {e}")))?;
    if let Some(r) = &plan.replaces
        && let Some(was) = &r.identity
        && trust.identity() != Some(was.as_str())
    {
        let now = trust.identity().map_or("nobody".to_string(), str::to_string);
        return Err(undo(format!("{what} is signed by {now}, not by {was} like {} {}; not updating", plan.name, r.version)));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(undo("cancelled".into()));
    }
    let replaced_version = plan.replaces.as_ref().map(|r| r.version.to_string());
    let current = plan.replaces.as_ref().zip(replaced_version.as_deref()).map(|(r, v)| Current { path: &r.path, version: v });
    let activated = artcraft_toolbox_install::activate(layout, &info, plan.kind, &placed, current).map_err(|e| undo(e.to_string()))?;

    // Versions beyond `keepPrevious`: best effort, the update stands either way.
    let (mut pruned, mut warnings) = (Vec::new(), Vec::new());
    for (v, path, kind) in &plan.prune {
        let path = match (&plan.replaces, &activated.previous) {
            (Some(r), Some(moved)) if &r.version == v => moved.clone(),
            _ => path.clone(),
        };
        let vs = v.to_string();
        match artcraft_toolbox_install::remove_version(layout, &AppInfo { version: &vs, ..info }, *kind, &path) {
            Ok(()) => pruned.push(v.clone()),
            Err(e) => warnings.push(format!("{} {v} was kept: {e}", plan.name)),
        }
    }
    let utf8 = |p: &Path| p.to_str().map(str::to_string).ok_or("the install location is not a UTF-8 path");
    let path = utf8(&activated.active)?;
    let moved = match (&plan.replaces, &activated.previous) {
        (Some(r), Some(p)) if !pruned.contains(&r.version) => Some((r.version.clone(), utf8(p)?)),
        _ => None,
    };
    let inst =
        Installation { app: plan.id.clone(), version: plan.version.clone(), kind: plan.kind, path, installed_at: clock(), active: true, trust: Some(trust) };
    Ok(Done { inst, moved, pruned, warnings })
}

/// Stream the asset into `<dir>/<file>.part`, resuming what an earlier attempt left, and rename
/// it to `<file>` once it has exactly the listed size. `progress` hears the bytes so far.
pub(crate) fn download(
    plan: &Fetch,
    transport: &dyn Transport,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> std::result::Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", plan.file));
    let done = dir.join(plan.file);
    let _ = std::fs::remove_file(&done);
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > plan.size {
        have = 0;
    }
    for _ in 0..2 {
        let d = match transport.download(plan.url, have) {
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
        progress(have);
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
                progress(have);
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
        InstallMsg::Installed(done) => {
            let Done { inst, moved, pruned, warnings } = *done;
            for w in &warnings {
                log::warn!("{w}");
            }
            let summary = json!({
                "app": inst.app, "version": inst.version, "path": inst.path, "trust": inst.trust,
                "removed": pruned, "warnings": warnings,
            });
            let app = inst.app.clone();
            let recorded = (|| {
                s.inventory.record(inst)?;
                if let Some((v, path)) = moved {
                    s.inventory.set_path(&app, &v, path)?;
                }
                for v in &pruned {
                    s.inventory.remove(&app, v);
                }
                Ok::<(), artcraft_toolbox_model::Error>(())
            })()
            .map_err(EngineError::from)
            .and_then(|()| s.save_inventory());
            s.found.remove(&app);
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

/// Remove the active version, then every kept one.
fn uninstall(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?.clone();
    busy(s, &app)?;
    let inst = s.inventory().current(&app.id).cloned().ok_or_else(|| refused(format!("{} isn't installed", app.name)))?;
    let layout = s.layout.clone().ok_or_else(|| refused("this session has no install location"))?;
    let (version, bundle_id) = (inst.version.to_string(), app.bundle_id());
    artcraft_toolbox_install::uninstall(&layout, &info(&app, &bundle_id, &version, None), inst.kind, Path::new(&inst.path))?;
    s.inventory.remove(&app.id, &inst.version);
    let mut kept_too = Vec::new();
    let mut warnings = Vec::new();
    for old in s.inventory().previous(&app.id).into_iter().cloned().collect::<Vec<_>>() {
        let v = old.version.to_string();
        match artcraft_toolbox_install::remove_version(&layout, &info(&app, &bundle_id, &v, None), old.kind, Path::new(&old.path)) {
            Ok(()) => {
                s.inventory.remove(&app.id, &old.version);
                kept_too.push(old.version);
            }
            Err(e) => warnings.push(format!("{} {v} was kept: {e}", app.name)),
        }
    }
    s.save_inventory()?;
    s.rescan();
    Ok(json!({"app": app.id, "version": inst.version, "removed": inst.path, "removedVersions": kept_too, "warnings": warnings}))
}

fn launch(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?;
    let inst = s.inventory().current(&app.id).ok_or_else(|| refused(format!("{} isn't installed", echo(&app.name))))?;
    if !inst.active {
        return Err(refused(format!("no version of {} is active", echo(&app.name))));
    }
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
            kept: root.join("data/versions.noindex"),
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

    // Updates, rollback, pruning, adoption: a GitHub with PhotoCraft 0.4.0 and 0.5.0.

    fn release_url(v: &str, file: &str) -> String {
        format!("https://github.com/storytold/photocraft/releases/download/v{v}/{file}")
    }

    fn appimage_of(v: &str) -> Vec<u8> {
        let mut b = appimage();
        b.extend(v.as_bytes());
        b
    }

    /// PhotoCraft 0.4.0 and 0.5.0 for Linux, each with its checksums.
    struct Two;

    impl Transport for Two {
        fn get(&self, req: &Request<'_>) -> std::result::Result<Response, NetError> {
            let v = if req.url.contains("/v0.4.0/") { "0.4.0" } else { "0.5.0" };
            let body = format!("{}  photocraft-{v}-linux-x86_64.AppImage\n", sha(&appimage_of(v)));
            Ok(Response::Ok { body: body.into_bytes(), etag: None, rate: RateLimit::default() })
        }
        fn download(&self, url: &str, from: u64) -> std::result::Result<Download, NetError> {
            let v = if url.contains("/v0.4.0/") { "0.4.0" } else { "0.5.0" };
            let body = appimage_of(v);
            let rest = body.get(from as usize..).unwrap_or_default().to_vec();
            Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(body.len() as u64) })
        }
    }

    fn two() -> (PathBuf, Session) {
        let root = temp("update");
        let store = Store::open(root.join("data")).unwrap();
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(store), Some(Arc::new(Two)));
        s.set_host(Target::from_consts("linux", "x86_64"));
        s.set_layout(Some(Layout {
            apps: root.join("apps"),
            desktop_entries: Some(root.join("share/applications")),
            icons: Some(root.join("share/icons")),
            start_menu: None,
            downloads: root.join("data/downloads"),
            kept: root.join("data/versions.noindex"),
        }));
        let rel = |v: &str| {
            let file = format!("photocraft-{v}-linux-x86_64.AppImage");
            json!({"tag_name": format!("v{v}"), "assets": [
                {"name": file, "size": appimage_of(v).len(), "browser_download_url": release_url(v, &file)},
                {"name": "SHA256SUMS.txt", "size": 100, "browser_download_url": release_url(v, "SHA256SUMS.txt")},
            ]})
        };
        s.ingest_releases("photocraft", &json!([rel("0.5.0"), rel("0.4.0")]).to_string(), 1).unwrap();
        (root, s)
    }

    fn path_of(root: &Path, v: &str) -> PathBuf {
        root.join("apps").join("photocraft").join(v).join("photocraft.AppImage")
    }

    fn entry(root: &Path) -> String {
        std::fs::read_to_string(root.join("share/applications/ai.storyteller.photocraft.desktop")).unwrap_or_default()
    }

    fn versions_of(s: &Session) -> Vec<(String, bool)> {
        s.inventory().versions("photocraft").iter().map(|i| (i.version.to_string(), i.active)).collect()
    }

    #[test]
    fn update_keeps_the_previous_version_and_rollback_switches_back() {
        use crate::versions_cmds::{ROLLBACK, VERSIONS};
        let (root, mut s) = two();
        s.execute(INSTALL, json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
        assert!(matches!(s.app_status("photocraft").unwrap().status, crate::Status::UpdateAvailable { .. }));
        let out = s.execute(UPDATE, json!({"app": "photocraft"})).unwrap();
        assert_eq!(out["version"], "0.5.0");
        assert_eq!(out["trust"]["state"], "unsigned", "AppImages carry no platform signature");
        assert_eq!(versions_of(&s), [("0.5.0".to_string(), true), ("0.4.0".to_string(), false)]);
        assert!(path_of(&root, "0.4.0").is_file() && path_of(&root, "0.5.0").is_file());
        assert!(entry(&root).contains("X-ArtCraft-Toolbox-Version=0.5.0"));
        assert!(s.execute(UPDATE, json!({"app": "photocraft"})).unwrap_err().to_string().contains("up to date"));

        // Back to the kept version, no download; then nothing older is kept.
        let out = s.execute(ROLLBACK, json!({"app": "photocraft"})).unwrap();
        assert_eq!((out["version"].as_str(), out["previous"].as_str()), (Some("0.4.0"), Some("0.5.0")));
        assert_eq!(versions_of(&s), [("0.5.0".to_string(), false), ("0.4.0".to_string(), true)]);
        assert!(entry(&root).contains("X-ArtCraft-Toolbox-Version=0.4.0"));
        assert!(s.execute(ROLLBACK, json!({"app": "photocraft"})).unwrap_err().to_string().contains("no earlier version"));
        // The newer one is kept: updating switches to it rather than downloading it again.
        assert!(s.execute(UPDATE, json!({"app": "photocraft"})).unwrap_err().to_string().contains("switch to it"));
        s.execute(ROLLBACK, json!({"app": "photocraft", "version": "0.5.0"})).unwrap();
        assert_eq!(s.inventory().current("photocraft").unwrap().version.to_string(), "0.5.0");
        for (p, want) in [
            (json!({"app": "photocraft", "version": "0.5.0"}), "already the version in use"),
            (json!({"app": "photocraft", "version": "9.0.0"}), "isn't kept"),
            (json!({"app": "photocraft", "version": 4}), "must be a string"),
        ] {
            let e = s.execute(ROLLBACK, p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        let listed = s.execute(VERSIONS, json!({"app": "photocraft"})).unwrap();
        assert_eq!(listed["installed"].as_array().map(Vec::len), Some(2));
        // Saved: a new session sees the same.
        let saved = Store::open(root.join("data")).unwrap().load_inventory().unwrap().unwrap();
        assert_eq!(saved.current("photocraft").unwrap().version.to_string(), "0.5.0");

        // Uninstall removes every version.
        let out = s.execute(UNINSTALL, json!({"app": "photocraft"})).unwrap();
        assert_eq!(out["removedVersions"], json!(["0.4.0"]));
        assert!(!root.join("apps/photocraft").exists() && s.inventory().versions("photocraft").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keep_previous_zero_removes_the_replaced_version() {
        let (root, mut s) = two();
        s.execute("settings.set", json!({"keepPrevious": 0})).unwrap();
        s.execute(INSTALL, json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
        let out = s.execute(UPDATE, json!({"app": "photocraft"})).unwrap();
        assert_eq!(out["removed"], json!(["0.4.0"]));
        assert_eq!(versions_of(&s), [("0.5.0".to_string(), true)]);
        assert!(!path_of(&root, "0.4.0").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_update_from_another_developer_is_refused_and_changes_nothing() {
        let (root, mut s) = two();
        s.execute(INSTALL, json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
        // As if 0.4.0 had been signed: 0.5.0 (unsigned here) is from somebody else.
        let mut inv = s.inventory().clone();
        let mut inst = inv.current("photocraft").unwrap().clone();
        inst.trust = Some(Trust::Signed {
            signer: "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)".into(),
            team: Some("DJ6XS33FX8".into()),
            notarized: true,
        });
        inv.record(inst).unwrap();
        s.set_inventory(inv);
        let e = s.execute(UPDATE, json!({"app": "photocraft"})).unwrap_err().to_string();
        assert!(e.contains("signed by nobody, not by DJ6XS33FX8"), "{e}");
        assert_eq!(versions_of(&s), [("0.4.0".to_string(), true)]);
        assert!(!path_of(&root, "0.5.0").exists(), "the placed version is removed again");
        assert!(entry(&root).contains("X-ArtCraft-Toolbox-Version=0.4.0"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_app_installed_by_hand_is_found_and_adopted() {
        use crate::versions_cmds::{ADOPT, RESCAN};
        let (root, mut s) = two();
        assert!(s.execute(ADOPT, json!({"app": "photocraft"})).unwrap_err().to_string().contains("no copy"));
        let by_hand = path_of(&root, "0.3.0");
        std::fs::create_dir_all(by_hand.parent().unwrap()).unwrap();
        std::fs::write(&by_hand, appimage()).unwrap();
        let out = s.execute(RESCAN, json!({})).unwrap();
        assert_eq!(out["foundOutsideToolbox"]["photocraft"], json!(["0.3.0"]));
        assert_eq!(s.app_status("photocraft").unwrap().found, Some(Version::new(0, 3, 0)));
        let out = s.execute(ADOPT, json!({"app": "photocraft"})).unwrap();
        assert_eq!(out["adopted"], json!(["0.3.0"]));
        assert_eq!(versions_of(&s), [("0.3.0".to_string(), true)]);
        assert_eq!(s.app_status("photocraft").unwrap().found, None);
        assert!(s.execute(ADOPT, json!({"app": "photocraft"})).unwrap_err().to_string().contains("already managed"));
        // From now on it updates like any other.
        s.execute(UPDATE, json!({"app": "photocraft"})).unwrap();
        assert_eq!(s.inventory().current("photocraft").unwrap().version.to_string(), "0.5.0");

        // A record whose files were deleted is forgotten on the next rescan.
        std::fs::remove_file(path_of(&root, "0.3.0")).unwrap();
        let out = s.execute(RESCAN, json!({})).unwrap();
        assert!(out["changes"][0].as_str().unwrap().contains("0.3.0 is no longer"), "{out}");
        assert_eq!(versions_of(&s), [("0.5.0".to_string(), true)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn update_all_starts_what_is_due_and_switches_to_kept_versions() {
        use crate::versions_cmds::{ROLLBACK, UPDATE_ALL};
        let (root, mut s) = two();
        assert_eq!(s.execute(UPDATE_ALL, json!({})).unwrap()["started"], json!([]));
        s.execute(INSTALL, json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
        // Only the apps set to update automatically, when asked for those.
        assert_eq!(s.execute(UPDATE_ALL, json!({"onlyAutomatic": true})).unwrap()["started"], json!([]));
        let out = s.execute(UPDATE_ALL, json!({})).unwrap();
        let job = out["started"][0]["job"].as_u64().map(crate::JobId).unwrap();
        s.wait_job(job).unwrap();
        assert_eq!(s.inventory().current("photocraft").unwrap().version.to_string(), "0.5.0");
        // After a rollback, the newer version is kept: switched to without a download.
        s.execute(ROLLBACK, json!({"app": "photocraft"})).unwrap();
        let out = s.execute(UPDATE_ALL, json!({})).unwrap();
        assert_eq!(out["switched"], json!([{"app": "photocraft", "version": "0.5.0"}]));
        assert_eq!(s.inventory().current("photocraft").unwrap().version.to_string(), "0.5.0");
        assert!(s.execute(UPDATE_ALL, json!({"onlyAutomatic": "yes"})).unwrap_err().to_string().contains("true or false"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn update_needs_an_installed_app() {
        let (root, mut s) = two();
        assert!(s.execute(UPDATE, json!({"app": "photocraft"})).unwrap_err().to_string().contains("isn't installed"));
        s.execute(INSTALL, json!({"app": "photocraft"})).unwrap();
        let e = s.execute(INSTALL, json!({"app": "photocraft"})).unwrap_err().to_string();
        assert!(e.contains("update it instead"), "{e}");
        assert!(s.execute(UPDATE, json!({"app": "photocraft", "version": "0.5.0"})).unwrap_err().to_string().contains("version in use"));
        // An explicit older version is a downgrade the user asked for.
        s.execute(UPDATE, json!({"app": "photocraft", "version": "0.4.0"})).unwrap();
        assert_eq!(versions_of(&s), [("0.5.0".to_string(), false), ("0.4.0".to_string(), true)]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
