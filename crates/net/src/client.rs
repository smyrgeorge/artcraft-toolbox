//! [`Client`]: the real [`Transport`], on ureq with rustls and the OS certificate store.

use std::io::Read;

use crate::{GITHUB_API_VERSION, NetError, Policy, Request, Response, Transport, classify, parse_rate, url};

/// How much of an error response's body is read for its message.
const ERROR_BODY_BYTES: u64 = 64 * 1024;

pub struct Client {
    agent: ureq::Agent,
    policy: Policy,
    token: Option<String>,
}

impl Client {
    /// A client for `policy`. `token` (a GitHub token the user provided) is sent only to the
    /// policy's token hosts.
    pub fn new(policy: Policy, user_agent: &str, token: Option<String>) -> Client {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let config = ureq::Agent::config_builder()
            .https_only(!policy.allow_http)
            .http_status_as_error(false)
            // Redirects are followed below, one checked hop at a time.
            .max_redirects(0)
            .timeout_global(Some(policy.timeout))
            .timeout_connect(Some(policy.connect_timeout))
            .user_agent(user_agent)
            .tls_config(tls)
            .build();
        let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic()));
        Client { agent: config.new_agent(), policy, token }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }
}

impl Transport for Client {
    fn get(&self, req: &Request<'_>) -> Result<Response, NetError> {
        let mut target = req.url.to_string();
        for _ in 0..=self.policy.max_redirects {
            let u = url::check(&self.policy, &target)?;
            let mut call = self.agent.get(&target).header("Accept", req.accept).header("X-GitHub-Api-Version", GITHUB_API_VERSION);
            if let Some(etag) = req.etag {
                call = call.header("If-None-Match", etag);
            }
            if let Some(token) = &self.token
                && self.policy.token_hosts.iter().any(|h| h.eq_ignore_ascii_case(&u.host))
            {
                call = call.header("Authorization", format!("Bearer {token}"));
            }
            let mut resp = call.call().map_err(map_err)?;
            let status = resp.status().as_u16();
            let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
            let rate = parse_rate(header);
            log::debug!("GET {} -> {status} (rate {:?})", u.origin(), rate.remaining);
            match status {
                301 | 302 | 303 | 307 | 308 => {
                    let location = header("location").ok_or_else(|| NetError::Other(format!("HTTP {status} without a Location")))?;
                    target = url::resolve_location(&u, &location).ok_or_else(|| NetError::Forbidden(location.chars().take(120).collect()))?;
                }
                304 => return Ok(Response::NotModified { rate }),
                200..=299 => {
                    let etag = header("etag").filter(|e| e.len() <= 256);
                    let limit = u64::try_from(req.max_bytes).unwrap_or(u64::MAX);
                    let body = resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| match e {
                        ureq::Error::BodyExceedsLimit(_) => NetError::TooLarge(req.max_bytes),
                        e => map_err(e),
                    })?;
                    return Ok(Response::Ok { body, etag, rate });
                }
                _ => {
                    let retry_after = header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
                    let mut body = Vec::new();
                    // The message is a nicety: a body that can't be read still yields the status.
                    let _ = resp.body_mut().as_reader().take(ERROR_BODY_BYTES).read_to_end(&mut body);
                    return Err(classify(status, &rate, retry_after, &body));
                }
            }
        }
        Err(NetError::TooManyRedirects)
    }
}

fn map_err(e: ureq::Error) -> NetError {
    match e {
        ureq::Error::Timeout(_) => NetError::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => NetError::Connect(e.to_string()),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => NetError::Timeout,
        ureq::Error::Io(io) => NetError::Connect(io.to_string()),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => NetError::Tls(e.to_string()),
        ureq::Error::TooManyRedirects => NetError::TooManyRedirects,
        ureq::Error::BodyExceedsLimit(n) => NetError::TooLarge(usize::try_from(n).unwrap_or(usize::MAX)),
        other => NetError::Other(other.to_string()),
    }
}
