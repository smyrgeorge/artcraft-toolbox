//! M7: the publisher's signed catalog and aggregated feed (docs/architecture.md § 12), against a
//! fake GitHub keyed by URL and a test signing key. One check costs two requests instead of one
//! per app, a newer catalog adds apps without a toolbox release, bad or foreign documents fall
//! back to the API and keep the built-in catalog, and an app outside the contract installs from
//! the feed's digest and never without one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use artcraft_toolbox_engine::net::{Download, NetError, RateLimit, Request, Response, Transport};
use artcraft_toolbox_engine::update_cmds::{CHECK, CheckSummary, RemoteState};
use artcraft_toolbox_engine::{Catalog, CatalogSource, Layout, Session, Status, Target, Version};
use artcraft_toolbox_feed::aggregate;
use artcraft_toolbox_release::signing::{Kind, SecretKey, sign};
use artcraft_toolbox_store::Store;
use serde_json::json;
use sha2::Digest;

const PHOTOCRAFT: &str = include_str!("../../feed/tests/fixtures/photocraft-releases.json");
const ARTCRAFT: &str = include_str!("../../feed/tests/fixtures/artcraft-releases.json");
const CATALOG_URL: &str = "https://raw.githubusercontent.com/smyrgeorge/artcraft-toolbox/feed/artcraft-catalog.json";
const FEED_URL: &str = "https://raw.githubusercontent.com/smyrgeorge/artcraft-toolbox/feed/artcraft-feed.json";
const ASSET_URL: &str = "https://github.com/o/patterncraft/releases/download/v9.0.0/PatternCraft_9.0.0_amd64.AppImage";

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-remote-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn publisher() -> SecretKey {
    SecretKey::from_seed([42; 32])
}

fn appimage() -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend(std::iter::repeat_n(9u8, 20_000));
    b
}

fn digest(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The built-in catalog with the test publisher's key, as TOML text and parsed.
fn builtin_text() -> String {
    artcraft_toolbox_catalog::BUILTIN
        .lines()
        .map(|l| if l.starts_with("public_key = ") { format!("public_key = \"{}\"", publisher().public()) } else { l.to_string() })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Revision `revision` of the catalog: the built-in apps plus `testcraft` (a contract app) and
/// `patterncraft` (outside the contract, a Linux build matched by a pattern).
fn remote_text(revision: u64) -> String {
    builtin_text().replace("revision = 1\n", &format!("revision = {revision}\n"))
        + "\n[[app]]\nid = \"testcraft\"\nname = \"TestCraft\"\ntagline = \"Added by the remote catalog\"\nrepo = \"storytold/testcraft\"\n"
        + "\n[[app]]\nid = \"patterncraft\"\nname = \"PatternCraft\"\ntagline = \"Outside the contract\"\nrepo = \"o/patterncraft\"\n[app.assets]\n\"linux-x86_64\" = \"PatternCraft_{version}_amd64.AppImage\"\n"
}

/// The aggregated feed: PhotoCraft's and ArtCraft's real feeds, one PatternCraft release (with
/// its digest unless `digest` is false), nothing for the rest.
fn feed_text(catalog: &Catalog, with_digest: bool) -> String {
    let mut apps = BTreeMap::new();
    for app in catalog.apps.iter().chain(&catalog.toolbox) {
        let list = match app.id.as_str() {
            "photocraft" => serde_json::from_str(PHOTOCRAFT).unwrap(),
            "artcraft" => serde_json::from_str(ARTCRAFT).unwrap(),
            "patterncraft" => {
                let mut asset = json!({"name": "PatternCraft_9.0.0_amd64.AppImage", "size": appimage().len(), "browser_download_url": ASSET_URL});
                if with_digest {
                    asset["sha256"] = json!(digest(&appimage()));
                }
                json!([{"tag_name": "v9.0.0", "draft": false, "prerelease": false, "assets": [asset]}])
            }
            _ => json!([]),
        };
        apps.insert(app.id.clone(), list);
    }
    aggregate::build(1_700_000_000, apps)
}

#[derive(Clone)]
enum Answer {
    Ok { body: String, etag: Option<String> },
    Status(u16),
}

/// A GitHub keyed by URL. A request carrying the answer's ETag gets a 304. Unknown API feed
/// URLs answer an empty release list; anything else is a 404.
struct Fake {
    answers: Mutex<BTreeMap<String, Answer>>,
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake { answers: Mutex::new(BTreeMap::new()), seen: Mutex::new(Vec::new()) })
    }
    fn answer(&self, url: &str, body: String, etag: &str) {
        self.answers.lock().unwrap().insert(url.into(), Answer::Ok { body, etag: Some(etag.into()) });
    }
    fn status(&self, url: &str, status: u16) {
        self.answers.lock().unwrap().insert(url.into(), Answer::Status(status));
    }
    fn requests(&self) -> Vec<(String, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
    fn api_requests(&self) -> usize {
        self.requests().iter().filter(|(u, _)| u.starts_with("https://api.github.com/")).count()
    }
    fn raw_requests(&self) -> usize {
        self.requests().iter().filter(|(u, _)| u.starts_with("https://raw.githubusercontent.com/")).count()
    }
    fn reset(&self) {
        self.seen.lock().unwrap().clear();
    }
}

impl Transport for Fake {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        self.seen.lock().unwrap().push((req.url.to_string(), req.etag.map(str::to_string)));
        match self.answers.lock().unwrap().get(req.url).cloned() {
            Some(Answer::Ok { etag, .. }) if etag.is_some() && req.etag == etag.as_deref() => Ok(Response::NotModified { rate: RateLimit::default() }),
            Some(Answer::Ok { body, etag }) => Ok(Response::Ok { body: body.into_bytes(), etag, rate: RateLimit::default() }),
            Some(Answer::Status(status)) => Err(NetError::Status { status, message: "nope".into() }),
            None if req.url.contains("/releases?per_page=") => Ok(Response::Ok { body: b"[]".to_vec(), etag: None, rate: RateLimit::default() }),
            None => Err(NetError::Status { status: 404, message: "not found".into() }),
        }
    }
    fn download(&self, url: &str, from: u64) -> Result<Download, NetError> {
        assert_eq!(url, ASSET_URL);
        let rest = appimage().get(from as usize..).unwrap_or_default().to_vec();
        Ok(Download { reader: Box::new(std::io::Cursor::new(rest)), offset: from, total: Some(appimage().len() as u64) })
    }
}

fn t0() -> u64 {
    1_000_000
}
fn t1() -> u64 {
    1_000_000 + 3600
}

fn session(fake: &Arc<Fake>, store: Option<Store>) -> Session {
    let (mut s, warnings) = Session::open(Catalog::parse(&builtin_text()).unwrap(), store, Some(fake.clone()));
    assert!(warnings.is_empty(), "{warnings:?}");
    s.set_clock(t0);
    s.set_host(Target::from_consts("linux", "x86_64"));
    s
}

fn check(s: &mut Session) -> CheckSummary {
    serde_json::from_value(s.execute(CHECK, json!({})).unwrap()).unwrap()
}

/// The publisher's documents as the fake serves them: catalog revision 2 and a feed for it.
fn publish(fake: &Fake, catalog_etag: &str, feed_etag: &str, with_digest: bool) -> Catalog {
    let catalog = Catalog::parse(&remote_text(2)).unwrap();
    fake.answer(CATALOG_URL, sign(&publisher(), Kind::Catalog, &remote_text(2)), catalog_etag);
    fake.answer(FEED_URL, sign(&publisher(), Kind::Feed, &feed_text(&catalog, with_digest)), feed_etag);
    catalog
}

#[test]
fn one_check_costs_two_requests_and_a_newer_catalog_adds_apps() {
    let root = temp("two");
    let store = Store::open(&root).unwrap();
    let fake = Fake::new();
    publish(&fake, "W/\"c2\"", "W/\"f1\"", true);
    let mut s = session(&fake, Some(store.clone()));
    assert_eq!(s.catalog().apps.len(), 13);
    assert_eq!(s.catalog_source(), CatalogSource::Builtin);
    let summary = check(&mut s);
    assert_eq!(summary.catalog, RemoteState::Applied { revision: 2 }, "{summary:?}");
    assert_eq!(summary.feed, RemoteState::Fetched);
    assert_eq!((fake.raw_requests(), fake.api_requests()), (2, 0), "{:?}", fake.requests());
    assert_eq!(summary.aggregated.len(), 16, "13 + 2 added apps + the toolbox: {:?}", summary.aggregated);
    assert_eq!(summary.fetched.len(), 16);
    assert!(summary.failed.is_empty() && summary.skipped.is_empty());
    // The catalog grew, the rows follow, the feeds are in.
    assert_eq!(s.catalog().revision, 2);
    assert_eq!(s.catalog_source(), CatalogSource::Remote { fetched_at: t0() });
    let ids: Vec<String> = s.statuses().into_iter().map(|r| r.id).collect();
    assert!(ids.contains(&"testcraft".to_string()) && ids.contains(&"patterncraft".to_string()), "{ids:?}");
    assert_eq!(s.app_status("photocraft").unwrap().status, Status::NotInstalled { latest: Version::new(0, 5, 0) });
    assert_eq!(s.app_status("patterncraft").unwrap().status, Status::NotInstalled { latest: Version::new(9, 0, 0) });
    assert!(matches!(s.app_status("artcraft").unwrap().status, Status::Unsupported { .. }), "no Linux build");
    assert_eq!(s.app_status("testcraft").unwrap().checked_at, Some(t0()));
    let status = s.execute("catalog.status", json!({})).unwrap();
    assert_eq!((status["source"].as_str(), status["revision"].as_u64(), status["feedFetchedAt"].as_u64()), (Some("remote"), Some(2), Some(t0())));
    assert!(root.join("remote/catalog.json").is_file() && root.join("remote/feed.json").is_file() && root.join("feeds/photocraft.json").is_file());
    // An hour later both documents are unchanged: two conditional requests, every app unchanged.
    fake.reset();
    s.set_clock(t1);
    let summary = check(&mut s);
    assert_eq!((summary.catalog.clone(), summary.feed.clone()), (RemoteState::Unchanged, RemoteState::Unchanged), "{summary:?}");
    assert_eq!((fake.raw_requests(), fake.api_requests()), (2, 0));
    assert!(fake.requests().iter().all(|(_, etag)| etag.is_some()), "{:?}", fake.requests());
    assert_eq!(summary.unchanged.len(), 16);
    assert_eq!(s.app_status("testcraft").unwrap().checked_at, Some(t1()));
    assert_eq!(s.catalog_source(), CatalogSource::Remote { fetched_at: t1() });
    // Minutes later everything is fresh: not even the publisher is asked.
    fake.reset();
    let summary = check(&mut s);
    assert_eq!((summary.fresh.len(), fake.requests().len()), (16, 0));
    assert_eq!(summary.catalog, RemoteState::Off);
    // A new session on the same folder starts from the cached remote catalog, offline.
    let (mut again, warnings) = Session::open(Catalog::parse(&builtin_text()).unwrap(), Some(store), None);
    assert!(warnings.is_empty());
    again.set_host(Target::from_consts("linux", "x86_64"));
    assert_eq!((again.catalog().revision, again.catalog_source()), (2, CatalogSource::Remote { fetched_at: t1() }));
    assert!(again.catalog().get("testcraft").is_some());
    assert_eq!(again.app_status("patterncraft").unwrap().status, Status::NotInstalled { latest: Version::new(9, 0, 0) });
    assert_eq!(again.feed_fetched_at(), Some(t1()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_documents_fall_back_to_the_api_and_keep_the_built_in_catalog() {
    let fake = Fake::new();
    let other = SecretKey::from_seed([1; 32]);
    let catalog2 = Catalog::parse(&remote_text(2)).unwrap();
    // Signed by someone else.
    fake.answer(CATALOG_URL, sign(&other, Kind::Catalog, &remote_text(2)), "c");
    fake.answer(FEED_URL, sign(&other, Kind::Feed, &feed_text(&catalog2, true)), "f");
    fake.answer("https://api.github.com/repos/storytold/photocraft/releases?per_page=20", PHOTOCRAFT.into(), "p");
    let mut s = session(&fake, None);
    let summary = check(&mut s);
    let RemoteState::Failed { error } = &summary.catalog else { panic!("{summary:?}") };
    assert!(error.contains("another key"), "{error}");
    assert!(matches!(&summary.feed, RemoteState::Failed { error } if error.contains("another key")), "{summary:?}");
    assert_eq!((fake.raw_requests(), fake.api_requests()), (2, 14), "every app from the API: {:?}", fake.requests());
    assert_eq!(summary.fetched.len(), 14);
    assert!(summary.aggregated.is_empty());
    assert_eq!((s.catalog().revision, s.catalog_source()), (1, CatalogSource::Builtin));
    assert_eq!(s.app_status("photocraft").unwrap().status, Status::NotInstalled { latest: Version::new(0, 5, 0) }, "from the API");
    // A tampered payload, an older revision, a catalog that names another key, nothing published.
    let cases: Vec<(String, &str)> = vec![
        (sign(&publisher(), Kind::Catalog, &remote_text(2)).replace("TestCraft", "EvilCraft"), "doesn't match"),
        (sign(&publisher(), Kind::Catalog, &remote_text(0)), "older"),
        (sign(&publisher(), Kind::Catalog, &remote_text(2).replace(&publisher().public().to_string(), &other.public().to_string())), "pinned"),
        (sign(&publisher(), Kind::Feed, &remote_text(2)), "not a `catalog`"),
        ("{\"schema\":1}".into(), "not a signed document"),
    ];
    for (doc, want) in cases {
        let fake = Fake::new();
        fake.answer(CATALOG_URL, doc, "c");
        fake.answer(FEED_URL, sign(&publisher(), Kind::Feed, &feed_text(&Catalog::parse(&builtin_text()).unwrap(), true)), "f");
        let mut s = session(&fake, None);
        s.set_clock(t1);
        let summary = check(&mut s);
        let RemoteState::Failed { error } = &summary.catalog else { panic!("{want}: {summary:?}") };
        assert!(error.contains(want), "{want}: {error}");
        assert_eq!(summary.feed, RemoteState::Fetched, "a good feed still serves: {summary:?}");
        assert_eq!(fake.api_requests(), 0);
        assert_eq!(s.catalog().revision, 1);
    }
    let fake = Fake::new();
    fake.status(CATALOG_URL, 404);
    fake.status(FEED_URL, 404);
    let mut s = session(&fake, None);
    let summary = check(&mut s);
    assert!(matches!(&summary.catalog, RemoteState::Failed { error } if error.contains("not published")), "{summary:?}");
    assert!(matches!(&summary.feed, RemoteState::Failed { error } if error.contains("not published")), "{summary:?}");
    assert_eq!(fake.api_requests(), 14);
    // The setting off: the publisher is never asked.
    let fake = Fake::new();
    publish(&fake, "c", "f", true);
    let mut s = session(&fake, None);
    s.execute("settings.set", json!({"remoteCatalog": false})).unwrap();
    assert!(!s.remote_enabled());
    let summary = check(&mut s);
    assert_eq!((summary.catalog, summary.feed), (RemoteState::Off, RemoteState::Off));
    assert_eq!((fake.raw_requests(), fake.api_requests()), (0, 14));
    assert_eq!(s.execute("catalog.status", json!({})).unwrap()["enabled"], false);
}

#[test]
fn an_app_outside_the_contract_installs_from_the_feed_digest_and_never_without() {
    for with_digest in [true, false] {
        let root = temp(if with_digest { "digest" } else { "nodigest" });
        let fake = Fake::new();
        publish(&fake, "c", "f", with_digest);
        let mut s = session(&fake, Some(Store::open(root.join("data")).unwrap()));
        s.set_layout(Some(Layout {
            apps: root.join("apps"),
            desktop_entries: None,
            icons: None,
            start_menu: None,
            downloads: root.join("downloads"),
            kept: root.join("kept"),
        }));
        check(&mut s);
        assert_eq!(s.app_status("patterncraft").unwrap().status, Status::NotInstalled { latest: Version::new(9, 0, 0) });
        let result = s.execute("app.install", json!({"app": "patterncraft"}));
        if with_digest {
            let v = result.unwrap();
            assert_eq!(v["version"], "9.0.0", "{v}");
            assert_eq!(s.inventory().current("patterncraft").map(|i| i.version.clone()), Some(Version::new(9, 0, 0)));
            assert_eq!(s.app_status("patterncraft").unwrap().status, Status::UpToDate { installed: Version::new(9, 0, 0) });
        } else {
            let e = result.unwrap_err().to_string();
            assert!(e.contains("only installs what it can verify") && e.contains("PatternCraft_9.0.0_amd64.AppImage"), "{e}");
            assert!(s.inventory().current("patterncraft").is_none());
            assert!(!root.join("downloads").exists() || std::fs::read_dir(root.join("downloads")).unwrap().next().is_none(), "nothing was downloaded");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
