//! Background jobs: purge, GC, backup, optimize, session and token expiry (owner: W2).
//!
//! §12: one tokio task per job, each using the writer through `Db`. [`spawn`] starts them for `serve` (the tests and
//! the admin commands call the job bodies directly):
//! - **purge and GC**, at startup and every 6 h: Trash roots older than 30 days
//!   ([`wikanvas_store::items::purge_expired`], leaves first), then attachment GC, then blob GC with the orphan sweep
//!   ([`purge_and_gc`]);
//! - **revision prune**, daily ([`wikanvas_store::revisions::prune`]);
//! - **backup**, every `WIKANVAS_BACKUP_INTERVAL` while `WIKANVAS_BACKUP_KEEP` is not 0, single-flight with on-demand
//!   backups and after the free-space check ([`backup_now`]). The first run is due one interval after the newest
//!   `backups/wikanvas-*.db`, so restarts never postpone it ([`first_backup_delay`]);
//! - **hourly**: `PRAGMA optimize`, expired sessions, tokens 30 days after revocation or expiry, and the idle keys of
//!   the rate limiters and the token last-use map ([`hourly`]).
//!
//! The preview worker is started with the state ([`crate::board_previews::spawn`], W6).

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tokio::task::JoinHandle;
use wikanvas_api::ErrorCode;
use wikanvas_store::gc::BlobGc;

use crate::config::Config;
use crate::error::ApiError;
use crate::ratelimit::{self, Limiters};
use crate::state::{AppState, Flight};

/// Trash roots are purged after this long (§5).
pub const TRASH_RETENTION_MS: i64 = 30 * 86_400_000;
/// The purge and GC period.
pub const PURGE_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The revision prune period.
pub const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// The optimize / expiry period.
pub const HOURLY: Duration = Duration::from_secs(3600);
/// The scheduled backup never runs sooner than this after startup (the startup purge and GC go first).
pub const BACKUP_STARTUP_DELAY: Duration = Duration::from_secs(60);

/// The running jobs; [`Jobs::stop`] aborts them (at shutdown, before the WAL checkpoint).
#[derive(Debug, Default)]
pub struct Jobs {
    handles: Vec<JoinHandle<()>>,
}

impl Jobs {
    pub fn stop(self) {
        for h in self.handles {
            h.abort();
        }
    }
}

/// What a purge-and-GC run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    pub items_purged: u64,
    pub attachments_deleted: u64,
    pub blobs: BlobGc,
}

/// Purge, attachment GC, blob GC and the sweep, in that order.
pub async fn purge_and_gc(state: &AppState) -> Result<PurgeReport, ApiError> {
    let now = state.now();
    let items_purged = state
        .db
        .write(move |tx| wikanvas_store::items::purge_expired(tx, now - TRASH_RETENTION_MS))
        .await?;
    let attachments_deleted = state
        .db
        .write(move |tx| wikanvas_store::gc::attachments(tx, now))
        .await?;
    let blobs = wikanvas_store::gc::blobs(&state.db, &state.blobs).await?;
    Ok(PurgeReport {
        items_purged,
        attachments_deleted,
        blobs,
    })
}

/// The daily revision prune; returns the number of revisions deleted.
pub async fn prune_revisions(state: &AppState) -> Result<u64, ApiError> {
    let now = state.now();
    Ok(state
        .db
        .write(move |tx| wikanvas_store::revisions::prune(tx, now))
        .await?)
}

/// What an hourly run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HourlyReport {
    pub sessions_deleted: u64,
    pub tokens_deleted: u64,
}

/// `PRAGMA optimize`, expired sessions, dead tokens and the limiters' idle keys.
pub async fn hourly(state: &AppState) -> Result<HourlyReport, ApiError> {
    let now = state.now();
    let report = state
        .db
        .write(move |tx| {
            let sessions_deleted = wikanvas_store::sessions::delete_expired(tx, now)?;
            let tokens_deleted = wikanvas_store::tokens::purge(tx, now)?;
            Ok(HourlyReport {
                sessions_deleted,
                tokens_deleted,
            })
        })
        .await?;
    state
        .db
        .with_writer(|conn| Ok(conn.execute_batch("PRAGMA optimize")?))
        .await?;
    ratelimit::prune(Limiters {
        token: &state.token_limiter,
        bytes: &state.token_bytes_limiter,
        auth: &state.auth_fail_limiter,
        login: &state.login_limiter,
    });
    state.last_used.prune(now);
    Ok(report)
}

/// A backup now (scheduled or on demand): single-flight (429 `busy`), after the free-space check (507), counted
/// against `WIKANVAS_BACKUP_KEEP`. Returns the file and its size.
///
/// The check and the copy run on the blocking pool, which owns the single-flight guard: a caller that goes away (an
/// HTTP client that left) never frees the flag while the copy is still being written. Another process's backup (the
/// admin command while the server runs) holds the lock of [`wikanvas_store::backup::create`] and also gives `busy`.
pub async fn backup_now(state: &AppState) -> Result<(PathBuf, u64), ApiError> {
    let guard = state
        .single_flight
        .try_start(Flight::Backup)
        .ok_or_else(ApiError::busy)?;
    let (worker, now) = (state.clone(), state.now());
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        worker.check_free_space()?;
        let config = &worker.config;
        let path =
            wikanvas_store::backup::create(&worker.db, &config.data_dir, config.backup_keep, now)
                .map_err(|e| {
                if e.is(ErrorCode::RateLimited) {
                    ApiError::busy()
                } else {
                    ApiError::from(e)
                }
            })?;
        let size = std::fs::metadata(&path).map_err(ApiError::internal)?.len();
        Ok((path, size))
    })
    .await
    .map_err(ApiError::internal)?
}

/// When the scheduled backup first runs after startup (§12): `None` while `WIKANVAS_BACKUP_KEEP=0` turns it off (D21).
/// Otherwise what is left of `WIKANVAS_BACKUP_INTERVAL` since the newest `backups/wikanvas-*.db` (by modification
/// time), and at least [`BACKUP_STARTUP_DELAY`], so a backup that is due, or missing, runs soon after startup. A server
/// that restarts more often than the interval still gets its backups.
pub fn first_backup_delay(config: &Config, now_ms: i64) -> Option<Duration> {
    if config.backup_keep == 0 {
        return None;
    }
    let newest_ms = wikanvas_store::backup::list(&config.backups_dir())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .filter_map(|d| i64::try_from(d.as_millis()).ok())
        .max();
    let remaining = match newest_ms {
        Some(at) => {
            let age = Duration::from_millis(u64::try_from(now_ms - at).unwrap_or(0));
            config.backup_interval.saturating_sub(age)
        }
        None => Duration::ZERO,
    };
    Some(remaining.max(BACKUP_STARTUP_DELAY))
}

fn every<F, Fut>(
    state: &AppState,
    period: Duration,
    first: Duration,
    name: &'static str,
    job: F,
) -> JoinHandle<()>
where
    F: Fn(AppState) -> Fut + Send + 'static,
    Fut: Future<Output = Result<String, ApiError>> + Send,
{
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(first).await;
        let mut ticks = tokio::time::interval(period);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            match job(state.clone()).await {
                Ok(summary) => tracing::info!(job = name, "{summary}"),
                Err(e) => tracing::warn!(job = name, error = %e, "background job failed"),
            }
        }
    })
}

/// Starts every job (see the module docs).
pub fn spawn(state: AppState) -> Jobs {
    let mut handles = vec![
        every(
            &state,
            PURGE_EVERY,
            Duration::ZERO,
            "purge",
            |s| async move { purge_and_gc(&s).await.map(|r| format!("{r:?}")) },
        ),
        every(
            &state,
            PRUNE_EVERY,
            Duration::from_secs(60),
            "revisions",
            |s| async move {
                prune_revisions(&s)
                    .await
                    .map(|n| format!("pruned {n} revisions"))
            },
        ),
        every(
            &state,
            HOURLY,
            Duration::from_secs(30),
            "hourly",
            |s| async move { hourly(&s).await.map(|r| format!("{r:?}")) },
        ),
    ];
    if let Some(first) = first_backup_delay(&state.config, state.now()) {
        tracing::info!(in_secs = first.as_secs(), "the next scheduled backup");
        handles.push(every(
            &state,
            state.config.backup_interval,
            first,
            "backup",
            |s| async move {
                backup_now(&s)
                    .await
                    .map(|(path, size)| format!("backup {} ({size} bytes)", path.display()))
            },
        ));
    }
    Jobs { handles }
}
