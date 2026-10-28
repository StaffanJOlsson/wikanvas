//! AppState: the database, blob store, config, preview queue, work permits and limiters (owner: W2)
//!
//! [`AppState::open`] is what `serve`, the admin commands, the tests and `TestServer` start from: it creates the data
//! directory layout, opens the database (pending migrations included, with the pre-migration backup), checks the
//! embedded Inkboard manifest (§10.5: a `format_version` mismatch refuses to start), seeds Inkboard's id generator,
//! and builds the spec. Password hashing (argon2id, D17) lives here too, behind one process-wide `Semaphore(2)`.
//!
//! Beyond the W0b field list (additive, R3): the [`Clock`] every limiter and "now" runs on, the login limiter, the
//! static [`Site`], the setup code, the token last-use map, the single-flight flags of export and backup, the
//! free-space probe (D33, injectable in tests) and the dummy hash for unknown emails (§8).

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use quire_api::ErrorCode;
use quire_store::blobs::BlobStore;
use quire_store::db::{Db, DbOptions};
use tokio::sync::Semaphore;

use crate::board_previews::{self, PreviewQueue};
use crate::config::{Config, SyncMode};
use crate::error::ApiError;
use crate::ratelimit::{AuthFailLimiter, Clock, LoginLimiter, TokenBytesLimiter, TokenLimiter};
use crate::static_files::{Embedded, Site};

/// Board GET, PUT and previews (D25).
pub const BOARD_PERMITS: usize = 1;
/// Document reads and Yjs POSTs take one, a whole-document PUT two at once (D25).
pub const DOC_PERMITS: usize = 3;
/// Attachment uploads (D25).
pub const UPLOAD_PERMITS: usize = 2;
/// Concurrent argon2 hashes or verifications (D17).
pub const ARGON2_PERMITS: usize = 2;

static ARGON2_WORK: Semaphore = Semaphore::const_new(ARGON2_PERMITS);

/// Shared state of every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub blobs: Arc<BlobStore>,
    pub config: Arc<Config>,
    pub previews: PreviewQueue,
    pub board_work: Arc<Semaphore>,
    pub doc_work: Arc<Semaphore>,
    pub upload_work: Arc<Semaphore>,
    pub token_limiter: Arc<TokenLimiter>,
    pub token_bytes_limiter: Arc<TokenBytesLimiter>,
    pub auth_fail_limiter: Arc<AuthFailLimiter>,
    /// The owner-exists cache (§8): once true it stays true; while false every request re-checks.
    pub owner_exists: Arc<AtomicBool>,
    /// The OpenAPI document, pretty-printed with a trailing newline, as served and committed (§7.4).
    pub openapi: Arc<str>,
    /// The time source of the limiters and of [`AppState::now`] (a fake clock in tests).
    pub clock: Clock,
    /// Login and setup attempts per IP, failed logins per email (§8).
    pub login_limiter: Arc<LoginLimiter>,
    /// The static files and the Inkboard manifest (§10.5, §11).
    pub site: Arc<Site>,
    /// The first-run setup code (`QUIRE_SETUP_TOKEN`, or generated at startup, D19).
    pub setup_code: Arc<str>,
    /// When each token's `last_used_at` was last written (§7.3).
    pub last_used: Arc<LastUsed>,
    /// Export and backup are single-flight (D47).
    pub single_flight: Arc<SingleFlight>,
    /// The free-space check of writes that stage files (D33); tests inject their own.
    pub free_space: FreeSpaceProbe,
    /// An argon2id hash verified for unknown emails, so a login takes the same time either way (§8).
    pub dummy_hash: Arc<tokio::sync::OnceCell<String>>,
}

/// `check_free_space(data_dir, min_free_mb)`: 507 `insufficient_storage` below the floor.
pub type FreeSpaceProbe = Arc<dyn Fn(&Path, u64) -> quire_store::Result<()> + Send + Sync>;

/// The token last-use map (§7.3): a token's row is written at most once a minute.
#[derive(Debug, Default)]
pub struct LastUsed {
    written: Mutex<HashMap<String, i64>>,
}

/// How often a token's `last_used_at` is written.
pub const LAST_USED_INTERVAL_MS: i64 = 60_000;

impl LastUsed {
    /// Whether the row of `token_id` should be written at `now`; when it should, `now` is remembered.
    pub fn due(&self, token_id: &str, now: i64) -> bool {
        let mut map = self.written.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(token_id) {
            Some(last) if now - last < LAST_USED_INTERVAL_MS => false,
            _ => {
                map.insert(token_id.to_owned(), now);
                true
            }
        }
    }

    /// Forgets tokens not written for an hour (the hourly job).
    pub fn prune(&self, now: i64) {
        let mut map = self.written.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, last| now - *last < 3_600_000);
    }
}

/// What is single-flight (D47).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flight {
    Export,
    Backup,
}

/// The single-flight flags.
#[derive(Debug, Default)]
pub struct SingleFlight {
    export: AtomicBool,
    backup: AtomicBool,
}

impl SingleFlight {
    fn flag(&self, which: Flight) -> &AtomicBool {
        match which {
            Flight::Export => &self.export,
            Flight::Backup => &self.backup,
        }
    }

    /// Takes the flag; `None` while another holder runs (429 `busy`). The flag is released when the guard drops.
    pub fn try_start(self: &Arc<Self>, which: Flight) -> Option<FlightGuard> {
        self.flag(which)
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(FlightGuard {
            flags: Arc::clone(self),
            which,
        })
    }
}

/// Holds a single-flight flag until dropped.
#[derive(Debug)]
pub struct FlightGuard {
    flags: Arc<SingleFlight>,
    which: Flight,
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        self.flags.flag(self.which).store(false, Ordering::Release);
    }
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("db", &self.db)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Why the server could not start.
#[derive(Debug)]
pub enum StartError {
    Io(std::path::PathBuf, std::io::Error),
    Store(quire_store::error::Error),
    /// The embedded Inkboard build does not fit this server (§10.5).
    Inkboard(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            StartError::Store(e) => write!(f, "database: {e}"),
            StartError::Inkboard(e) => write!(f, "inkboard: {e}"),
        }
    }
}

impl std::error::Error for StartError {}

impl From<ApiError> for StartError {
    fn from(e: ApiError) -> Self {
        StartError::Store(quire_store::Error::new(e.code, e.message))
    }
}

impl From<quire_store::error::Error> for StartError {
    fn from(e: quire_store::error::Error) -> Self {
        StartError::Store(e)
    }
}

impl AppState {
    /// Creates `data_dir/{files,tmp,backups}`, opens and migrates the database, and builds the state. The preview
    /// worker is started too ([`board_previews::spawn`]). Must run inside a tokio runtime.
    pub async fn open(config: Config) -> Result<AppState, StartError> {
        for dir in [
            config.data_dir.clone(),
            config.files_dir(),
            config.tmp_dir(),
            config.backups_dir(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|e| StartError::Io(dir.clone(), e))?;
        }
        let db = Db::open(
            config.db_path(),
            DbOptions {
                synchronous_full: config.sync == SyncMode::Full,
                ..DbOptions::default()
            },
        )?;
        let (migrate_db, data_dir) = (db.clone(), config.data_dir.clone());
        let migrated = tokio::task::spawn_blocking(move || {
            quire_store::migrate::run(&migrate_db, &data_dir, now_ms())
        })
        .await
        .map_err(|e| StartError::Io(config.db_path(), std::io::Error::other(e)))??;
        if migrated.from != migrated.to {
            tracing::info!(from = migrated.from, to = migrated.to, backup = ?migrated.backup, "migrated the database");
        }
        let site = Site::new(Arc::new(Embedded)).map_err(StartError::Inkboard)?;
        if site.manifest().is_none() {
            tracing::warn!(
                "no Inkboard manifest is embedded: boards open in preview mode (config.inkboard = null)"
            );
        }
        let mut seed = [0u8; 8];
        getrandom::fill(&mut seed).map_err(|e| StartError::Inkboard(format!("getrandom: {e}")))?;
        inkboard_core::model::seed_ids(u64::from_le_bytes(seed) | 1);
        let setup_code: Arc<str> = match &config.setup_token {
            Some(code) => code.as_str().into(),
            None => generate_setup_code()
                .map_err(|e| StartError::Inkboard(format!("getrandom: {e}")))?
                .into(),
        };
        let clock = if config.fake_clock {
            Clock::fake()
        } else {
            Clock::real()
        };
        let blobs = Arc::new(BlobStore::new(config.data_dir.clone()));
        let mut state = AppState {
            db,
            blobs,
            previews: PreviewQueue::noop(),
            board_work: Arc::new(Semaphore::new(BOARD_PERMITS)),
            doc_work: Arc::new(Semaphore::new(DOC_PERMITS)),
            upload_work: Arc::new(Semaphore::new(UPLOAD_PERMITS)),
            token_limiter: Arc::new(TokenLimiter::new(&config, clock.clone())),
            token_bytes_limiter: Arc::new(TokenBytesLimiter::new(&config, clock.clone())),
            auth_fail_limiter: Arc::new(AuthFailLimiter::new(clock.clone())),
            owner_exists: Arc::new(AtomicBool::new(false)),
            openapi: crate::openapi::spec_json().into(),
            login_limiter: Arc::new(LoginLimiter::new(clock.clone())),
            clock,
            site: Arc::new(site),
            setup_code,
            last_used: Arc::default(),
            single_flight: Arc::default(),
            free_space: Arc::new(|dir: &Path, min_free_mb: u64| {
                quire_store::shared::check_free_space(dir, min_free_mb)
            }),
            dummy_hash: Arc::default(),
            config: Arc::new(config),
        };
        state.owner_exists().await?;
        state.previews = board_previews::spawn(state.clone());
        Ok(state)
    }

    /// Unix milliseconds now, on [`AppState::clock`] (fake in tests).
    pub fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The same state serving `source` instead of the embedded files (tests); fails like startup on a bad manifest.
    pub fn with_assets(
        mut self,
        source: Arc<dyn crate::static_files::AssetSource>,
    ) -> Result<AppState, String> {
        self.site = Arc::new(Site::new(source)?);
        Ok(self)
    }

    /// The log line that names the setup link while no owner exists (§8).
    pub fn setup_message(&self) -> String {
        format!(
            "First-run setup: {}/setup code {}",
            self.config.public_url, self.setup_code
        )
    }

    /// 507 `insufficient_storage` when `/data` is below the free-space floor (D33). Uploads, board PUTs, export and
    /// backup call it before they stage anything.
    pub fn check_free_space(&self) -> Result<(), ApiError> {
        (self.free_space)(&self.config.data_dir, self.config.min_free_mb).map_err(ApiError::from)
    }

    /// The argon2id hash verified for an unknown email, made once per process.
    pub async fn dummy_hash(&self) -> Result<String, ApiError> {
        let cheap = self.config.test_argon2;
        self.dummy_hash
            .get_or_try_init(|| hash_password(cheap, "quire dummy password, never valid".into()))
            .await
            .cloned()
    }

    /// Whether an owner exists (§8): the cached `true`, or one reader query while the cache says no.
    pub async fn owner_exists(&self) -> Result<bool, ApiError> {
        if self.owner_exists.load(Ordering::Acquire) {
            return Ok(true);
        }
        let exists = self.db.read(quire_store::users::owner_exists).await?;
        if exists {
            self.owner_exists.store(true, Ordering::Release);
        }
        Ok(exists)
    }

    /// argon2id hash of `password` under the shared permit (D17), in `spawn_blocking`.
    pub async fn hash_password(&self, password: String) -> Result<String, ApiError> {
        hash_password(self.config.test_argon2, password).await
    }

    /// Whether `password` matches the PHC string `hash`, under the shared permit, in `spawn_blocking`.
    pub async fn verify_password(&self, hash: String, password: String) -> Result<bool, ApiError> {
        let cheap = self.config.test_argon2;
        let _permit = ARGON2_WORK.acquire().await.map_err(ApiError::internal)?;
        tokio::task::spawn_blocking(move || {
            argon2(cheap)
                .verify_password(password.as_bytes(), hash.as_str())
                .is_ok()
        })
        .await
        .map_err(ApiError::internal)
    }
}

/// The argon2id instance: the `argon2` 0.6 defaults (m=19456 KiB, t=2, p=1), or the cheapest valid parameters for
/// tests (`QUIRE_TEST_ARGON2=1`). A PHC string carries its own parameters, so verification works across both.
fn argon2(cheap: bool) -> Argon2<'static> {
    let params = if cheap {
        Params::new(Params::MIN_M_COST, Params::MIN_T_COST, 1, None).expect("valid parameters")
    } else {
        Params::default()
    };
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// argon2id hash of `password` (PHC string), under the shared permit, in `spawn_blocking`.
pub async fn hash_password(cheap: bool, password: String) -> Result<String, ApiError> {
    let _permit = ARGON2_WORK.acquire().await.map_err(ApiError::internal)?;
    tokio::task::spawn_blocking(move || {
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).map_err(ApiError::internal)?;
        argon2(cheap)
            .hash_password_with_salt(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)?
}

/// Passwords are 12–1024 bytes (D35).
pub fn check_password(password: &str) -> Result<(), ApiError> {
    check_password_field(password, "password")
}

/// [`check_password`] reporting `field` (`new` for a password change).
pub fn check_password_field(password: &str, field: &str) -> Result<(), ApiError> {
    let len = password.len();
    if !(12..=1024).contains(&len) {
        return Err(ApiError::validation(
            field,
            "a password is 12 to 1024 bytes long",
        ));
    }
    Ok(())
}

/// The setup code alphabet: no 0/O, 1/I/L, so a code read from a log is easy to type.
const SETUP_ALPHABET: &[u8; 32] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789!";

/// A new setup code `XXXX-XXXX-XXXX` (12 characters of a 31-symbol alphabet, about 59 bits).
pub fn generate_setup_code() -> Result<String, getrandom::Error> {
    let mut out = String::with_capacity(14);
    let mut picked = 0;
    while picked < 12 {
        let mut buf = [0u8; 32];
        getrandom::fill(&mut buf)?;
        for b in buf {
            let c = SETUP_ALPHABET[usize::from(b & 31)];
            if c == b'!' || picked == 12 {
                continue;
            }
            if picked > 0 && picked % 4 == 0 {
                out.push('-');
            }
            out.push(char::from(c));
            picked += 1;
        }
    }
    Ok(out)
}

/// Whether `given` is the setup code: case-insensitive, surrounding whitespace ignored, compared in constant time.
pub fn setup_code_matches(expected: &str, given: &str) -> bool {
    let a = expected.trim().to_ascii_uppercase();
    let b = given.trim().to_ascii_uppercase();
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Unix milliseconds now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The `ApiError` for a store error: its code, message and details; `internal` errors are logged with their cause
/// and answered with a generic message.
pub fn store_error(err: quire_store::error::Error) -> ApiError {
    if err.code == ErrorCode::Internal {
        return ApiError::internal(&err);
    }
    let mut api = ApiError::new(err.code, err.message);
    api.details = err.details;
    api
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_hash_roundtrip_cheap_and_default_verify() {
        let hash = hash_password(true, "correct horse battery".into())
            .await
            .unwrap();
        assert!(hash.starts_with("$argon2id$"));
        let verify = |pw: &str| {
            argon2(false)
                .verify_password(pw.as_bytes(), hash.as_str())
                .is_ok()
        };
        assert!(verify("correct horse battery"));
        assert!(!verify("wrong horse battery"));
    }

    #[test]
    fn setup_codes() {
        let code = generate_setup_code().unwrap();
        assert_eq!(code.len(), 14, "{code}");
        assert_eq!(code.as_bytes()[4], b'-');
        assert_eq!(code.as_bytes()[9], b'-');
        assert!(
            code.bytes()
                .all(|c| c == b'-' || SETUP_ALPHABET[..31].contains(&c)),
            "{code}"
        );
        assert_ne!(code, generate_setup_code().unwrap());
        assert!(setup_code_matches(
            &code,
            &format!(" {} ", code.to_lowercase())
        ));
        assert!(!setup_code_matches(&code, "AAAA-AAAA-AAAA"));
        assert!(!setup_code_matches(&code, ""));
    }

    #[test]
    fn single_flight_and_last_used() {
        let flights = Arc::new(SingleFlight::default());
        let guard = flights.try_start(Flight::Export).unwrap();
        assert!(flights.try_start(Flight::Export).is_none());
        assert!(flights.try_start(Flight::Backup).is_some());
        drop(guard);
        assert!(flights.try_start(Flight::Export).is_some());
        let lu = LastUsed::default();
        assert!(lu.due("t", 0));
        assert!(!lu.due("t", 59_999));
        assert!(lu.due("t", 60_000));
        lu.prune(60_000 + 3_600_000);
        assert!(lu.due("t", 60_000 + 3_600_000));
    }

    #[test]
    fn password_length() {
        assert!(check_password("short").is_err());
        assert!(check_password("twelve chars").is_ok());
        assert!(check_password(&"x".repeat(1025)).is_err());
    }
}
