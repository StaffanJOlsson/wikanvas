//! Transport: one ureq agent, TLS roots, retries, timeouts, no redirects, error mapping (owner: W8)
//!
//! §7.6 "Transport" and "Retries". Roots come from `--ca-file` / `QUIRE_CA_FILE` (PEM), else the system bundle
//! (`SSL_CERT_FILE`, else the first of the usual bundle paths); an https URL with no bundle exits 3, and no flag
//! skips verification. Redirects are never followed: a 3xx exits 3 and names the `Location`. Plain `http://` to a
//! non-loopback host is refused unless `QUIRE_ALLOW_HTTP=1` or the profile's `allow_http`. Environment proxies are
//! not used, so the token goes only to the configured origin.
//!
//! Retries (`--retries`, default 2), after `Retry-After` (capped at 60 s), else 1 s and then 3 s: every method when
//! the server answered 429 or 503, or when the request never left (DNS, connect, TLS handshake); GET and HEAD also
//! after a timeout or a reset, including one while the body is read into memory ([`Client::send_text`]). A write
//! that was sent is never retried, and its error says `details.sent: true`. "Never left" is decided by the stage:
//! every error of the resolver ([`Lookup`]) and of the connector chain ([`ConnectStage`]: TCP connect and TLS
//! handshake, which ureq completes before it writes the request) is "not sent"; any later error is "sent".

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ureq::http::{self, Method, Response};
use ureq::tls::{PemItem, RootCerts, TlsConfig, TlsProvider};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};
use ureq::{Agent, Body};

use crate::config::{Env, Origin, Resolved, Scheme};
use crate::error::Failure;
use crate::output;

/// The default timeout of a request, in seconds.
pub const TIMEOUT: u64 = 30;
/// The default timeout of a transfer (export, boards, attachments), in seconds.
pub const TRANSFER_TIMEOUT: u64 = 300;
/// The largest body the CLI reads into memory (JSON bodies, content). Downloads stream.
pub const MAX_BODY: u64 = 1 << 30;
/// The system bundle locations tried after `SSL_CERT_FILE` (§7.6).
pub const SYSTEM_BUNDLES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// A request to send: the method, a path under the origin, an encoded query, headers and an optional body.
#[derive(Clone, Debug)]
pub struct Req {
    pub method: Method,
    /// Starts with `/api/`; may carry its own query string (`quire api`).
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    /// The body and its content type.
    pub body: Option<(Vec<u8>, String)>,
    /// Exports, boards and attachments: the transfer timeout.
    pub transfer: bool,
    /// Send the token (the default).
    pub auth: bool,
}

impl Req {
    pub fn new(method: Method, path: impl Into<String>) -> Req {
        Req {
            method,
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: None,
            transfer: false,
            auth: true,
        }
    }
    pub fn get(path: impl Into<String>) -> Req {
        Req::new(Method::GET, path)
    }
    pub fn post(path: impl Into<String>) -> Req {
        Req::new(Method::POST, path)
    }
    pub fn put(path: impl Into<String>) -> Req {
        Req::new(Method::PUT, path)
    }
    pub fn patch(path: impl Into<String>) -> Req {
        Req::new(Method::PATCH, path)
    }
    pub fn delete(path: impl Into<String>) -> Req {
        Req::new(Method::DELETE, path)
    }
    /// Adds a query parameter (encoded when the URL is built).
    pub fn query(mut self, name: &str, value: impl Into<String>) -> Req {
        self.query.push((name.to_owned(), value.into()));
        self
    }
    /// Adds a query parameter when `value` is set.
    pub fn query_opt(self, name: &str, value: Option<impl Into<String>>) -> Req {
        match value {
            Some(v) => self.query(name, v),
            None => self,
        }
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Req {
        self.headers.push((name.to_owned(), value.into()));
        self
    }
    /// A JSON body.
    pub fn json(mut self, value: &Value) -> Req {
        self.body = Some((value.to_string().into_bytes(), "application/json".into()));
        self
    }
    /// A raw body of `content_type`.
    pub fn raw(mut self, bytes: Vec<u8>, content_type: &str) -> Req {
        self.body = Some((bytes, content_type.to_owned()));
        self
    }
    pub fn transfer(mut self) -> Req {
        self.transfer = true;
        self
    }
    /// Without the token (public routes).
    pub fn public(mut self) -> Req {
        self.auth = false;
        self
    }
    fn is_read(&self) -> bool {
        self.method == Method::GET || self.method == Method::HEAD
    }
}

/// Percent-encodes a query component (RFC 3986 unreserved characters stay).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The client of one command: the agent, the origin and the token.
pub struct Client {
    agent: Agent,
    pub origin: Origin,
    token: Option<String>,
    retries: u32,
    timeout: Option<u64>,
}

/// How a transport error is reported and whether it may be retried.
struct NetError {
    code: &'static str,
    message: String,
    /// The request may have reached the server.
    sent: bool,
}

fn classify(err: &ureq::Error) -> NetError {
    use std::io::ErrorKind as K;
    use ureq::Timeout as T;
    // An error of the connect stage: whatever it is, nothing of the request was written.
    if let ureq::Error::Io(io) = err
        && let Some(BeforeSend(inner)) = io.get_ref().and_then(|e| e.downcast_ref::<BeforeSend>())
    {
        return NetError {
            sent: false,
            ..classify(inner)
        };
    }
    let message = err.to_string();
    let (code, sent) = match err {
        ureq::Error::HostNotFound => ("network", false),
        ureq::Error::ConnectionFailed => ("network", false),
        ureq::Error::Timeout(T::Resolve | T::Connect) => ("timeout", false),
        ureq::Error::Timeout(_) => ("timeout", true),
        ureq::Error::Tls(_) | ureq::Error::Pem(_) | ureq::Error::Rustls(_) => ("tls", false),
        ureq::Error::Io(io) => {
            if io
                .get_ref()
                .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
            {
                // Outside the connect stage (marked above), a TLS error comes from reading the response, after the
                // request was written: an alert from a TLS-terminating proxy, a decrypt error.
                ("tls", true)
            } else {
                match io.kind() {
                    K::ConnectionRefused
                    | K::AddrNotAvailable
                    | K::NetworkUnreachable
                    | K::HostUnreachable
                    | K::NetworkDown
                    | K::AddrInUse => ("network", false),
                    K::TimedOut | K::WouldBlock => ("timeout", true),
                    _ => ("network", true),
                }
            }
        }
        _ => ("network", true),
    };
    NetError {
        code,
        message,
        sent,
    }
}

/// Marks an error of the connect stage (TCP connect, TLS handshake): the request was not written.
#[derive(Debug)]
struct BeforeSend(ureq::Error);

impl fmt::Display for BeforeSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for BeforeSend {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// An error of the connect stage, marked "not sent". With only a global timeout set, ureq reports a connect or
/// handshake timeout as `Timeout::Global`, which here becomes `Timeout::Connect`; any other error is wrapped in
/// [`BeforeSend`] (ureq passes an I/O error whose inner error is not its own through unchanged).
fn before_send(err: ureq::Error) -> ureq::Error {
    match err {
        ureq::Error::Timeout(_) => ureq::Error::Timeout(ureq::Timeout::Connect),
        other => ureq::Error::Io(std::io::Error::other(BeforeSend(other))),
    }
}

/// The default connector chain (TCP, then rustls for https), with every error marked by [`before_send`]. ureq
/// finishes the TLS handshake (`complete_io`) inside `connect`, before it writes the request; a pooled connection
/// skips `connect` altogether.
#[derive(Debug, Default)]
struct ConnectStage(DefaultConnector);

impl Connector<()> for ConnectStage {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        self.0.connect(details, chained).map_err(before_send)
    }
}

/// The default resolver, with a failed lookup reported as `HostNotFound` (std reports it as an I/O error without
/// a kind of its own), so a DNS failure counts as "never left".
#[derive(Debug, Default)]
struct Lookup(DefaultResolver);

impl Resolver for Lookup {
    fn resolve(
        &self,
        uri: &http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        self.0.resolve(uri, config, timeout).map_err(|e| match e {
            ureq::Error::Io(_) => ureq::Error::HostNotFound,
            // A lookup that ran out of time never sent anything, whichever timeout was in force.
            ureq::Error::Timeout(_) => ureq::Error::Timeout(ureq::Timeout::Resolve),
            // Nothing was sent before the lookup either.
            other => before_send(other),
        })
    }
}

/// Loads the TLS roots: `ca_file`, else `SSL_CERT_FILE`, else the first system bundle that exists.
fn load_roots(ca_file: Option<&std::path::Path>, env: &Env) -> Result<RootCerts, Failure> {
    let (path, source) = if let Some(p) = ca_file {
        (
            p.to_path_buf(),
            "--ca-file / QUIRE_CA_FILE / the profile's ca_file",
        )
    } else if let Some(p) = env.get("SSL_CERT_FILE") {
        (std::path::PathBuf::from(p), "SSL_CERT_FILE")
    } else {
        match SYSTEM_BUNDLES
            .iter()
            .map(std::path::Path::new)
            .find(|p| p.is_file())
        {
            Some(p) => (p.to_path_buf(), "the system bundle"),
            None => {
                return Err(Failure::config(format!(
                    "no CA bundle for https: none of {} exists; pass --ca-file or set SSL_CERT_FILE",
                    SYSTEM_BUNDLES.join(", ")
                )));
            }
        }
    };
    let pem = std::fs::read(&path).map_err(|e| {
        Failure::config(format!(
            "cannot read the CA bundle {} ({source}): {e}",
            path.display()
        ))
    })?;
    let mut certs = Vec::new();
    for item in ureq::tls::parse_pem(&pem) {
        if let Ok(PemItem::Certificate(cert)) = item {
            certs.push(cert);
        }
    }
    if certs.is_empty() {
        return Err(Failure::config(format!(
            "the CA bundle {} ({source}) holds no certificate",
            path.display()
        )));
    }
    Ok(RootCerts::Specific(Arc::new(certs)))
}

impl Client {
    /// The client for a resolved configuration. No URL, plain HTTP to a non-loopback host, or an https URL without
    /// a CA bundle exit 3 before any request.
    pub fn new(
        resolved: &Resolved,
        env: &Env,
        retries: u32,
        timeout: Option<u64>,
    ) -> Result<Client, Failure> {
        let origin = resolved.url.clone().ok_or_else(|| {
            Failure::config(
                "no Quire URL: pass --url, set QUIRE_URL, or run `quire auth login --url URL --token-stdin`",
            )
        })?;
        if origin.scheme == Scheme::Http && !origin.is_loopback() && !resolved.allow_http {
            return Err(Failure::config(format!(
                "{} is plain HTTP to a host that is not this machine: use https, or set QUIRE_ALLOW_HTTP=1 \
                 (or allow_http = true in the profile) if the network is trusted",
                origin.base()
            )));
        }
        let roots = match origin.scheme {
            Scheme::Https => load_roots(resolved.ca_file.as_deref(), env)?,
            Scheme::Http => RootCerts::Specific(Arc::new(Vec::new())),
        };
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(roots)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .user_agent(format!("quire-cli/{}", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build();
        let agent = Agent::with_parts(config, ConnectStage::default(), Lookup::default());
        Ok(Client {
            agent,
            origin,
            token: resolved.token.clone(),
            retries,
            timeout,
        })
    }

    /// Whether a token is configured.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// The URL of a request: the origin plus the path by concatenation (never a URL join), plus the query.
    pub fn url_of(&self, req: &Req) -> String {
        let mut url = format!("{}{}", self.origin.base(), req.path);
        for (i, (k, v)) in req.query.iter().enumerate() {
            let sep = if i == 0 && !req.path.contains('?') {
                '?'
            } else {
                '&'
            };
            url.push(sep);
            url.push_str(&encode_component(k));
            url.push('=');
            url.push_str(&encode_component(v));
        }
        url
    }

    fn attempt(
        &self,
        req: &Req,
        url: &str,
    ) -> Result<Result<Response<Body>, ureq::Error>, Failure> {
        let mut builder = http::Request::builder().method(req.method.clone()).uri(url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if req.auth
            && let Some(t) = &self.token
        {
            builder = builder.header("Authorization", format!("Bearer {t}"));
        }
        let secs = self.timeout.unwrap_or(if req.transfer {
            TRANSFER_TIMEOUT
        } else {
            TIMEOUT
        });
        let timeout = Some(Duration::from_secs(secs));
        let bad = |e: http::Error| Failure::usage(format!("cannot build the request: {e}"));
        Ok(match &req.body {
            Some((bytes, ct)) => {
                let request = builder
                    .header("Content-Type", ct.as_str())
                    .body(bytes.as_slice())
                    .map_err(bad)?;
                let request = self
                    .agent
                    .configure_request(request)
                    .timeout_global(timeout)
                    .build();
                self.agent.run(request)
            }
            None => {
                let request = builder.body(()).map_err(bad)?;
                let request = self
                    .agent
                    .configure_request(request)
                    .timeout_global(timeout)
                    .build();
                self.agent.run(request)
            }
        })
    }

    fn need_token(&self, req: &Req) -> Result<(), Failure> {
        if req.auth && self.token.is_none() {
            return Err(Failure::config(
                "no API token: pass --token-file or --token-stdin, set QUIRE_TOKEN or QUIRE_TOKEN_FILE, or run \
                 `quire auth login`",
            ));
        }
        Ok(())
    }

    /// Sends `req` with the retry rule and returns the response of any status (3xx included). Only transport
    /// failures are errors here.
    pub fn send_any(&self, req: &Req) -> Result<Response<Body>, Failure> {
        self.need_token(req)?;
        let mut attempt = 0;
        self.exchange(req, &self.url_of(req), &mut attempt)
    }

    /// The retry loop up to the response headers. `attempt` counts every attempt of the command's request, so a
    /// retry after a failed body read ([`send_text`](Self::send_text)) uses the same `--retries` budget.
    fn exchange(&self, req: &Req, url: &str, attempt: &mut u32) -> Result<Response<Body>, Failure> {
        loop {
            let started = Instant::now();
            let result = self.attempt(req, url)?;
            let elapsed = started.elapsed().as_millis() as u64;
            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let request_id = header(resp.headers(), quire_api::common::header::REQUEST_ID);
                    output::debug(json!({
                        "method": req.method.as_str(), "url": url, "status": status,
                        "duration_ms": elapsed, "request_id": request_id, "attempt": *attempt + 1,
                    }));
                    if let Some(server) =
                        header(resp.headers(), quire_api::common::header::API_VERSION)
                        && api_version_older(&server)
                    {
                        output::warn_api_version_once(&server);
                    }
                    if (status == 429 || status == 503) && *attempt < self.retries {
                        let wait = retry_after(resp.headers()).unwrap_or(backoff(*attempt));
                        std::thread::sleep(wait);
                        *attempt += 1;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(err) => {
                    let net = classify(&err);
                    output::debug(json!({
                        "method": req.method.as_str(), "url": url, "status": null,
                        "duration_ms": elapsed, "request_id": null, "attempt": *attempt + 1,
                        "error": net.message,
                    }));
                    if *attempt < self.retries && (!net.sent || req.is_read()) {
                        std::thread::sleep(backoff(*attempt));
                        *attempt += 1;
                        continue;
                    }
                    return Err(Failure::network(
                        net.code,
                        format!(
                            "{} {}: {}{}",
                            req.method,
                            url,
                            net.message,
                            unknown_outcome(req, net.sent)
                        ),
                        net.sent,
                    ));
                }
            }
        }
    }

    /// Sends `req` and returns a 2xx response; any other status is the command's error (§7.7).
    pub fn send(&self, req: &Req) -> Result<Response<Body>, Failure> {
        let resp = self.send_any(req)?;
        expect_success(resp)
    }

    /// Sends `req` and reads its 2xx response into memory (JSON, Markdown, envelopes; at most [`MAX_BODY`]). A GET
    /// or HEAD whose body fails in transit (a timeout, a reset) is sent again, within the same `--retries` budget;
    /// a write whose body fails is exit 4 with `details.sent: true`.
    pub fn send_text(&self, req: &Req) -> Result<Text, Failure> {
        match self.send_text_or_stream(req, |_| false)? {
            Answer::Text(t) => Ok(t),
            Answer::Stream(_) => unreachable!("send_text_or_stream streams only when asked to"),
        }
    }

    /// Like [`send_text`](Self::send_text), but a response for which `stream` is true is returned unread, for a
    /// download (`quire api` decides by the content type).
    pub fn send_text_or_stream(
        &self,
        req: &Req,
        stream: impl Fn(&Response<Body>) -> bool,
    ) -> Result<Answer, Failure> {
        self.need_token(req)?;
        let url = self.url_of(req);
        let mut attempt = 0;
        loop {
            let resp = expect_success(self.exchange(req, &url, &mut attempt)?)?;
            if stream(&resp) {
                return Ok(Answer::Stream(resp));
            }
            let status = resp.status().as_u16();
            let headers = resp.headers().clone();
            let started = Instant::now();
            let err = match read_body(resp) {
                Ok(bytes) => {
                    let text = String::from_utf8(bytes).map_err(|_| {
                        Failure::bad_response(format!(
                            "{} {url}: the response is not UTF-8",
                            req.method
                        ))
                    })?;
                    return Ok(Answer::Text(Text {
                        status,
                        headers,
                        text,
                    }));
                }
                Err(err) => err,
            };
            let net = classify(&err);
            output::debug(json!({
                "method": req.method.as_str(), "url": url, "status": status,
                "duration_ms": started.elapsed().as_millis() as u64,
                "request_id": header(&headers, quire_api::common::header::REQUEST_ID),
                "attempt": attempt + 1, "error": format!("reading the response: {}", net.message),
            }));
            let too_big = matches!(err, ureq::Error::BodyExceedsLimit(_));
            if req.is_read() && !too_big && attempt < self.retries {
                std::thread::sleep(backoff(attempt));
                attempt += 1;
                continue;
            }
            return Err(Failure::network(
                net.code,
                format!(
                    "{} {url}: reading the response: {}{}",
                    req.method,
                    net.message,
                    unknown_outcome(req, true)
                ),
                true,
            ));
        }
    }
}

/// The note on an error after which a write's outcome is unknown.
fn unknown_outcome(req: &Req, sent: bool) -> &'static str {
    if sent && !req.is_read() {
        "; the request may have reached the server: check before retrying"
    } else {
        ""
    }
}

/// A 2xx response read into memory.
#[derive(Debug)]
pub struct Text {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub text: String,
}

impl Text {
    /// A response header as text.
    pub fn header(&self, name: &str) -> Option<String> {
        header(&self.headers, name)
    }
}

/// What [`Client::send_text_or_stream`] returns.
pub enum Answer {
    Text(Text),
    /// The response, its body unread.
    Stream(Response<Body>),
}

/// A 2xx response as it is; a 3xx is exit 3 naming the `Location`; any other status is the server's error.
pub fn expect_success(mut resp: Response<Body>) -> Result<Response<Body>, Failure> {
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(resp);
    }
    let request_id = header(resp.headers(), quire_api::common::header::REQUEST_ID);
    if (300..400).contains(&status) {
        let location = header(resp.headers(), "location");
        return Err(Failure::redirect(status, location.as_deref(), request_id));
    }
    let body = resp
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_vec()
        .unwrap_or_default();
    Err(Failure::from_response(status, request_id, &body))
}

/// A response header as text.
pub fn header(headers: &http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Whether the server's `Quire-Api-Version` is older than the CLI's (a lower minor of the same major, or a lower
/// major).
fn api_version_older(server: &str) -> bool {
    fn parse(v: &str) -> Option<(u32, u32)> {
        let (a, b) = v.trim().split_once('.')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    }
    match (parse(server), parse(quire_api::API_VERSION)) {
        (Some(s), Some(c)) => s < c,
        _ => false,
    }
}

fn retry_after(headers: &http::HeaderMap) -> Option<Duration> {
    let secs: u64 = header(headers, "retry-after")?.trim().parse().ok()?;
    Some(Duration::from_secs(secs.min(60)))
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(if attempt == 0 { 1 } else { 3 })
}

fn body_error(err: std::io::Error) -> Failure {
    let err = ureq::Error::from(err);
    let net = classify(&err);
    Failure::network(
        net.code,
        format!("reading the response: {}", net.message),
        true,
    )
}

/// Reads a whole body (at most [`MAX_BODY`]).
fn read_body(resp: Response<Body>) -> Result<Vec<u8>, ureq::Error> {
    let mut bytes = Vec::new();
    resp.into_body()
        .into_with_config()
        .limit(MAX_BODY)
        .reader()
        .read_to_end(&mut bytes)
        .map_err(ureq::Error::from)?;
    Ok(bytes)
}

/// What a streamed download wrote.
pub struct Written {
    pub bytes: u64,
    pub sha256: String,
}

/// Streams a body to `out`, hashing it on the way.
pub fn stream_to(
    resp: Response<Body>,
    out: &mut dyn Write,
    what: &str,
) -> Result<Written, Failure> {
    let mut reader = resp.into_body().into_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(body_error(e)),
        };
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| Failure::io(what, &e))?;
        total += n as u64;
    }
    out.flush().map_err(|e| Failure::io(what, &e))?;
    Ok(Written {
        bytes: total,
        sha256: hex(&hasher.finalize()),
    })
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(encode_component("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(encode_component("quok -zebra"), "quok%20-zebra");
    }

    #[test]
    fn api_versions() {
        assert!(api_version_older("0.9"));
        assert!(!api_version_older("1.0"));
        assert!(!api_version_older("1.1"));
        assert!(!api_version_older("x"));
    }

    #[test]
    fn retry_waits() {
        let mut h = http::HeaderMap::new();
        h.insert("retry-after", "7".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_secs(7)));
        h.insert("retry-after", "600".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_secs(60)));
        h.insert(
            "retry-after",
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&h), None);
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(3));
        assert_eq!(backoff(5), Duration::from_secs(3));
    }

    #[test]
    fn transport_errors_say_whether_sent() {
        let io = |k| ureq::Error::Io(std::io::Error::from(k));
        use std::io::ErrorKind as K;
        assert!(!classify(&io(K::ConnectionRefused)).sent);
        assert!(!classify(&ureq::Error::HostNotFound).sent);
        assert!(!classify(&ureq::Error::Timeout(ureq::Timeout::Connect)).sent);
        assert!(classify(&ureq::Error::Timeout(ureq::Timeout::Global)).sent);
        assert_eq!(
            classify(&ureq::Error::Timeout(ureq::Timeout::Global)).code,
            "timeout"
        );
        assert!(classify(&io(K::ConnectionReset)).sent);
        assert!(classify(&io(K::UnexpectedEof)).sent);
        assert_eq!(classify(&ureq::Error::Tls("x")).code, "tls");
    }

    /// A TLS error while the response is read: rustls wraps it in an I/O error (InvalidData).
    fn tls_read_error() -> ureq::Error {
        ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::DecryptError,
        ))
    }

    #[test]
    fn a_tls_error_after_the_handshake_counts_as_sent() {
        let net = classify(&tls_read_error());
        assert_eq!((net.code, net.sent), ("tls", true));
    }

    #[test]
    fn every_connect_stage_error_counts_as_not_sent() {
        use std::io::ErrorKind as K;
        let io = |k| ureq::Error::Io(std::io::Error::from(k));
        // With only a global timeout set, ureq names a connect or handshake timeout `Global`.
        let t = before_send(ureq::Error::Timeout(ureq::Timeout::Global));
        assert!(
            matches!(t, ureq::Error::Timeout(ureq::Timeout::Connect)),
            "{t}"
        );
        for (err, code) in [
            (ureq::Error::Timeout(ureq::Timeout::Global), "timeout"),
            (tls_read_error(), "tls"),
            (io(K::ConnectionReset), "network"),
            (io(K::UnexpectedEof), "network"),
            (io(K::TimedOut), "timeout"),
            (io(K::ConnectionRefused), "network"),
            (ureq::Error::Tls("x"), "tls"),
        ] {
            let what = err.to_string();
            let net = classify(&before_send(err));
            assert_eq!((net.code, net.sent), (code, false), "{what}");
            assert!(!net.message.contains("BeforeSend"), "{}", net.message);
        }
        // Errors pass through the wrapper's Display unchanged.
        let e = before_send(io(K::ConnectionReset));
        assert_eq!(classify(&e).message, io(K::ConnectionReset).to_string());
    }
}
