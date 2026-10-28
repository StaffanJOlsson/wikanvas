//! AppState: the database, blob store, config, preview queue, work permits and limiters (owner: W2)
//!
//! [`AppState::open`] is what `serve`, the admin commands, the tests and `TestServer` start from: it creates the data
//! directory layout, opens the database (pending migrations included, with the pre-migration backup), and builds the
//! spec. Password hashing (argon2id, D17) lives here too, behind one process-wide `Semaphore(2)`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use quire_api::ErrorCode;
use quire_store::blobs::BlobStore;
use quire_store::db::{Db, DbOptions};
use tokio::sync::Semaphore;

use crate::board_previews::{self, PreviewQueue};
use crate::config::{Config, SyncMode};
use crate::error::ApiError;
use crate::ratelimit::{AuthFailLimiter, TokenBytesLimiter, TokenLimiter};

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
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            StartError::Store(e) => write!(f, "database: {e}"),
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
        let blobs = Arc::new(BlobStore::new(config.data_dir.clone()));
        let mut state = AppState {
            db,
            blobs,
            previews: PreviewQueue::noop(),
            board_work: Arc::new(Semaphore::new(BOARD_PERMITS)),
            doc_work: Arc::new(Semaphore::new(DOC_PERMITS)),
            upload_work: Arc::new(Semaphore::new(UPLOAD_PERMITS)),
            token_limiter: Arc::new(TokenLimiter::new(&config)),
            token_bytes_limiter: Arc::new(TokenBytesLimiter::new(&config)),
            auth_fail_limiter: Arc::new(AuthFailLimiter::new()),
            owner_exists: Arc::new(AtomicBool::new(false)),
            openapi: crate::openapi::spec_json().into(),
            config: Arc::new(config),
        };
        state.owner_exists().await?;
        state.previews = board_previews::spawn(state.clone());
        Ok(state)
    }

    /// Whether an owner exists (§8): the cached `true`, or one reader query while the cache says no.
    pub async fn owner_exists(&self) -> Result<bool, ApiError> {
        if self.owner_exists.load(Ordering::Acquire) {
            return Ok(true);
        }
        let exists = self
            .db
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'owner')",
                    [],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .await?;
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
    let len = password.len();
    if !(12..=1024).contains(&len) {
        return Err(ApiError::validation(
            "password",
            "a password is 12 to 1024 bytes long",
        ));
    }
    Ok(())
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
    fn password_length() {
        assert!(check_password("short").is_err());
        assert!(check_password("twelve chars").is_ok());
        assert!(check_password(&"x".repeat(1025)).is_err());
    }
}
