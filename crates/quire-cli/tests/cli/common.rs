//! Shared helpers of the CLI tests (owner: W8)
//!
//! [`Sandbox`] is a temporary directory holding `HOME` and the `QUIRE_CONFIG` file, and builds an assert_cmd
//! [`Command`] for the `quire` binary that inherits no `QUIRE_*` variable and no config or CA location from the
//! environment the tests run in. [`Harness`] adds a running `quire_server::testing::TestServer` and points
//! `QUIRE_URL` and `QUIRE_TOKEN` at it. [`Mock`] is a scripted HTTP listener (std `TcpListener`, one request per
//! connection) that records every request and answers what the test's handler says. [`RawListener`] speaks no
//! HTTP at all, for TLS handshakes that stall or fail.

#![allow(dead_code)]

use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assert_cmd::Command;
use quire_api::Scope;
use quire_server::testing::TestServer;
use serde_json::Value;
use tempfile::TempDir;

/// Variables outside `QUIRE_*` that the CLI reads for its config directory, its CA bundle or logging; a test sets
/// them explicitly when it needs them.
const SCRUBBED: &[&str] = &[
    "XDG_CONFIG_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "RUST_LOG",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// A UUID used as a reference where any id will do.
pub const ID: &str = "0192f3a4-5b6c-7d8e-9f01-23456789abcd";

/// This machine's address on its outbound interface (no packet is sent), when it has one that is not loopback.
pub fn non_loopback_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// A well-formed token that no server knows.
pub fn fake_token(seed: u8) -> String {
    quire_api::token::encode(&[seed; 32])
}

/// A temporary `HOME` and config file for one `quire` invocation or more. Deleted on drop.
pub struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    /// Creates `<tmp>/home` (mode 0700). The config file's directory is left to the CLI, which creates it 0700.
    pub fn new() -> Sandbox {
        let dir = tempfile::Builder::new()
            .prefix("quire-cli-test-")
            .tempdir()
            .expect("a temporary directory");
        DirBuilder::new()
            .mode(0o700)
            .create(dir.path().join("home"))
            .expect("the temporary HOME");
        Sandbox { dir }
    }

    /// The temporary directory; also the working directory of every command.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `HOME`: `<tmp>/home`.
    pub fn home(&self) -> PathBuf {
        self.path().join("home")
    }

    /// `QUIRE_CONFIG`: `<tmp>/config/config.toml` (not created).
    pub fn config_path(&self) -> PathBuf {
        self.path().join("config").join("config.toml")
    }

    /// Writes the profile file with `mode`.
    pub fn write_config(&self, text: &str, mode: u32) {
        let path = self.config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Writes a file in the sandbox with `mode` and returns its path.
    pub fn file(&self, name: &str, contents: &[u8], mode: u32) -> PathBuf {
        let path = self.path().join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    /// `quire` with `HOME` and `QUIRE_CONFIG` in the sandbox, run in the sandbox, and with no URL or token.
    pub fn quire(&self) -> Command {
        self.quire_bin(Path::new(env!("CARGO_BIN_EXE_quire")))
    }

    /// Like [`quire`](Self::quire), for another copy of the binary.
    pub fn quire_bin(&self, bin: &Path) -> Command {
        let mut cmd = Command::new(bin);
        for (name, _) in std::env::vars_os() {
            if name.as_encoded_bytes().starts_with(b"QUIRE_") {
                cmd.env_remove(&name);
            }
        }
        for name in SCRUBBED {
            cmd.env_remove(OsStr::new(name));
        }
        cmd.env("HOME", self.home())
            .env("QUIRE_CONFIG", self.config_path())
            .current_dir(self.path())
            .timeout(Duration::from_secs(60));
        cmd
    }

    /// `quire` pointed at `url` with `token` (through the environment).
    pub fn quire_at(&self, url: &str, token: &str) -> Command {
        let mut cmd = self.quire();
        cmd.env("QUIRE_URL", url).env("QUIRE_TOKEN", token);
        cmd
    }
}

/// A [`TestServer`] and a [`Sandbox`] whose commands talk to it.
pub struct Harness {
    /// Stops on drop, before the sandbox is deleted.
    pub server: TestServer,
    pub sandbox: Sandbox,
}

impl Harness {
    /// Starts a server with an owner and one token per scope.
    pub fn start() -> Harness {
        Harness {
            server: TestServer::start(),
            sandbox: Sandbox::new(),
        }
    }

    /// `quire` as the owner's write token: [`Sandbox::quire`] plus `QUIRE_URL` and `QUIRE_TOKEN`.
    pub fn quire(&self) -> Command {
        self.quire_as(Scope::Write)
    }

    /// `quire` as the owner's token of `scope`.
    pub fn quire_as(&self, scope: Scope) -> Command {
        self.sandbox
            .quire_at(&self.server.url, self.server.token(scope))
    }

    /// `quire --json …` as `scope`; returns the exit code, stdout as JSON (or Null) and stderr.
    pub fn json(&self, scope: Scope, args: &[&str]) -> (i32, Value, String) {
        run_json(self.quire_as(scope).arg("--json").args(args))
    }
}

/// Runs a command; returns the exit code, stdout parsed as one JSON value (Null when empty) and stderr.
pub fn run_json(cmd: &mut Command) -> (i32, Value, String) {
    let out = cmd.output().expect("quire runs");
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let value = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!("stdout is not one JSON value ({e}): {stdout}\nstderr: {stderr}")
        })
    };
    (code, value, stderr)
}

/// The last stderr line, parsed as JSON (the §7.7 error object).
pub fn last_error(stderr: &str) -> Value {
    let line = stderr
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or_else(|| panic!("stderr is empty"));
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not a JSON line ({e}): {line}"))
}

// ------------------------------------------------------------------------------------------------- the mock listener

/// One request the mock received.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    /// The request target: path and query.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    /// A header's value (case-insensitive name).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// The path without the query.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }
    /// A query parameter (not decoded).
    pub fn query(&self, name: &str) -> Option<&str> {
        let q = self.target.split_once('?')?.1;
        q.split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
    }
}

/// What the mock does with a request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// Answer, then close the connection.
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Read the request, then close without an answer (a lost response).
    Drop,
    /// Read the request, wait, then close without an answer.
    Hang(Duration),
    /// Answer 200 with headers that promise a 64-byte JSON body, send its first 9 bytes, then wait and close.
    BodyHangs(Duration),
    /// Answer 200 with headers that promise a 64-byte JSON body, send its first 9 bytes, then close.
    BodyDrops,
}

impl Reply {
    /// A JSON answer.
    pub fn json(status: u16, body: &Value) -> Reply {
        Reply::Respond {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
        }
    }
    /// A raw answer of `content_type`.
    pub fn raw(status: u16, content_type: &str, body: &[u8]) -> Reply {
        Reply::Respond {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.to_vec(),
        }
    }
    /// A body-less answer.
    pub fn empty(status: u16) -> Reply {
        Reply::Respond {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    /// The §7.1 error envelope.
    pub fn error(status: u16, code: &str) -> Reply {
        Reply::json(
            status,
            &serde_json::json!({"error": {"code": code, "message": format!("mock {code}")}, "request_id": "mock-err"}),
        )
    }
    /// Adds a header.
    pub fn with(mut self, name: &str, value: &str) -> Reply {
        if let Reply::Respond { headers, .. } = &mut self {
            headers.push((name.into(), value.into()));
        }
        self
    }
}

type Handler = dyn Fn(&Recorded, usize) -> Reply + Send + Sync;

struct MockState {
    requests: Mutex<Vec<Recorded>>,
    connections: AtomicUsize,
    stop: AtomicBool,
    handler: Box<Handler>,
}

/// A scripted HTTP listener on 127.0.0.1. The handler gets each request and its index (0-based, in arrival order).
pub struct Mock {
    /// `http://127.0.0.1:<port>`.
    pub url: String,
    pub port: u16,
    host: std::net::IpAddr,
    state: Arc<MockState>,
}

impl Mock {
    /// Binds 127.0.0.1:0 and serves.
    pub fn start(handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock");
        Mock::serve(listener, handler)
    }

    /// Binds `ip`:0 and serves; `url` then names that address.
    pub fn start_on(
        ip: std::net::IpAddr,
        handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    ) -> Mock {
        let listener = TcpListener::bind((ip, 0)).expect("bind the mock");
        let mut mock = Mock::serve(listener, handler);
        mock.url = format!("http://{ip}:{}", mock.port);
        mock.host = ip;
        mock
    }

    /// Serves on an already bound listener.
    pub fn serve(
        listener: TcpListener,
        handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    ) -> Mock {
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(MockState {
            requests: Mutex::new(Vec::new()),
            connections: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            handler: Box::new(handler),
        });
        let s = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                s.connections.fetch_add(1, Ordering::SeqCst);
                let s = Arc::clone(&s);
                std::thread::spawn(move || serve_one(stream, &s));
            }
        });
        Mock {
            url: format!("http://127.0.0.1:{port}"),
            port,
            host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            state,
        }
    }

    /// A mock that answers every request with `reply`.
    pub fn always(reply: Reply) -> Mock {
        Mock::start(move |_, _| reply.clone())
    }

    /// Every request so far.
    pub fn requests(&self) -> Vec<Recorded> {
        self.state.requests.lock().unwrap().clone()
    }

    /// Accepted connections so far.
    pub fn connections(&self) -> usize {
        self.state.connections.load(Ordering::SeqCst)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect((self.host, self.port));
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Recorded> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _): &&(String, String)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.clone())
    };
    let mut body = Vec::new();
    if let Some(len) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(len, 0);
        reader.read_exact(&mut body).ok()?;
    } else if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let n = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            let mut chunk = vec![0; n + 2];
            reader.read_exact(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    }
    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        302 => "Found",
        404 => "Not Found",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn serve_one(stream: TcpStream, state: &MockState) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let Some(req) = read_request(&mut reader) else {
        return;
    };
    let index = {
        let mut list = state.requests.lock().unwrap();
        list.push(req.clone());
        list.len() - 1
    };
    let mut stream = stream;
    match (state.handler)(&req, index) {
        Reply::Respond {
            status,
            headers,
            body,
        } => {
            let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
            let has = |n: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
            if !has("x-request-id") {
                head.push_str(&format!("X-Request-Id: mock-{index}\r\n"));
            }
            if !has("quire-api-version") {
                head.push_str("Quire-Api-Version: 1.0\r\n");
            }
            for (k, v) in &headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            let send_body = status != 204 && status != 304 && req.method != "HEAD";
            if status != 204 {
                head.push_str(&format!(
                    "Content-Length: {}\r\n",
                    if send_body { body.len() } else { 0 }
                ));
            }
            head.push_str("Connection: close\r\n\r\n");
            let _ = stream.write_all(head.as_bytes());
            if send_body {
                let _ = stream.write_all(&body);
            }
            let _ = stream.flush();
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Drop => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Hang(d) => {
            std::thread::sleep(d);
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::BodyHangs(d) => {
            write_partial_body(&mut stream, index);
            std::thread::sleep(d);
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::BodyDrops => {
            write_partial_body(&mut stream, index);
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// A 200 head that promises a 64-byte JSON body, and its first 9 bytes.
fn write_partial_body(stream: &mut TcpStream, index: usize) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nX-Request-Id: mock-{index}\r\nQuire-Api-Version: 1.0\r\n\
         Content-Type: application/json\r\nContent-Length: 64\r\nConnection: close\r\n\r\n{{\"items\":"
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.flush();
}

// ------------------------------------------------------------------------------------------------ the raw listener

/// A placeholder CA bundle: one PEM block, which the CLI accepts as a certificate. Nothing is verified against it
/// before a server answers the ClientHello, so it serves handshakes that stall or fail.
pub const PLACEHOLDER_CA: &[u8] =
    b"-----BEGIN CERTIFICATE-----\nMIIBAA==\n-----END CERTIFICATE-----\n";

/// A TCP listener on 127.0.0.1 that speaks no HTTP. Each connection gets `greeting` (when not empty) and is then
/// held open until the client closes it. For an https URL, no greeting stalls the TLS handshake and a plain-text
/// greeting fails it; either way the request is never written.
pub struct RawListener {
    pub port: u16,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl RawListener {
    pub fn start(greeting: &'static [u8]) -> RawListener {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the raw listener");
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, halt) = (Arc::clone(&connections), Arc::clone(&stop));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                count.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    if !greeting.is_empty() {
                        let _ = stream.write_all(greeting);
                        let _ = stream.flush();
                    }
                    // Hold the connection until the client gives up.
                    let mut sink = [0u8; 4096];
                    while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
                });
            }
        });
        RawListener {
            port,
            connections,
            stop,
        }
    }

    /// Accepted connections so far.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for RawListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}
