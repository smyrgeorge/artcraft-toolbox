//! [`Client`] against a local HTTP server: what goes out (headers, token scoping, redirect
//! checks) and how every kind of answer comes back. No internet access.

use std::io::{Read, Write};

use artcraft_toolbox_net::Download;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use artcraft_toolbox_net::{Client, NetError, Policy, RateLimit, Request, Response, Transport};

/// A server that answers each connection with the next canned response and records the request
/// heads it received.
struct Server {
    host: &'static str,
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(responses: Vec<String>) -> Server {
        Server::start_on("127.0.0.1", responses)
    }

    /// Listening on the first address `host` resolves to, which is also the first a client
    /// connecting to `host` tries (`localhost` is `::1` before 127.0.0.1 on Windows).
    fn start_on(host: &'static str, responses: Vec<String>) -> Server {
        let listener = TcpListener::bind((host, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                log.lock().unwrap().push(read_head(&mut stream));
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Server { host, port, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}:{}{path}", self.host, self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_head(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
        head.push(byte[0]);
    }
    String::from_utf8_lossy(&head).to_lowercase()
}

fn reply(status: &str, headers: &[&str], body: &str) -> String {
    let mut s = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
    for h in headers {
        s.push_str(h);
        s.push_str("\r\n");
    }
    s.push_str("\r\n");
    s.push_str(body);
    s
}

fn policy(hosts: &[&str], token_hosts: &[&str]) -> Policy {
    Policy {
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        token_hosts: token_hosts.iter().map(|h| h.to_string()).collect(),
        allow_http: true,
        max_redirects: 3,
        timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(2),
        download_timeout: Duration::from_secs(5),
        max_download_bytes: 1 << 20,
    }
}

fn client(hosts: &[&str], token_hosts: &[&str], token: Option<&str>) -> Client {
    Client::new(policy(hosts, token_hosts), "ArtCraft-Toolbox/test", token.map(str::to_string))
}

fn get<'a>(url: &'a str, etag: Option<&'a str>) -> Request<'a> {
    Request { url, accept: "application/vnd.github+json", etag, max_bytes: 1024 }
}

const RATE: [&str; 3] = ["x-ratelimit-limit: 60", "x-ratelimit-remaining: 41", "x-ratelimit-reset: 1791528397"];

#[test]
fn ok_with_etag_and_rate_limit_headers() {
    let s = Server::start(vec![reply("200 OK", &[RATE[0], RATE[1], RATE[2], "etag: W/\"abc\""], "[]")]);
    let r = client(&["127.0.0.1"], &[], None).get(&get(&s.url("/repos/o/r/releases"), None)).unwrap();
    let rate = RateLimit { limit: Some(60), remaining: Some(41), reset_at: Some(1_791_528_397) };
    assert_eq!(r, Response::Ok { body: b"[]".to_vec(), etag: Some("W/\"abc\"".into()), rate });
    let head = &s.requests()[0];
    assert!(head.starts_with("get /repos/o/r/releases http/1.1"), "{head}");
    assert!(head.contains("user-agent: artcraft-toolbox/test"), "{head}");
    assert!(head.contains("accept: application/vnd.github+json"), "{head}");
    assert!(head.contains("x-github-api-version: 2022-11-28"), "{head}");
    assert!(!head.contains("authorization"), "{head}");
}

#[test]
fn conditional_requests_and_not_modified() {
    let s = Server::start(vec![reply("304 Not Modified", &[RATE[1]], "")]);
    let r = client(&["127.0.0.1"], &[], None).get(&get(&s.url("/x"), Some("W/\"abc\""))).unwrap();
    assert_eq!(r, Response::NotModified { rate: RateLimit { remaining: Some(41), ..RateLimit::default() } });
    assert!(s.requests()[0].contains("if-none-match: w/\"abc\""));
}

#[test]
fn rate_limit_refusals() {
    let s = Server::start(vec![
        reply("403 Forbidden", &["x-ratelimit-remaining: 0", "x-ratelimit-reset: 1791528397"], r#"{"message":"API rate limit exceeded"}"#),
        reply("429 Too Many Requests", &["retry-after: 30"], ""),
    ]);
    let c = client(&["127.0.0.1"], &[], None);
    assert_eq!(c.get(&get(&s.url("/a"), None)), Err(NetError::RateLimited { reset_at: Some(1_791_528_397), retry_after: None }));
    assert_eq!(c.get(&get(&s.url("/b"), None)), Err(NetError::RateLimited { reset_at: None, retry_after: Some(30) }));
}

#[test]
fn errors_carry_github_messages() {
    let s = Server::start(vec![reply("404 Not Found", &[], r#"{"message":"Not Found"}"#)]);
    assert_eq!(client(&["127.0.0.1"], &[], None).get(&get(&s.url("/missing"), None)), Err(NetError::Status { status: 404, message: "Not Found".into() }));
}

#[test]
fn bodies_are_capped() {
    let big = "x".repeat(4096);
    let s = Server::start(vec![reply("200 OK", &[], &big)]);
    assert_eq!(client(&["127.0.0.1"], &[], None).get(&get(&s.url("/big"), None)), Err(NetError::TooLarge(1024)));
}

#[test]
fn redirects_are_followed_only_to_allowed_hosts() {
    let s = Server::start(vec![reply("301 Moved", &["location: /moved"], ""), reply("200 OK", &[], "[1]")]);
    let r = client(&["127.0.0.1"], &[], None).get(&get(&s.url("/old"), None)).unwrap();
    assert!(matches!(r, Response::Ok { ref body, .. } if body == b"[1]"));
    assert!(s.requests()[1].starts_with("get /moved "), "{:?}", s.requests());

    let s = Server::start(vec![reply("302 Found", &["location: https://evil.example/payload"], "")]);
    assert_eq!(client(&["127.0.0.1"], &[], None).get(&get(&s.url("/x"), None)), Err(NetError::Forbidden("https://evil.example/payload".into())));
    assert_eq!(s.requests().len(), 1, "the forbidden host is never contacted");

    let loops: Vec<String> = (0..5).map(|_| reply("302 Found", &["location: /again"], "")).collect();
    let s = Server::start(loops);
    assert_eq!(client(&["127.0.0.1"], &[], None).get(&get(&s.url("/again"), None)), Err(NetError::TooManyRedirects));
}

#[test]
fn the_token_goes_only_to_token_hosts_and_never_follows_a_redirect_elsewhere() {
    // `localhost` reaches the same machine as 127.0.0.1 but is another host: it may receive
    // requests (it is in `hosts`) but not the token (it is not in `token_hosts`).
    let b = Server::start_on("localhost", vec![reply("200 OK", &[], "[]")]);
    let a = Server::start(vec![reply("302 Found", &[&format!("location: {}", b.url("/next"))], "")]);
    let c = client(&["127.0.0.1", "localhost"], &["127.0.0.1"], Some("ghp_secret"));
    assert!(matches!(c.get(&get(&a.url("/first"), None)), Ok(Response::Ok { .. })));
    assert!(a.requests()[0].contains("authorization: bearer ghp_secret"), "{:?}", a.requests());
    let to_b = b.requests();
    assert_eq!(to_b.len(), 1);
    assert!(to_b[0].starts_with("get /next ") && !to_b[0].contains("authorization"), "{to_b:?}");
}

#[test]
fn no_token_without_a_token_host() {
    let s = Server::start(vec![reply("200 OK", &[], "[]")]);
    let _ = client(&["127.0.0.1"], &[], Some("ghp_secret")).get(&get(&s.url("/x"), None)).unwrap();
    assert!(!s.requests()[0].contains("authorization"));
}

#[test]
fn forbidden_and_unreachable_hosts() {
    let c = client(&["127.0.0.1"], &[], None);
    assert!(matches!(c.get(&get("http://example.com/", None)), Err(NetError::Forbidden(_))));
    let strict = Client::new(Policy { allow_http: false, ..policy(&["127.0.0.1"], &[]) }, "t", None);
    assert!(matches!(strict.get(&get("http://127.0.0.1:1/", None)), Err(NetError::Forbidden(_))), "plain http is refused by default");
    // A closed port: a connection error, not a panic. Windows retries a refused connection for
    // about as long as the connect timeout, so it may end as a timeout there.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert!(matches!(c.get(&get(&format!("http://127.0.0.1:{closed}/"), None)), Err(NetError::Connect(_) | NetError::Timeout)));
}

fn body(mut d: Download) -> Vec<u8> {
    let mut v = Vec::new();
    d.reader.read_to_end(&mut v).unwrap();
    v
}

#[test]
fn downloads_stream_the_whole_file() {
    let s = Server::start(vec![reply("200 OK", &[], "0123456789")]);
    let d = client(&["127.0.0.1"], &["127.0.0.1"], Some("ghp_secret")).download(&s.url("/a.AppImage"), 0).unwrap();
    assert_eq!((d.offset, d.total), (0, Some(10)));
    assert_eq!(body(d), b"0123456789");
    let head = &s.requests()[0];
    assert!(!head.contains("range:") && !head.contains("authorization"), "downloads never carry the token: {head}");
}

#[test]
fn downloads_resume_with_a_range() {
    let s = Server::start(vec![reply("206 Partial Content", &["content-range: bytes 4-9/10"], "456789")]);
    let d = client(&["127.0.0.1"], &[], None).download(&s.url("/a"), 4).unwrap();
    assert_eq!((d.offset, d.total), (4, Some(10)));
    assert_eq!(body(d), b"456789");
    assert!(s.requests()[0].contains("range: bytes=4-"));

    // A server that ignores the range sends everything from 0: the caller starts over.
    let s = Server::start(vec![reply("200 OK", &[], "0123456789")]);
    assert_eq!(client(&["127.0.0.1"], &[], None).download(&s.url("/a"), 4).unwrap().offset, 0);

    // A partial answer from the wrong place is an error, never silently appended.
    let s = Server::start(vec![reply("206 Partial Content", &["content-range: bytes 2-9/10"], "23456789")]);
    assert!(matches!(client(&["127.0.0.1"], &[], None).download(&s.url("/a"), 4), Err(NetError::Other(_))));

    let s = Server::start(vec![reply("416 Range Not Satisfiable", &[], "")]);
    assert!(matches!(client(&["127.0.0.1"], &[], None).download(&s.url("/a"), 99), Err(NetError::Status { status: 416, .. })));
}

#[test]
fn downloads_follow_checked_redirects_and_respect_the_size_cap() {
    let b = Server::start_on("localhost", vec![reply("200 OK", &[], "payload")]);
    let a = Server::start(vec![reply("302 Found", &[&format!("location: {}", b.url("/asset"))], "")]);
    let d = client(&["127.0.0.1", "localhost"], &[], None).download(&a.url("/releases/download/v1/x"), 0).unwrap();
    assert_eq!(body(d), b"payload");

    let s = Server::start(vec![reply("302 Found", &["location: https://evil.example/x"], "")]);
    assert!(matches!(client(&["127.0.0.1"], &[], None).download(&s.url("/x"), 0), Err(NetError::Forbidden(_))));

    // Announced as larger than the cap (1 MiB here): refused before reading.
    let s = Server::start(vec![reply("206 Partial Content", &["content-range: bytes 0-0/99999999"], "x")]);
    assert!(matches!(client(&["127.0.0.1"], &[], None).download(&s.url("/x"), 0), Err(NetError::TooLarge(_))));
}
