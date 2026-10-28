//! Background jobs: purge, GC, backup, optimize, session and token expiry (owner: W2).
//!
//! §12: one tokio task per job, each using the writer through `Db`. [`spawn`] starts them for `serve` (the tests and
//! the admin commands call the job bodies directly):
//! - **purge and GC**, at startup and every 6 h: Trash roots older than 30 days ([`quire_store::items::purge_expired`],
//!   leaves first), then attachment GC, then blob GC with the orphan sweep ([`purge_and_gc`]);
//! - **revision prune**, daily ([`quire_store::revisions::prune`]);
//! - **backup**, every `QUIRE_BACKUP_INTERVAL` while `QUIRE_BACKUP_KEEP` is not 0, single-flight with on-demand
//!   backups and after the free-space check ([`backup_now`]);
//! - **hourly**: `PRAGMA optimize`, expired sessions, tokens 30 days after revocation or expiry, and the idle keys of
//!   the rate limiters and the token last-use map ([`hourly`]).
//!
//! The preview worker is started with the state ([`crate::board_previews::spawn`], W6).

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use quire_store::gc::BlobGc;
use tokio::task::JoinHandle;

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
        .write(move |tx| quire_store::items::purge_expired(tx, now - TRASH_RETENTION_MS))
        .await?;
    let attachments_deleted = state
        .db
        .write(move |tx| quire_store::gc::attachments(tx, now))
        .await?;
    let blobs = quire_store::gc::blobs(&state.db, &state.blobs).await?;
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
        .write(move |tx| quire_store::revisions::prune(tx, now))
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
            let sessions_deleted = quire_store::sessions::delete_expired(tx, now)?;
            let tokens_deleted = quire_store::tokens::purge(tx, now)?;
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
/// against `QUIRE_BACKUP_KEEP`. Returns the file and its size.
pub async fn backup_now(state: &AppState) -> Result<(PathBuf, u64), ApiError> {
    let _guard = state
        .single_flight
        .try_start(Flight::Backup)
        .ok_or_else(ApiError::busy)?;
    state.check_free_space()?;
    let (db, dir, keep, now) = (
        state.db.clone(),
        state.config.data_dir.clone(),
        state.config.backup_keep,
        state.now(),
    );
    let path =
        tokio::task::spawn_blocking(move || quire_store::backup::create(&db, &dir, keep, now))
            .await
            .map_err(ApiError::internal)??;
    let size = std::fs::metadata(&path).map_err(ApiError::internal)?.len();
    Ok((path, size))
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
    if state.config.backup_keep > 0 {
        let interval = state.config.backup_interval;
        handles.push(every(
            &state,
            interval,
            interval,
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
