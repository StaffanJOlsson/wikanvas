//! Configuration: flags, environment and the profile file; URL and token resolved as a pair (owner: W8)
//!
//! §7.6 "Configuration". With `--profile P`: URL = `--url`, else P's `url`; token = `--token-stdin` or
//! `--token-file`, else P's token only when the URL is P's; `QUIRE_URL`, `QUIRE_TOKEN` and `QUIRE_TOKEN_FILE` are
//! ignored. Otherwise: URL = `--url`, `QUIRE_URL`, the profile named by `QUIRE_PROFILE` or `default_profile`, or
//! (inside the server container) `QUIRE_LISTEN`'s port on 127.0.0.1; token = `--token-stdin`, `--token-file`,
//! `QUIRE_TOKEN`, `QUIRE_TOKEN_FILE`, then that profile's token only when the URL is the profile's. A URL counts as
//! the profile's when it came from the profile or names the same origin. A profile token that would pair with a
//! foreign URL is exit 3 ("token not bound to this URL"). Every token is checked offline (`token::check`) before any
//! request.
//!
//! The profile file is `QUIRE_CONFIG`, else `$XDG_CONFIG_HOME/quire/config.toml` (`~/.config/quire/config.toml`).
//! A file that holds a `token` and is readable by group or others is refused; a group-readable token file only
//! warns. The CLI creates the directory 0700 and writes the file as a 0600 temporary file renamed into place.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use crate::args::GlobalArgs;
use crate::error::Failure;
use crate::output;

/// The environment variables the CLI reads (an empty value counts as unset).
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: HashMap<String, String>,
}

const NAMES: &[&str] = &[
    "QUIRE_URL",
    "QUIRE_TOKEN",
    "QUIRE_TOKEN_FILE",
    "QUIRE_PROFILE",
    "QUIRE_CONFIG",
    "QUIRE_LISTEN",
    "QUIRE_CA_FILE",
    "QUIRE_ALLOW_HTTP",
    "SSL_CERT_FILE",
];

impl Env {
    /// The process environment.
    pub fn from_process() -> Env {
        let vars = NAMES
            .iter()
            .filter_map(|&name| std::env::var(name).ok().map(|v| (name.to_owned(), v)))
            .collect();
        Env { vars }
    }

    /// A fixed environment (tests).
    #[cfg(test)]
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Env {
        Env {
            vars: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    /// The variable's value, `None` when unset or empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

// ------------------------------------------------------------------------------------------------- origins

/// `http` or `https`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

/// A Quire URL: `scheme://host[:port]`, with no path, query or user info.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub scheme: Scheme,
    /// Lowercased; an IPv6 address keeps its brackets.
    pub host: String,
    /// `None` for the scheme's default port.
    pub port: Option<u16>,
}

impl Origin {
    /// Parses `scheme://host[:port][/]`.
    pub fn parse(input: &str) -> Result<Origin, String> {
        let s = input.trim();
        let (scheme, rest) = if let Some(rest) = strip_prefix_ci(s, "https://") {
            (Scheme::Https, rest)
        } else if let Some(rest) = strip_prefix_ci(s, "http://") {
            (Scheme::Http, rest)
        } else {
            return Err(format!(
                "{input:?} is not a Quire URL: it starts with https:// (or http://)"
            ));
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.is_empty() {
            return Err(format!("{input:?} has no host"));
        }
        if authority.contains(['/', '?', '#', '@', '\\']) || authority.contains(char::is_whitespace)
        {
            return Err(format!(
                "{input:?} is not a Quire URL: use scheme://host[:port], with no path, query or user"
            ));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (addr, after) = v6
                .split_once(']')
                .ok_or_else(|| format!("{input:?}: an unclosed [ in the host"))?;
            if addr.parse::<std::net::Ipv6Addr>().is_err() {
                return Err(format!("{input:?}: [{addr}] is not an IPv6 address"));
            }
            let port = match after {
                "" => None,
                p => Some(
                    p.strip_prefix(':')
                        .ok_or_else(|| format!("{input:?}: text after the host"))?,
                ),
            };
            (format!("[{}]", addr.to_ascii_lowercase()), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_ascii_lowercase(), Some(p)),
                None => (authority.to_ascii_lowercase(), None),
            }
        };
        if host.is_empty() || host.contains(':') && !host.starts_with('[') {
            return Err(format!("{input:?} has no valid host"));
        }
        let port = match port {
            None => None,
            Some(p) => {
                let n: u16 = p
                    .parse()
                    .ok()
                    .filter(|n| *n != 0)
                    .ok_or_else(|| format!("{input:?}: {p:?} is not a port"))?;
                let default = match scheme {
                    Scheme::Http => 80,
                    Scheme::Https => 443,
                };
                (n != default).then_some(n)
            }
        };
        Ok(Origin { scheme, host, port })
    }

    /// The base the request paths are appended to: `scheme://host[:port]`, no trailing slash.
    pub fn base(&self) -> String {
        let scheme = match self.scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
        };
        match self.port {
            Some(port) => format!("{scheme}://{}:{port}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }

    /// `localhost`, 127.0.0.0/8 or `[::1]`: plain HTTP is always allowed there.
    pub fn is_loopback(&self) -> bool {
        if self.host == "localhost" || self.host == "[::1]" {
            return true;
        }
        self.host
            .parse::<Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

/// The default URL inside the server container: `QUIRE_LISTEN`'s port on 127.0.0.1 when it listens on every
/// address (`0.0.0.0` in the image), otherwise the address itself (the rule of `quire-server healthcheck`).
fn listen_origin(listen: &str) -> Result<Origin, String> {
    let addr: SocketAddr = listen
        .parse()
        .map_err(|_| format!("QUIRE_LISTEN={listen:?} is not an address such as 0.0.0.0:8080"))?;
    let ip = match addr.ip() {
        ip if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        ip => ip,
    };
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    Ok(Origin {
        scheme: Scheme::Http,
        host,
        port: (addr.port() != 80).then_some(addr.port()),
    })
}

// ------------------------------------------------------------------------------------------------- the profile file

/// The profile file (`config.toml`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProfileFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

/// `[profiles.NAME]`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Profile {
    pub url: Option<String>,
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
    pub ca_file: Option<PathBuf>,
    pub allow_http: Option<bool>,
}

/// The profile file's path: `QUIRE_CONFIG`, else `$XDG_CONFIG_HOME/quire/config.toml`.
pub fn config_path(env: &Env) -> Result<PathBuf, Failure> {
    if let Some(path) = env.get("QUIRE_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    use etcetera::BaseStrategy as _;
    let base = etcetera::choose_base_strategy().map_err(|e| {
        Failure::config(format!(
            "no config directory ({e}): set QUIRE_CONFIG or HOME"
        ))
    })?;
    Ok(base.config_dir().join("quire").join("config.toml"))
}

/// Whether group or others may read the file.
fn readable_by_others(meta: &fs::Metadata) -> bool {
    meta.permissions().mode() & 0o044 != 0
}

/// Reads the profile file; a missing file is an empty one. A file that holds a `token` and is readable by group or
/// others is refused (exit 3).
pub fn load(path: &Path) -> Result<ProfileFile, Failure> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProfileFile::default()),
        Err(e) => {
            return Err(Failure::config(format!(
                "cannot read the profile file {}: {e}",
                path.display()
            )));
        }
    };
    let file: ProfileFile = toml::from_str(&text).map_err(|e| {
        Failure::config(format!(
            "the profile file {} is not valid: {}",
            path.display(),
            e.message()
        ))
    })?;
    if file.profiles.values().any(|p| p.token.is_some()) {
        let meta = fs::metadata(path)
            .map_err(|e| Failure::config(format!("cannot stat {}: {e}", path.display())))?;
        if readable_by_others(&meta) {
            return Err(Failure::config(format!(
                "the profile file {} holds a token and is readable by group or others: run chmod 600 on it",
                path.display()
            )));
        }
    }
    Ok(file)
}

/// How `auth login` stores the token in the profile.
pub enum StoredToken {
    /// `token = "qpat_…"` (from `--token-stdin`).
    Inline(String),
    /// `token_file = "/abs/path"` (from `--token-file`).
    File(PathBuf),
}

fn read_table(path: &Path) -> Result<toml::Table, Failure> {
    match fs::read_to_string(path) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| {
            Failure::config(format!(
                "the profile file {} is not valid: {}",
                path.display(),
                e.message()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(Failure::config(format!(
            "cannot read the profile file {}: {e}",
            path.display()
        ))),
    }
}

fn profile_table<'a>(
    table: &'a mut toml::Table,
    name: &str,
) -> Result<&'a mut toml::Table, Failure> {
    let profiles = table
        .entry("profiles")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| Failure::config("the profile file's `profiles` is not a table"))?;
    profiles
        .entry(name)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| Failure::config(format!("profiles.{name} is not a table")))
}

/// Writes profile `name` with `url` and the token (other keys of that profile and other profiles are kept). Sets
/// `default_profile` when the file has none.
pub fn write_profile(
    path: &Path,
    name: &str,
    url: &Origin,
    token: &StoredToken,
) -> Result<(), Failure> {
    let mut table = read_table(path)?;
    {
        let profile = profile_table(&mut table, name)?;
        profile.insert("url".into(), toml::Value::String(url.base()));
        profile.remove("token");
        profile.remove("token_file");
        match token {
            StoredToken::Inline(t) => {
                profile.insert("token".into(), toml::Value::String(t.clone()));
            }
            StoredToken::File(f) => {
                profile.insert(
                    "token_file".into(),
                    toml::Value::String(f.to_string_lossy().into_owned()),
                );
            }
        }
    }
    if !table.contains_key("default_profile") {
        table.insert("default_profile".into(), toml::Value::String(name.into()));
    }
    write_atomic(path, &table)
}

/// Removes profile `name`'s `token` and `token_file`. Returns whether there was one.
pub fn remove_token(path: &Path, name: &str) -> Result<bool, Failure> {
    let mut table = read_table(path)?;
    let removed = match table
        .get_mut("profiles")
        .and_then(|p| p.as_table_mut())
        .and_then(|p| p.get_mut(name))
        .and_then(|p| p.as_table_mut())
    {
        Some(profile) => {
            let a = profile.remove("token").is_some();
            let b = profile.remove("token_file").is_some();
            a || b
        }
        None => false,
    };
    if removed {
        write_atomic(path, &table)?;
    }
    Ok(removed)
}

/// Creates the directory (0700) and replaces the file with a 0600 temporary file renamed into place.
fn write_atomic(path: &Path, table: &toml::Table) -> Result<(), Failure> {
    let text = toml::to_string(table)
        .map_err(|e| Failure::config(format!("cannot write the profile file: {e}")))?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| Failure::io(&format!("creating {}", dir.display()), &e))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".into());
    let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(Failure::io(&format!("writing {}", path.display()), &e));
    }
    Ok(())
}

/// Writes a secret (and a newline) to `path` as a 0600 file: a temporary file in the same directory, renamed into
/// place, so the secret is never in a file with wider permissions.
pub fn write_secret(path: &Path, secret: &str) -> Result<(), Failure> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "token".into());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(secret.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(Failure::io(&format!("writing {}", path.display()), &e));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------- resolution

/// The URL and token the CLI uses, with where they came from (`quire config show`).
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    /// The profile file.
    pub path: PathBuf,
    /// The profile in play: `--profile`, `QUIRE_PROFILE` or `default_profile`.
    pub profile: Option<String>,
    pub url: Option<Origin>,
    /// `--url`, `QUIRE_URL`, `profile` or `QUIRE_LISTEN`.
    pub url_source: Option<&'static str>,
    pub token: Option<String>,
    /// `--token-stdin`, `--token-file`, `QUIRE_TOKEN`, `QUIRE_TOKEN_FILE` or `profile`.
    pub token_source: Option<&'static str>,
    pub ca_file: Option<PathBuf>,
    /// `QUIRE_ALLOW_HTTP=1`, or the profile's `allow_http = true` (when the URL is the profile's).
    pub allow_http: bool,
}

pub fn read_token_file(path: &Path, what: &str) -> Result<String, Failure> {
    let f = fs::File::open(path).map_err(|e| {
        Failure::config(format!(
            "cannot read the token file {} ({what}): {e}",
            path.display()
        ))
    })?;
    if let Ok(meta) = f.metadata()
        && readable_by_others(&meta)
    {
        output::warn(
            "token_file_permissions",
            &format!(
                "the token file {} is readable by group or others (chmod 600 it unless it is a Docker secret)",
                path.display()
            ),
            Some(json!({ "path": path })),
        );
    }
    let mut text = String::new();
    f.take(64 * 1024).read_to_string(&mut text).map_err(|e| {
        Failure::config(format!(
            "cannot read the token file {}: {e}",
            path.display()
        ))
    })?;
    let token = text.trim().to_owned();
    if token.is_empty() {
        return Err(Failure::config(format!(
            "the token file {} is empty",
            path.display()
        )));
    }
    Ok(token)
}

pub fn check_token(token: &str, source: &str) -> Result<(), Failure> {
    quire_api::token::check(token).map_err(|e| {
        let reason = match e {
            quire_api::token::TokenFormatError::Prefix => "prefix",
            quire_api::token::TokenFormatError::Length => "length",
            quire_api::token::TokenFormatError::Alphabet => "alphabet",
            quire_api::token::TokenFormatError::Checksum => "checksum",
        };
        Failure::cli(
            "cli_config",
            format!("the token from {source} is malformed: {e}"),
            Some(json!({ "reason": reason, "source": source })),
        )
    })
}

/// Resolves the URL and the token (see the module docs). `read_stdin` reads the token for `--token-stdin`.
pub fn resolve(
    g: &GlobalArgs,
    env: &Env,
    read_stdin: impl FnOnce() -> std::io::Result<String>,
) -> Result<Resolved, Failure> {
    let path = config_path(env)?;
    let file = load(&path)?;
    let explicit = g.profile.as_deref();

    let (profile_name, profile) = match explicit {
        Some(name) => match file.profiles.get(name) {
            Some(p) => (Some(name.to_owned()), Some(p)),
            None => {
                return Err(Failure::config(format!(
                    "no profile named {name:?} in {}",
                    path.display()
                )));
            }
        },
        None => {
            let (name, from) = match env.get("QUIRE_PROFILE") {
                Some(n) => (Some(n.to_owned()), "QUIRE_PROFILE"),
                None => (file.default_profile.clone(), "default_profile"),
            };
            match name {
                Some(name) => match file.profiles.get(&name) {
                    Some(p) => (Some(name), Some(p)),
                    None => {
                        return Err(Failure::config(format!(
                            "{from} names {name:?}, which is not a profile in {}",
                            path.display()
                        )));
                    }
                },
                None => (None, None),
            }
        }
    };

    let profile_url = match profile.and_then(|p| p.url.as_deref()) {
        Some(u) => Some(Origin::parse(u).map_err(|e| {
            Failure::config(format!(
                "profile {:?}: url: {e}",
                profile_name.as_deref().unwrap_or_default()
            ))
        })?),
        None => None,
    };

    let parse_url = |u: &str, source: &str| {
        Origin::parse(u).map_err(|e| Failure::config(format!("{source}: {e}")))
    };
    let (url, url_source) = if let Some(u) = g.url.as_deref() {
        (Some(parse_url(u, "--url")?), Some("--url"))
    } else if explicit.is_none()
        && let Some(u) = env.get("QUIRE_URL")
    {
        (Some(parse_url(u, "QUIRE_URL")?), Some("QUIRE_URL"))
    } else if let Some(u) = &profile_url {
        (Some(u.clone()), Some("profile"))
    } else if explicit.is_none()
        && let Some(listen) = env.get("QUIRE_LISTEN")
    {
        (
            Some(listen_origin(listen).map_err(Failure::config)?),
            Some("QUIRE_LISTEN"),
        )
    } else {
        (None, None)
    };
    let bound = match (&url, &profile_url) {
        (Some(u), Some(p)) => u == p,
        _ => false,
    };

    let (token, token_source) = if g.token_stdin {
        let text = read_stdin()
            .map_err(|e| Failure::config(format!("cannot read the token from stdin: {e}")))?;
        let t = text.trim().to_owned();
        if t.is_empty() {
            return Err(Failure::config("--token-stdin: no token on stdin"));
        }
        (Some(t), Some("--token-stdin"))
    } else if let Some(f) = g.token_file.as_deref() {
        (
            Some(read_token_file(f, "--token-file")?),
            Some("--token-file"),
        )
    } else if explicit.is_none()
        && let Some(t) = env.get("QUIRE_TOKEN")
    {
        (Some(t.trim().to_owned()), Some("QUIRE_TOKEN"))
    } else if explicit.is_none()
        && let Some(f) = env.get("QUIRE_TOKEN_FILE")
    {
        (
            Some(read_token_file(Path::new(f), "QUIRE_TOKEN_FILE")?),
            Some("QUIRE_TOKEN_FILE"),
        )
    } else if let Some(p) = profile.filter(|p| p.token.is_some() || p.token_file.is_some()) {
        let name = profile_name.as_deref().unwrap_or_default();
        if bound {
            let t = match (&p.token, &p.token_file) {
                (Some(t), _) => t.trim().to_owned(),
                (None, Some(f)) => {
                    let f = if f.is_relative() {
                        path.parent().unwrap_or(Path::new(".")).join(f)
                    } else {
                        f.clone()
                    };
                    read_token_file(&f, "the profile's token_file")?
                }
                (None, None) => unreachable!("filtered above"),
            };
            (Some(t), Some("profile"))
        } else if let Some(u) = &url {
            return Err(Failure::cli(
                "cli_config",
                format!(
                    "token not bound to this URL: profile {name:?} has a token for {}, but the URL is {} (from {}); \
                     pass --token-file or --token-stdin, or use --profile",
                    profile_url
                        .as_ref()
                        .map(Origin::base)
                        .unwrap_or_else(|| "no URL".into()),
                    u.base(),
                    url_source.unwrap_or("?")
                ),
                Some(json!({ "profile": name, "url_source": url_source })),
            ));
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    if let (Some(t), Some(source)) = (&token, token_source) {
        check_token(t, source)?;
    }

    let ca_file = g
        .ca_file
        .clone()
        .or_else(|| env.get("QUIRE_CA_FILE").map(PathBuf::from))
        .or_else(|| profile.filter(|_| bound).and_then(|p| p.ca_file.clone()));
    let allow_http = matches!(env.get("QUIRE_ALLOW_HTTP"), Some("1" | "true"))
        || (bound && profile.and_then(|p| p.allow_http) == Some(true))
        || url_source == Some("QUIRE_LISTEN");

    Ok(Resolved {
        path,
        profile: profile_name,
        url,
        url_source,
        token,
        token_source,
        ca_file,
        allow_http,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(seed: u8) -> String {
        quire_api::token::encode(&[seed; 32])
    }

    struct Dir(tempfile::TempDir);

    fn dir() -> Dir {
        Dir(tempfile::tempdir().expect("tempdir"))
    }

    impl Dir {
        fn config(&self, text: &str) -> String {
            let p = self.0.path().join("config.toml");
            fs::write(&p, text).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            p.to_string_lossy().into_owned()
        }
    }

    fn g(list: &[&str]) -> GlobalArgs {
        use clap::Parser;
        crate::args::Cli::try_parse_from(
            std::iter::once("quire")
                .chain(list.iter().copied())
                .chain(["config", "show"]),
        )
        .expect("parses")
        .global
    }

    fn no_stdin() -> std::io::Result<String> {
        panic!("stdin read")
    }

    #[test]
    fn origins() {
        let o = Origin::parse("HTTPS://Wiki.Example.ORG/").unwrap();
        assert_eq!(o.base(), "https://wiki.example.org");
        assert_eq!(Origin::parse("https://h:443").unwrap().base(), "https://h");
        assert_eq!(
            Origin::parse("http://h:8080").unwrap().base(),
            "http://h:8080"
        );
        assert_eq!(
            Origin::parse("http://[::1]:9").unwrap().base(),
            "http://[::1]:9"
        );
        assert!(Origin::parse("http://[::1]:9").unwrap().is_loopback());
        assert!(Origin::parse("http://127.0.0.5").unwrap().is_loopback());
        assert!(Origin::parse("http://localhost:1").unwrap().is_loopback());
        assert!(!Origin::parse("http://example.org").unwrap().is_loopback());
        for bad in [
            "wiki.example.org",
            "ftp://h",
            "https://",
            "https://h/path",
            "https://u@h",
            "https://h:0",
            "https://h:x",
            "https://h?q",
            "https://[zz]",
        ] {
            assert!(Origin::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            listen_origin("0.0.0.0:8080").unwrap().base(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            listen_origin("[::]:9000").unwrap().base(),
            "http://127.0.0.1:9000"
        );
        assert_eq!(
            listen_origin("[::1]:9000").unwrap().base(),
            "http://[::1]:9000"
        );
    }

    #[test]
    fn pair_rules() {
        let d = dir();
        let (a, b) = (token(1), token(2));
        let cfg = d.config(&format!(
            "default_profile = \"home\"\n[profiles.home]\nurl = \"https://home.example\"\ntoken = \"{a}\"\n\
             [profiles.work]\nurl = \"https://work.example\"\ntoken = \"{b}\"\n"
        ));
        // The default profile.
        let r = resolve(
            &g(&[]),
            &Env::from_pairs(&[("QUIRE_CONFIG", &cfg)]),
            no_stdin,
        )
        .unwrap();
        assert_eq!(r.url.unwrap().base(), "https://home.example");
        assert_eq!(
            (r.url_source, r.token_source),
            (Some("profile"), Some("profile"))
        );
        assert_eq!(r.token.as_deref(), Some(a.as_str()));
        // QUIRE_PROFILE picks another one.
        let env = Env::from_pairs(&[("QUIRE_CONFIG", &cfg), ("QUIRE_PROFILE", "work")]);
        let r = resolve(&g(&[]), &env, no_stdin).unwrap();
        assert_eq!(r.token.as_deref(), Some(b.as_str()));
        // A foreign URL never gets the profile's token.
        let env = Env::from_pairs(&[
            ("QUIRE_CONFIG", &cfg),
            ("QUIRE_URL", "https://evil.example"),
        ]);
        let err = resolve(&g(&[]), &env, no_stdin).unwrap_err();
        assert_eq!(err.exit, quire_api::cli::ExitCode::Configuration);
        assert!(err.message().contains("not bound"), "{}", err.message());
        // The same origin is the profile's URL.
        let env = Env::from_pairs(&[
            ("QUIRE_CONFIG", &cfg),
            ("QUIRE_URL", "https://HOME.example/"),
        ]);
        let r = resolve(&g(&[]), &env, no_stdin).unwrap();
        assert_eq!(r.token_source, Some("profile"));
        // An explicit --profile beats QUIRE_URL and QUIRE_TOKEN.
        let env = Env::from_pairs(&[
            ("QUIRE_CONFIG", &cfg),
            ("QUIRE_URL", "https://evil.example"),
            ("QUIRE_TOKEN", &a),
        ]);
        let r = resolve(&g(&["--profile", "work"]), &env, no_stdin).unwrap();
        assert_eq!(r.url.unwrap().base(), "https://work.example");
        assert_eq!(r.token.as_deref(), Some(b.as_str()));
        // --profile with a foreign --url and no explicit token: exit 3.
        let err = resolve(
            &g(&["--profile", "work", "--url", "https://x.example"]),
            &env,
            no_stdin,
        )
        .unwrap_err();
        assert_eq!(err.exit, quire_api::cli::ExitCode::Configuration);
        // An explicit token pairs with any URL.
        let r = resolve(&g(&["--url", "https://x.example"]), &env, no_stdin).unwrap();
        assert_eq!(
            (r.url_source, r.token_source),
            (Some("--url"), Some("QUIRE_TOKEN"))
        );
        // --token-stdin.
        let r = resolve(&g(&["--token-stdin"]), &env, || Ok(format!("{b}\n"))).unwrap();
        assert_eq!(
            (r.token.as_deref(), r.token_source),
            (Some(b.as_str()), Some("--token-stdin"))
        );
        // A missing profile.
        assert!(resolve(&g(&["--profile", "nope"]), &env, no_stdin).is_err());
    }

    #[test]
    fn listen_and_nothing() {
        let d = dir();
        let cfg = d.0.path().join("none.toml").to_string_lossy().into_owned();
        let r = resolve(
            &g(&[]),
            &Env::from_pairs(&[("QUIRE_CONFIG", &cfg)]),
            no_stdin,
        )
        .unwrap();
        assert!(r.url.is_none() && r.token.is_none());
        let env = Env::from_pairs(&[("QUIRE_CONFIG", &cfg), ("QUIRE_LISTEN", "0.0.0.0:8080")]);
        let r = resolve(&g(&[]), &env, no_stdin).unwrap();
        assert_eq!(r.url.unwrap().base(), "http://127.0.0.1:8080");
        assert_eq!(r.url_source, Some("QUIRE_LISTEN"));
        assert!(r.allow_http);
    }

    #[test]
    fn malformed_and_unreadable() {
        let d = dir();
        let cfg = d.config("[profiles.a]\nurl = \"https://a.example\"\n");
        let env = Env::from_pairs(&[("QUIRE_CONFIG", &cfg), ("QUIRE_TOKEN", "qpat_short")]);
        let err = resolve(&g(&[]), &env, no_stdin).unwrap_err();
        assert_eq!(err.exit, quire_api::cli::ExitCode::Configuration);
        assert_eq!(err.line()["error"]["details"]["reason"], "length");
        let with_token = d.config(&format!(
            "[profiles.a]\nurl = \"https://a.example\"\ntoken = \"{}\"\n",
            token(3)
        ));
        fs::set_permissions(&with_token, fs::Permissions::from_mode(0o640)).unwrap();
        let err = resolve(
            &g(&[]),
            &Env::from_pairs(&[("QUIRE_CONFIG", &with_token)]),
            no_stdin,
        )
        .unwrap_err();
        assert!(
            err.message().contains("readable by group"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn write_and_remove() {
        let d = dir();
        let path = d.0.path().join("sub").join("config.toml");
        let url = Origin::parse("https://a.example").unwrap();
        write_profile(&path, "default", &url, &StoredToken::Inline(token(4))).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let file = load(&path).unwrap();
        assert_eq!(file.default_profile.as_deref(), Some("default"));
        assert_eq!(
            file.profiles["default"].token.as_deref(),
            Some(token(4).as_str())
        );
        write_profile(&path, "two", &url, &StoredToken::File("/run/t".into())).unwrap();
        let file = load(&path).unwrap();
        assert_eq!(file.default_profile.as_deref(), Some("default"));
        assert_eq!(
            file.profiles["two"].token_file.as_deref(),
            Some(Path::new("/run/t"))
        );
        assert!(remove_token(&path, "default").unwrap());
        assert!(!remove_token(&path, "default").unwrap());
        assert!(load(&path).unwrap().profiles["default"].token.is_none());
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["config.toml"]);
    }
}
