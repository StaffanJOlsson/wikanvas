//! Configuration from QUIRE_* environment variables, and the derived public URL (owner: W2).
//!
//! Every variable of ARCHITECTURE §13.3 and §8 is parsed here, once, at startup: a malformed value stops the server
//! with a message naming the variable. Compose passes unset variables as empty strings, so an empty value counts as
//! unset for every variable (§8). The behaviour behind most of them (CORS, HSTS, backups, limits) lives in the modules
//! that use them.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use quire_api::Limits;

/// The default listen address of a bare `quire-server serve` (the image sets `QUIRE_LISTEN=0.0.0.0:8080`).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8080";
/// Every body without a route-specific limit (D33).
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;
/// A single Yjs update POST (D33).
pub const MAX_UPDATE_BYTES: u64 = 16 * 1024 * 1024;
/// A Markdown document PUT (D33).
pub const MAX_MARKDOWN_BYTES: u64 = 1024 * 1024;
/// A ProseMirror JSON document PUT (D33).
pub const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// JSON nesting depth (D33).
pub const JSON_DEPTH: u32 = 100;
/// The longest search query `q` (§7.1).
pub const Q_MAX_CHARS: u32 = 500;
/// The largest `limit` of a paginated list (§7.1).
pub const PAGE_MAX: u32 = 100;
/// The per-token request burst (D47).
pub const TOKEN_BURST: u32 = 120;
/// The request timeout (§8).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The body deadline of uploads and board PUTs (§8).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// `QUIRE_TLS`: how Caddy terminates HTTPS in front of Quire (D57). Only `acme` sends HSTS (§11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsMode {
    /// Caddy's local CA (default): `localhost` or LAN names. No HSTS.
    Internal,
    /// Public certificates through ACME. HSTS on HTTPS requests.
    Acme,
}

/// `QUIRE_SYNC`: SQLite's `synchronous` PRAGMA (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    Normal,
    Full,
}

/// A URL scheme Quire can be reached under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    /// The scheme's default port.
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// An origin `scheme://host[:port]`: the public URL (§8) or a CORS origin (D48). The host is lowercase, IPv6 hosts
/// keep their brackets, and a default port is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub scheme: Scheme,
    pub host: String,
    /// `None` when it is the scheme's default.
    pub port: Option<u16>,
}

impl Origin {
    /// Parses `scheme://host[:port]` with an optional trailing `/` (and nothing else after the authority).
    pub fn parse(s: &str) -> Result<Origin, String> {
        let (scheme, rest) = if let Some(rest) = s.strip_prefix("https://") {
            (Scheme::Https, rest)
        } else if let Some(rest) = s.strip_prefix("http://") {
            (Scheme::Http, rest)
        } else {
            return Err(format!("{s:?} is not an http:// or https:// URL"));
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains(['/', '?', '#', '@']) {
            return Err(format!(
                "{s:?} must be scheme://host[:port], with no path, query or user"
            ));
        }
        let (host, port) =
            split_host_port(authority).ok_or_else(|| format!("{s:?} has no valid host[:port]"))?;
        let port = port.filter(|p| *p != scheme.default_port());
        Ok(Origin { scheme, host, port })
    }

    /// `host[:port]`, the port only when it is not the scheme's default.
    pub fn host_port(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }

    /// The effective port.
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(self.scheme.default_port())
    }

    /// Whether `host` (lowercase) and `port` (`None`: the default of `scheme`) name this origin's authority.
    pub fn matches_authority(&self, host: &str, port: Option<u16>, scheme: Scheme) -> bool {
        self.host == host && self.port_or_default() == port.unwrap_or(scheme.default_port())
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme.as_str(), self.host_port())
    }
}

/// Splits `host[:port]` (IPv6 in brackets), lowercasing the host. `None` for an empty host or a bad port.
pub fn split_host_port(s: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &s[..end + 2];
        let after = &rest[end + 1..];
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?),
        };
        (host, port)
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (s, None),
        }
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    if !host.starts_with('[') && host.contains(':') {
        return None;
    }
    let port = match port {
        None => None,
        Some(p) => Some(p.parse::<u16>().ok()?),
    };
    Some((host.to_ascii_lowercase(), port))
}

/// An entry of `QUIRE_ALLOWED_HOSTS`: a host with any port, or an exact `host:port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPattern {
    pub host: String,
    pub port: Option<u16>,
}

impl HostPattern {
    pub fn matches(&self, host: &str, port: Option<u16>) -> bool {
        self.host == host && (self.port.is_none() || self.port == port)
    }
}

/// An IP address or CIDR block of `QUIRE_TRUSTED_PROXIES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl IpNet {
    /// Parses `a.b.c.d`, `a.b.c.d/n`, an IPv6 address or `v6/n`.
    pub fn parse(s: &str) -> Result<IpNet, String> {
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = ip
            .parse()
            .map_err(|_| format!("{s:?} is not an IP address or CIDR block"))?;
        let addr = canonical_ip(addr);
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("{s:?} has a bad prefix length"))?,
        };
        Ok(IpNet { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical_ip(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) as the IPv4 address it carries; anything else unchanged.
pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// A variable that does not parse; startup stops with this message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub var: &'static str,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.var, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// The server's configuration (ARCHITECTURE §13.3, §8, D33, D47, D48, D57).
#[derive(Clone, Debug)]
pub struct Config {
    /// `QUIRE_DATA_DIR` (default `./data`; the image sets `/data`): `quire.db`, `files/`, `tmp/`, `backups/`.
    pub data_dir: PathBuf,
    /// `QUIRE_LISTEN` (default `127.0.0.1:8080`).
    pub listen: SocketAddr,
    /// The public URL (§8): `QUIRE_PUBLIC_URL`, else `https://QUIRE_DOMAIN[:QUIRE_HTTPS_PORT]`, else
    /// `http://localhost:<QUIRE_LISTEN port>`.
    pub public_url: Origin,
    /// `QUIRE_DOMAIN`, lowercase: allowed as `Host` with any port.
    pub domain: Option<String>,
    /// `QUIRE_TLS` (default `internal`).
    pub tls: TlsMode,
    /// `QUIRE_HTTPS_PORT` (default 443).
    pub https_port: u16,
    /// `QUIRE_ALLOWED_HOSTS`.
    pub allowed_hosts: Vec<HostPattern>,
    /// `QUIRE_TRUSTED_PROXIES`: peers whose `X-Forwarded-For` / `-Proto` count.
    pub trusted_proxies: Vec<IpNet>,
    /// `QUIRE_CORS_ORIGINS` (D48); empty = CORS off.
    pub cors_origins: Vec<Origin>,
    /// `QUIRE_SETUP_TOKEN`: a fixed setup code instead of a generated one (D19).
    pub setup_token: Option<String>,
    /// `QUIRE_BACKUP_INTERVAL` (default 24h).
    pub backup_interval: Duration,
    /// `QUIRE_BACKUP_KEEP` (default 7).
    pub backup_keep: u32,
    /// `QUIRE_MAX_UPLOAD_MB` (default 25).
    pub max_upload_mb: u64,
    /// `QUIRE_MAX_BOARD_MB` (default 50).
    pub max_board_mb: u64,
    /// `QUIRE_MAX_DOC_MB` (default 16).
    pub max_doc_mb: u64,
    /// `QUIRE_MIN_FREE_MB` (default 1024; capped at 10% of the filesystem, D33).
    pub min_free_mb: u64,
    /// `QUIRE_TOKEN_RATE`: requests per minute per token (default 600; burst [`TOKEN_BURST`]).
    pub token_rate_per_min: u32,
    /// `QUIRE_TOKEN_BYTES_PER_HOUR` (default 1 GiB).
    pub token_bytes_per_hour: u64,
    /// `QUIRE_WORKER_THREADS` (default 2, D25).
    pub worker_threads: usize,
    /// `QUIRE_MAX_BLOCKING_THREADS` (default 16, D25).
    pub max_blocking_threads: usize,
    /// `QUIRE_SYNC` (default `NORMAL`).
    pub sync: SyncMode,
    /// `QUIRE_TEST_ARGON2=1`: cheap argon2 parameters, for tests only.
    pub test_argon2: bool,
    /// Tests only (no variable): the limiters and "now" run on a fake clock (`AppState::clock`).
    pub fake_clock: bool,
    /// Every request but uploads and board PUTs: 30 s, then 408 `request_timeout` (no variable).
    pub request_timeout: Duration,
    /// Uploads and board PUTs: 120 s (no variable).
    pub upload_timeout: Duration,
}

impl Config {
    /// Reads the process environment.
    pub fn from_env() -> Result<Config, ConfigError> {
        Config::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the variables through `get` (tests pass a map). Empty values count as unset.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let var = |name: &str| {
            get(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let data_dir = var("QUIRE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        let listen: SocketAddr = parse_with(&var, "QUIRE_LISTEN", DEFAULT_LISTEN, |s| {
            s.parse()
                .map_err(|_| format!("{s:?} is not an address:port"))
        })?;
        let domain = match var("QUIRE_DOMAIN") {
            None => None,
            Some(d) => {
                let (host, port) = split_host_port(&d).ok_or_else(|| ConfigError {
                    var: "QUIRE_DOMAIN",
                    message: format!("{d:?} is not a host name"),
                })?;
                if port.is_some() {
                    return Err(ConfigError {
                        var: "QUIRE_DOMAIN",
                        message: format!("{d:?}: give the port in QUIRE_HTTPS_PORT"),
                    });
                }
                Some(host)
            }
        };
        let tls = parse_with(&var, "QUIRE_TLS", "internal", |s| match s {
            "internal" => Ok(TlsMode::Internal),
            "acme" => Ok(TlsMode::Acme),
            _ => Err(format!("{s:?}: expected internal or acme")),
        })?;
        let https_port: u16 = parse_num(&var, "QUIRE_HTTPS_PORT", 443)?;
        if https_port == 0 {
            return Err(ConfigError {
                var: "QUIRE_HTTPS_PORT",
                message: "must be 1-65535".into(),
            });
        }
        let public_url = match var("QUIRE_PUBLIC_URL") {
            Some(u) => Origin::parse(&u).map_err(|message| ConfigError {
                var: "QUIRE_PUBLIC_URL",
                message,
            })?,
            None => match &domain {
                Some(d) => Origin {
                    scheme: Scheme::Https,
                    host: d.clone(),
                    port: (https_port != 443).then_some(https_port),
                },
                None => Origin {
                    scheme: Scheme::Http,
                    host: "localhost".into(),
                    port: (listen.port() != 80).then_some(listen.port()),
                },
            },
        };
        let allowed_hosts = parse_list(&var, "QUIRE_ALLOWED_HOSTS", |s| {
            split_host_port(s)
                .map(|(host, port)| HostPattern { host, port })
                .ok_or_else(|| format!("{s:?} is not host or host:port"))
        })?;
        let trusted_proxies = parse_list(&var, "QUIRE_TRUSTED_PROXIES", IpNet::parse)?;
        let cors_origins = parse_list(&var, "QUIRE_CORS_ORIGINS", |s| {
            if s == "*" {
                return Err("* is not allowed: list exact origins (D48)".into());
            }
            let origin = Origin::parse(s)?;
            if s.ends_with('/') {
                return Err(format!("{s:?}: an origin has no trailing /"));
            }
            Ok(origin)
        })?;
        let backup_interval = parse_with(&var, "QUIRE_BACKUP_INTERVAL", "24h", parse_duration)?;
        Ok(Config {
            data_dir,
            listen,
            public_url,
            domain,
            tls,
            https_port,
            allowed_hosts,
            trusted_proxies,
            cors_origins,
            setup_token: var("QUIRE_SETUP_TOKEN"),
            backup_interval,
            backup_keep: parse_num(&var, "QUIRE_BACKUP_KEEP", 7)?,
            max_upload_mb: parse_positive(&var, "QUIRE_MAX_UPLOAD_MB", 25)?,
            max_board_mb: parse_positive(&var, "QUIRE_MAX_BOARD_MB", 50)?,
            max_doc_mb: parse_positive(&var, "QUIRE_MAX_DOC_MB", 16)?,
            min_free_mb: parse_num(&var, "QUIRE_MIN_FREE_MB", 1024)?,
            token_rate_per_min: parse_positive(&var, "QUIRE_TOKEN_RATE", 600)?,
            token_bytes_per_hour: parse_positive(
                &var,
                "QUIRE_TOKEN_BYTES_PER_HOUR",
                1024 * 1024 * 1024,
            )?,
            worker_threads: parse_positive(&var, "QUIRE_WORKER_THREADS", 2)?,
            max_blocking_threads: parse_positive(&var, "QUIRE_MAX_BLOCKING_THREADS", 16)?,
            sync: parse_with(&var, "QUIRE_SYNC", "NORMAL", |s| {
                match s.to_ascii_uppercase().as_str() {
                    "NORMAL" => Ok(SyncMode::Normal),
                    "FULL" => Ok(SyncMode::Full),
                    _ => Err(format!("{s:?}: expected NORMAL or FULL")),
                }
            })?,
            test_argon2: parse_with(&var, "QUIRE_TEST_ARGON2", "0", |s| match s {
                "1" | "true" => Ok(true),
                "0" | "false" => Ok(false),
                _ => Err(format!("{s:?}: expected 1 or 0")),
            })?,
            fake_clock: false,
            request_timeout: REQUEST_TIMEOUT,
            upload_timeout: UPLOAD_TIMEOUT,
        })
    }

    /// The configuration of a test server: `data_dir`, defaults otherwise, cheap argon2.
    pub fn for_tests(data_dir: impl Into<PathBuf>) -> Config {
        let mut config = Config::from_lookup(|_| None).expect("the defaults parse");
        config.data_dir = data_dir.into();
        config.test_argon2 = true;
        config
    }

    /// The SQLite database file.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("quire.db")
    }

    /// The blob store root (`files/aa/bb/<sha256>`, D26).
    pub fn files_dir(&self) -> PathBuf {
        self.data_dir.join("files")
    }

    /// Staging for uploads and exports (never `/tmp`, a small tmpfs).
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    /// Backups (`quire-*.db`, `pre-migrate-*.db`).
    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    /// `limits` of `GET /api/v1/config` (§7.2).
    pub fn limits(&self) -> Limits {
        Limits {
            max_upload_mb: self.max_upload_mb,
            max_board_mb: self.max_board_mb,
            max_doc_mb: self.max_doc_mb,
            max_update_bytes: MAX_UPDATE_BYTES,
            max_markdown_bytes: MAX_MARKDOWN_BYTES,
            max_json_bytes: MAX_JSON_BYTES,
            max_body_bytes: MAX_BODY_BYTES,
            json_depth: JSON_DEPTH,
            q_max_chars: Q_MAX_CHARS,
            page_max: PAGE_MAX,
            token_rate_per_min: self.token_rate_per_min,
            token_burst: TOKEN_BURST,
            token_bytes_per_hour: self.token_bytes_per_hour,
        }
    }
}

/// The URL `quire-server healthcheck` probes by default: `QUIRE_LISTEN`'s port on 127.0.0.1 when the listen address
/// is unspecified (`0.0.0.0` in the image, or `[::]`); a specific address (`127.0.0.1`, `[::1]`, a LAN address) is
/// probed as it is, because a server bound to one address does not answer on another.
pub fn healthcheck_url(listen: SocketAddr) -> String {
    let ip = match listen.ip() {
        ip if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        ip => ip,
    };
    format!("http://{}/healthz", SocketAddr::new(ip, listen.port()))
}

fn parse_with<T>(
    var: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<T, ConfigError> {
    let value = var(name).unwrap_or_else(|| default.to_owned());
    parse(&value).map_err(|message| ConfigError { var: name, message })
}

fn parse_num<T: std::str::FromStr + ToString>(
    var: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError> {
    parse_with(var, name, &default.to_string(), |s| {
        s.parse()
            .map_err(|_| format!("{s:?} is not a non-negative integer"))
    })
}

fn parse_positive<T: std::str::FromStr + ToString + PartialOrd + Default>(
    var: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError> {
    let value: T = parse_num(var, name, default)?;
    if value <= T::default() {
        return Err(ConfigError {
            var: name,
            message: "must be at least 1".into(),
        });
    }
    Ok(value)
}

fn parse_list<T>(
    var: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<Vec<T>, ConfigError> {
    let Some(value) = var(name) else {
        return Ok(Vec::new());
    };
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| parse(s).map_err(|message| ConfigError { var: name, message }))
        .collect()
}

/// `90s`, `30m`, `24h` or `7d`.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let bad = || format!("{s:?}: expected a number with s, m, h or d, e.g. 24h");
    let unit = s.chars().last().ok_or_else(bad)?;
    let secs: u64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return Err(bad()),
    };
    let n: u64 = s[..s.len() - 1].parse().map_err(|_| bad())?;
    if n == 0 {
        return Err(bad());
    }
    n.checked_mul(secs).map(Duration::from_secs).ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = config(&[]).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(c.public_url.to_string(), "http://localhost:8080");
        assert_eq!(c.tls, TlsMode::Internal);
        assert_eq!(c.https_port, 443);
        assert_eq!(c.backup_interval, Duration::from_secs(86_400));
        assert_eq!(
            (c.backup_keep, c.max_upload_mb, c.max_board_mb),
            (7, 25, 50)
        );
        assert_eq!((c.max_doc_mb, c.min_free_mb), (16, 1024));
        assert_eq!(
            (c.token_rate_per_min, c.token_bytes_per_hour),
            (600, 1 << 30)
        );
        assert_eq!((c.worker_threads, c.max_blocking_threads), (2, 16));
        assert_eq!(c.sync, SyncMode::Normal);
        assert!(!c.test_argon2 && c.cors_origins.is_empty() && c.trusted_proxies.is_empty());
        assert_eq!(c.limits().token_burst, 120);
    }

    #[test]
    fn empty_values_count_as_unset() {
        let c = config(&[
            ("QUIRE_PUBLIC_URL", ""),
            ("QUIRE_DOMAIN", ""),
            ("QUIRE_CORS_ORIGINS", ""),
            ("QUIRE_TRUSTED_PROXIES", " "),
            ("QUIRE_TLS", ""),
        ])
        .unwrap();
        assert_eq!(c.public_url.to_string(), "http://localhost:8080");
        assert!(c.domain.is_none() && c.cors_origins.is_empty());
    }

    #[test]
    fn public_url_derived_from_domain_and_port() {
        let c = config(&[("QUIRE_DOMAIN", "Quire.LAN")]).unwrap();
        assert_eq!(c.public_url.to_string(), "https://quire.lan");
        let c = config(&[("QUIRE_DOMAIN", "quire.lan"), ("QUIRE_HTTPS_PORT", "8443")]).unwrap();
        assert_eq!(c.public_url.to_string(), "https://quire.lan:8443");
        let c = config(&[
            ("QUIRE_DOMAIN", "quire.lan"),
            ("QUIRE_PUBLIC_URL", "http://quire:8080/"),
        ])
        .unwrap();
        assert_eq!(c.public_url.to_string(), "http://quire:8080");
        let c = config(&[("QUIRE_PUBLIC_URL", "https://wiki.example.org:443")]).unwrap();
        assert_eq!(c.public_url.host_port(), "wiki.example.org");
        let c = config(&[("QUIRE_LISTEN", "0.0.0.0:9000")]).unwrap();
        assert_eq!(c.public_url.to_string(), "http://localhost:9000");
        assert!(config(&[("QUIRE_PUBLIC_URL", "https://x.org/wiki")]).is_err());
        assert!(config(&[("QUIRE_PUBLIC_URL", "ftp://x.org")]).is_err());
        assert!(config(&[("QUIRE_DOMAIN", "x.org:443")]).is_err());
    }

    #[test]
    fn tls_and_sync_values() {
        assert_eq!(config(&[("QUIRE_TLS", "acme")]).unwrap().tls, TlsMode::Acme);
        let err = config(&[("QUIRE_TLS", "off")]).unwrap_err();
        assert_eq!(err.var, "QUIRE_TLS");
        assert_eq!(
            config(&[("QUIRE_SYNC", "full")]).unwrap().sync,
            SyncMode::Full
        );
        assert!(config(&[("QUIRE_SYNC", "OFF")]).is_err());
    }

    #[test]
    fn cors_star_and_malformed_origins_refused() {
        assert_eq!(
            config(&[("QUIRE_CORS_ORIGINS", "*")]).unwrap_err().var,
            "QUIRE_CORS_ORIGINS"
        );
        assert!(config(&[("QUIRE_CORS_ORIGINS", "https://a.org/")]).is_err());
        assert!(config(&[("QUIRE_CORS_ORIGINS", "a.org")]).is_err());
        let c = config(&[("QUIRE_CORS_ORIGINS", "https://a.org, http://b.lan:3000")]).unwrap();
        let shown: Vec<String> = c.cors_origins.iter().map(ToString::to_string).collect();
        assert_eq!(shown, ["https://a.org", "http://b.lan:3000"]);
    }

    #[test]
    fn trusted_proxies_and_allowed_hosts() {
        let c = config(&[
            (
                "QUIRE_TRUSTED_PROXIES",
                "172.31.240.2, 10.0.0.0/8, fd00::/8",
            ),
            ("QUIRE_ALLOWED_HOSTS", "quire, wiki.lan:8443, [::1]:9"),
        ])
        .unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let trusted = |s: &str| c.trusted_proxies.iter().any(|n| n.contains(ip(s)));
        assert!(trusted("172.31.240.2") && trusted("::ffff:172.31.240.2"));
        assert!(!trusted("172.31.240.3"));
        assert!(trusted("10.200.0.1") && trusted("fd12::1") && !trusted("fe80::1"));
        let allowed = |h: &str, p: Option<u16>| c.allowed_hosts.iter().any(|a| a.matches(h, p));
        assert!(allowed("quire", Some(8080)) && allowed("quire", None));
        assert!(allowed("wiki.lan", Some(8443)) && !allowed("wiki.lan", Some(443)));
        assert!(allowed("[::1]", Some(9)));
        assert!(config(&[("QUIRE_TRUSTED_PROXIES", "10.0.0.0/33")]).is_err());
        assert!(config(&[("QUIRE_TRUSTED_PROXIES", "caddy")]).is_err());
    }

    #[test]
    fn numbers_and_durations() {
        let c = config(&[
            ("QUIRE_BACKUP_INTERVAL", "90m"),
            ("QUIRE_MAX_UPLOAD_MB", "5"),
        ])
        .unwrap();
        assert_eq!(c.backup_interval, Duration::from_secs(5400));
        assert_eq!(c.max_upload_mb, 5);
        assert!(config(&[("QUIRE_BACKUP_INTERVAL", "24")]).is_err());
        assert!(config(&[("QUIRE_BACKUP_INTERVAL", "0h")]).is_err());
        assert!(config(&[("QUIRE_MAX_UPLOAD_MB", "0")]).is_err());
        assert!(config(&[("QUIRE_MAX_UPLOAD_MB", "-1")]).is_err());
        assert!(config(&[("QUIRE_WORKER_THREADS", "two")]).is_err());
    }

    #[test]
    fn healthcheck_url_uses_loopback() {
        let url = |s: &str| healthcheck_url(s.parse().unwrap());
        assert_eq!(url("0.0.0.0:8080"), "http://127.0.0.1:8080/healthz");
        assert_eq!(url("[::]:8080"), "http://127.0.0.1:8080/healthz");
        assert_eq!(url("127.0.0.1:9"), "http://127.0.0.1:9/healthz");
        assert_eq!(url("[::1]:8080"), "http://[::1]:8080/healthz");
        assert_eq!(url("127.0.0.2:8080"), "http://127.0.0.2:8080/healthz");
        assert_eq!(url("10.1.2.3:80"), "http://10.1.2.3:80/healthz");
    }
}
