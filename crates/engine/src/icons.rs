//! App icons: each craft's own 128 px PNG (`App::icon_url`, docs/release-contract.md › Icons),
//! fetched from `raw.githubusercontent.com` (not the rate-limited API), cached in the data folder
//! and decoded to RGBA on the worker, so the UI only uploads a texture.
//!
//! `icons.refresh` fetches the icons that are missing or older than [`MAX_AGE_SECS`], with the
//! cached ETag. An icon that can't be fetched or decoded leaves the monogram tile in place.

use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use artcraft_toolbox_net::{NetError, Request, Response, Transport};
use artcraft_toolbox_store::{IconMeta, MAX_ICON_BYTES, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::CommandSpec;
use crate::jobs::{JobMsg, JobState, PoolTask, Running, spawn_pool};
use crate::update_cmds::Failure;
use crate::{EngineError, JobId, Result, Session, params};

pub const REFRESH_ICONS: &str = "icons.refresh";
/// Icons are checked again after a week.
pub const MAX_AGE_SECS: u64 = 7 * 86_400;
/// After a refresh, the next automatic one waits at least this long (offline machines don't retry
/// every minute).
pub const RETRY_SECS: u64 = 3600;
/// Largest icon side accepted.
pub const MAX_SIDE: u32 = 1024;
const WORKERS: usize = 4;

/// A decoded icon, ready for any toolkit: 8-bit RGBA, unpremultiplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// Changes whenever the image does, so a UI knows to rebuild its texture.
    pub revision: u64,
}

/// An app's icon as the session holds it.
#[derive(Debug, Clone)]
pub struct Icon {
    pub image: IconImage,
    pub fetched_at: u64,
    pub etag: Option<String>,
}

/// Decode a PNG to RGBA, refusing anything that isn't a sane icon (wrong format, larger than
/// [`MAX_SIDE`] a side or [`MAX_ICON_BYTES`] on disk). Never panics.
pub fn decode_png(bytes: &[u8]) -> std::result::Result<(u32, u32, Vec<u8>), String> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err(format!("the icon is larger than {MAX_ICON_BYTES} bytes"));
    }
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 16 * 1024 * 1024 });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG icon ({e})"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("the icon is {width}×{height}; at most {MAX_SIDE}×{MAX_SIDE} is accepted"));
    }
    let size = reader.output_buffer_size().ok_or("the icon is too large to decode")?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(|e| format!("the icon is damaged ({e})"))?;
    let data = buf.get(..frame.buffer_size()).ok_or("the icon is damaged")?;
    let pixels = (width as usize).saturating_mul(height as usize);
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| match p {
                [r, g, b] => [*r, *g, *b, 255],
                _ => [0; 4],
            })
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| match p {
                [g, a] => [*g, *g, *g, *a],
                _ => [0; 4],
            })
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("the icon's palette wasn't expanded".into()),
    };
    if rgba.len() != pixels.saturating_mul(4) {
        return Err("the icon is damaged".into());
    }
    Ok((width, height, rgba))
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec { id: REFRESH_ICONS, label: "Refresh App Icons", params: r#"{"force"?:bool}"#, enabled: can_refresh, run, start: Some(start) }]
}

fn can_refresh(s: &Session) -> std::result::Result<(), String> {
    if !s.online() {
        return Err("this session has no network access".into());
    }
    if s.jobs.runs(REFRESH_ICONS) {
        return Err("the icons are already being refreshed".into());
    }
    Ok(())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = start(s, p)?;
    s.wait_job(id)
}

/// What an icon refresh did. Also the command's JSON result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSummary {
    pub fetched: Vec<String>,
    pub unchanged: Vec<String>,
    /// Fetched less than a week ago: not requested.
    pub fresh: Vec<String>,
    pub failed: Vec<Failure>,
}

impl IconSummary {
    pub fn done(&self) -> usize {
        self.fetched.len() + self.unchanged.len() + self.fresh.len() + self.failed.len()
    }
}

pub(crate) enum IconMsg {
    Fetched { app: String, width: u32, height: u32, rgba: Vec<u8>, etag: Option<String>, at: u64 },
    Unchanged { app: String, at: u64 },
    Failed { app: String, error: String },
}

struct Task {
    app: String,
    url: String,
    etag: Option<String>,
}

impl PoolTask for Task {
    fn app(&self) -> &str {
        &self.app
    }
    fn panicked(&self) -> JobMsg {
        JobMsg::Icon(IconMsg::Failed { app: self.app.clone(), error: "internal error (logged)".into() })
    }
}

fn start(s: &mut Session, p: &Value) -> Result<JobId> {
    params::only(p, &["force"])?;
    let force = match params::object(p)?.get("force") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(EngineError::BadParams(format!("`force` must be true or false, got {}", params::kind(other)))),
    };
    let transport =
        s.transport.clone().ok_or_else(|| EngineError::Disabled { id: REFRESH_ICONS.into(), reason: "this session has no network access".into() })?;
    let now = s.now();
    s.icons_attempted_at = Some(now);
    let mut summary = IconSummary::default();
    let mut tasks = Vec::new();
    let items: Vec<String> = s.catalog().apps.iter().map(|a| a.id.clone()).collect();
    for app in &s.catalog().apps {
        let icon = s.icons.get(&app.id);
        if !force && icon.is_some_and(|i| now.saturating_sub(i.fetched_at) < MAX_AGE_SECS) {
            summary.fresh.push(app.id.clone());
            continue;
        }
        tasks.push(Task { app: app.id.clone(), url: app.icon_url(), etag: icon.and_then(|i| i.etag.clone()) });
    }
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (store, clock) = (s.store.clone(), s.clock);
    spawn_pool("icons-refresh", tasks, WORKERS, Arc::clone(&cancel), tx, move |task: &Task| {
        vec![JobMsg::Icon(fetch_one(task, transport.as_ref(), store.as_ref(), clock))]
    });
    let id = s.jobs.next_id();
    s.jobs.push(Running {
        id,
        command: REFRESH_ICONS,
        label: "Refreshing app icons".into(),
        rx,
        cancel,
        items,
        in_flight: BTreeSet::new(),
        state: JobState::Icons(summary),
    });
    Ok(id)
}

/// Fetch, validate, cache and decode one icon (on a worker).
fn fetch_one(task: &Task, transport: &dyn Transport, store: Option<&Store>, clock: fn() -> u64) -> IconMsg {
    let req = Request { url: &task.url, accept: "image/png", etag: task.etag.as_deref(), max_bytes: MAX_ICON_BYTES };
    let result = transport.get(&req);
    let (now, app) = (clock(), task.app.clone());
    match result {
        Ok(Response::NotModified { .. }) => {
            if let Some(store) = store
                && let Err(e) = store.touch_icon(&app, now)
            {
                log::warn!("{e}");
            }
            IconMsg::Unchanged { app, at: now }
        }
        Ok(Response::Ok { body, etag, .. }) => match decode_png(&body) {
            Err(error) => IconMsg::Failed { app, error },
            Ok((width, height, rgba)) => {
                if let Some(store) = store
                    && let Err(e) = store.save_icon(&app, &body, &IconMeta { etag: etag.clone(), fetched_at: now })
                {
                    log::warn!("{e}");
                }
                IconMsg::Fetched { app, width, height, rgba, etag, at: now }
            }
        },
        Err(NetError::Status { status: 404, .. }) => IconMsg::Failed { app, error: "the app has no icon at the usual path".into() },
        Err(e) => IconMsg::Failed { app, error: e.to_string() },
    }
}

pub(crate) fn apply(s: &mut Session, job: &mut Running, msg: IconMsg) {
    let JobState::Icons(summary) = &mut job.state else { return };
    match msg {
        IconMsg::Fetched { app, width, height, rgba, etag, at } => {
            job.in_flight.remove(&app);
            let revision = s.next_icon_revision();
            s.icons.insert(app.clone(), Icon { image: IconImage { width, height, rgba: rgba.into(), revision }, fetched_at: at, etag });
            summary.fetched.push(app);
        }
        IconMsg::Unchanged { app, at } => {
            job.in_flight.remove(&app);
            if let Some(i) = s.icons.get_mut(&app) {
                i.fetched_at = at;
            }
            summary.unchanged.push(app);
        }
        IconMsg::Failed { app, error } => {
            job.in_flight.remove(&app);
            log::warn!("icon of {app}: {error}");
            summary.failed.push(Failure { app, error });
        }
    }
}

pub(crate) fn finish(job: Running) -> Value {
    match job.state {
        JobState::Icons(summary) => serde_json::to_value(&summary).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

impl Session {
    /// `app`'s icon, once one has been fetched or loaded from the cache.
    pub fn icon(&self, app: &str) -> Option<&IconImage> {
        self.icons.get(app).map(|i| &i.image)
    }

    /// Should an icon refresh start by itself? When online, none runs, the last one started more
    /// than [`RETRY_SECS`] ago, and some icon is missing or older than [`MAX_AGE_SECS`].
    pub fn icons_due(&self) -> bool {
        let now = self.now();
        if !self.online() || self.jobs.runs(REFRESH_ICONS) || self.icons_attempted_at.is_some_and(|t| now.saturating_sub(t) < RETRY_SECS) {
            return false;
        }
        self.catalog.apps.iter().any(|a| self.icons.get(&a.id).is_none_or(|i| now.saturating_sub(i.fetched_at) >= MAX_AGE_SECS))
    }

    pub(crate) fn next_icon_revision(&mut self) -> u64 {
        self.icon_revision = self.icon_revision.saturating_add(1);
        self.icon_revision
    }

    /// Load the cached icons (at open).
    pub(crate) fn load_cached_icons(&mut self, store: &Store) {
        let ids: Vec<String> = self.catalog.apps.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            match store.load_icon(&id) {
                Ok(Some((png, meta))) => match decode_png(&png) {
                    Ok((width, height, rgba)) => {
                        let revision = self.next_icon_revision();
                        self.icons
                            .insert(id, Icon { image: IconImage { width, height, rgba: rgba.into(), revision }, fetched_at: meta.fetched_at, etag: meta.etag });
                    }
                    Err(e) => log::warn!("cached icon of {id}: {e}; it will be fetched again"),
                },
                Ok(None) => {}
                Err(e) => log::warn!("{e}; it will be fetched again"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;
    use artcraft_toolbox_catalog::Catalog;
    use artcraft_toolbox_net::RateLimit;
    use serde_json::json;
    use std::sync::Mutex;

    /// A valid 2×1 RGB PNG, written with the `png` crate's encoder.
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        out
    }

    #[test]
    fn decodes_icons_and_refuses_everything_else() {
        let (w, h, rgba) = decode_png(&tiny_png()).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        for bad in [Vec::new(), b"not a png".to_vec(), tiny_png()[..20].to_vec(), vec![0u8; MAX_ICON_BYTES + 1]] {
            assert!(decode_png(&bad).is_err());
        }
        // A header claiming 100000×1.
        let mut huge = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut huge, 100_000, 1);
            enc.set_color(png::ColorType::Grayscale);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&vec![0u8; 100_000]).unwrap();
        }
        assert!(decode_png(&huge).unwrap_err().contains("at most"));
    }

    struct Fake {
        png: Vec<u8>,
        seen: Mutex<Vec<(String, Option<String>)>>,
    }

    impl Transport for Fake {
        fn get(&self, req: &Request<'_>) -> std::result::Result<Response, NetError> {
            self.seen.lock().unwrap().push((req.url.to_string(), req.etag.map(str::to_string)));
            if req.url.contains("/wordcraft/") {
                return Err(NetError::Status { status: 404, message: "Not Found".into() });
            }
            if req.url.contains("/gridcraft/") {
                return Ok(Response::Ok { body: b"<html>".to_vec(), etag: None, rate: RateLimit::default() });
            }
            if req.etag.is_some() {
                return Ok(Response::NotModified { rate: RateLimit::default() });
            }
            Ok(Response::Ok { body: self.png.clone(), etag: Some("\"i\"".into()), rate: RateLimit::default() })
        }
    }

    fn t0() -> u64 {
        2_000_000
    }
    fn later() -> u64 {
        2_000_000 + MAX_AGE_SECS + RETRY_SECS
    }

    #[test]
    fn refresh_fetches_caches_and_reuses_icons() {
        let root = temp("icons");
        let store = Store::open(&root).unwrap();
        let fake = Arc::new(Fake { png: tiny_png(), seen: Mutex::new(Vec::new()) });
        let (mut s, _) = Session::open(Catalog::builtin().unwrap(), Some(store.clone()), Some(fake.clone()));
        s.set_clock(t0);
        assert!(s.icons_due());
        let r: IconSummary = serde_json::from_value(s.execute(REFRESH_ICONS, json!({})).unwrap()).unwrap();
        assert_eq!((r.fetched.len(), r.failed.len()), (10, 2), "{r:?}");
        assert!(r.failed.iter().any(|f| f.app == "wordcraft" && f.error.contains("no icon")));
        assert!(r.failed.iter().any(|f| f.app == "gridcraft" && f.error.contains("not a PNG")));
        let icon = s.icon("photocraft").unwrap().clone();
        assert_eq!((icon.width, icon.height), (2, 1));
        assert!(fake.seen.lock().unwrap().iter().all(|(url, _)| url.starts_with("https://raw.githubusercontent.com/storytold/")));
        // Not due again right away (failures included: they wait for RETRY_SECS).
        assert!(!s.icons_due());

        // A new session loads them from the cache, with their ETags...
        let (mut s2, _) = Session::open(Catalog::builtin().unwrap(), Some(store), Some(fake.clone()));
        s2.set_clock(later);
        assert_eq!(s2.icon("photocraft").map(|i| (i.width, i.height)), Some((2, 1)));
        // ...and a week later asks with them: 304s keep the images.
        fake.seen.lock().unwrap().clear();
        let r: IconSummary = serde_json::from_value(s2.execute(REFRESH_ICONS, json!({})).unwrap()).unwrap();
        assert_eq!(r.unchanged.len(), 10, "{r:?}");
        assert!(fake.seen.lock().unwrap().iter().filter(|(u, _)| u.contains("/photocraft/")).all(|(_, e)| e.as_deref() == Some("\"i\"")));
        assert!(s2.icon("photocraft").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn offline_sessions_never_refresh() {
        let mut s = Session::new().unwrap();
        assert!(!s.icons_due());
        assert!(s.execute(REFRESH_ICONS, json!({})).unwrap_err().to_string().contains("no network access"));
    }
}
