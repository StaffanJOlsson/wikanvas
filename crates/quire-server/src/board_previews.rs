//! The board preview worker: PreviewQueue and spawn (owner: W6).
//!
//! §12, D10: board writes [`PreviewQueue::enqueue`] the item id; one worker task deduplicates the ids and debounces
//! them (it renders an item [`DEBOUNCE`] after its last enqueue, and at most [`MAX_WAIT`] after its first, so steady
//! saving cannot starve it). A render ([`render`]) holds the `board_work` permit (D25) for the whole job: it reads the
//! stored board, splices its assets and renders every page with `inkboard_core::svg::export_approx` under
//! `spawn_blocking`, then stores the SVGs for that revision (`quire_store::previews::replace`, which ignores a render
//! that a newer save made stale). `GET …/board/pages/{n}` calls [`render`] directly when the current revision has no
//! preview yet. The worker skips items whose previews are already current, and items that no longer exist.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use quire_store::blobs::BlobStore;
use quire_store::db::Db;
use quire_store::{boards, previews};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::Instant;

use crate::error::ApiError;
use crate::state::AppState;

/// How long after an item's last enqueue the worker renders it (§12).
pub const DEBOUNCE: Duration = Duration::from_secs(5);

/// The longest an enqueued item waits, however often it is enqueued again.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// The handle board writes use to ask for fresh previews. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct PreviewQueue {
    tx: Option<mpsc::UnboundedSender<String>>,
}

impl PreviewQueue {
    /// A queue with no worker behind it: [`enqueue`](Self::enqueue) does nothing. [`AppState`] starts with one, and
    /// [`spawn`] replaces it.
    pub fn noop() -> PreviewQueue {
        PreviewQueue::default()
    }

    /// Asks for the previews of `item_id` to be rendered (deduplicated and debounced by the worker).
    pub fn enqueue(&self, item_id: &str) {
        if let Some(tx) = &self.tx {
            // The worker only stops with the runtime; a closed channel means shutdown.
            let _ = tx.send(item_id.to_owned());
        }
    }
}

/// Starts the preview worker for `state` and returns its queue. Must run inside a tokio runtime.
pub fn spawn(state: AppState) -> PreviewQueue {
    spawn_with(state, DEBOUNCE, MAX_WAIT)
}

/// [`spawn`] with other timings (tests).
pub fn spawn_with(state: AppState, debounce: Duration, max_wait: Duration) -> PreviewQueue {
    let (tx, rx) = mpsc::unbounded_channel();
    // The worker keeps only what it needs, not the state (whose queue would keep its own channel open).
    let worker = Worker {
        db: state.db.clone(),
        blobs: state.blobs.clone(),
        permits: state.board_work.clone(),
    };
    tokio::spawn(worker.run(rx, debounce, max_wait));
    PreviewQueue { tx: Some(tx) }
}

/// The page SVGs of one board revision, as [`render`] made and stored them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub rev: i64,
    /// One entry per page; `None` for an empty page.
    pub pages: Vec<Option<String>>,
}

/// Renders and stores the previews of `item_id`'s current revision, under the `board_work` permit. `not_found` when
/// the item has no board.
pub async fn render(state: &AppState, item_id: &str) -> Result<Rendered, ApiError> {
    render_with(&state.db, &state.blobs, &state.board_work, item_id).await
}

async fn render_with(
    db: &Db,
    blobs: &Arc<BlobStore>,
    permits: &Semaphore,
    item_id: &str,
) -> Result<Rendered, ApiError> {
    let _permit = permits.acquire().await.map_err(ApiError::internal)?;
    let id = item_id.to_owned();
    let stored = db.read(move |c| boards::stored(c, &id)).await?;
    let rev = stored.rev;
    let blobs = Arc::clone(blobs);
    let pages = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let full = boards::assemble(&blobs, &stored)?;
        drop(stored);
        quire_core::board::previews(&full).map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)??;
    let (id, svgs) = (item_id.to_owned(), pages.clone());
    db.write(move |tx| previews::replace(tx, &id, rev, &svgs))
        .await?;
    Ok(Rendered { rev, pages })
}

struct Worker {
    db: Db,
    blobs: Arc<BlobStore>,
    permits: Arc<Semaphore>,
}

/// When an enqueued item is due: its last enqueue plus the debounce, capped at its first enqueue plus the max wait.
#[derive(Clone, Copy)]
struct Due {
    first: Instant,
    at: Instant,
}

impl Worker {
    async fn run(
        self,
        mut rx: mpsc::UnboundedReceiver<String>,
        debounce: Duration,
        max_wait: Duration,
    ) {
        let mut due: HashMap<String, Due> = HashMap::new();
        let mut open = true;
        loop {
            let next = due.values().map(|d| d.at).min();
            if !open && next.is_none() {
                return;
            }
            tokio::select! {
                msg = rx.recv(), if open => match msg {
                    Some(id) => {
                        let now = Instant::now();
                        let entry = due.entry(id).or_insert(Due { first: now, at: now });
                        entry.at = (now + debounce).min(entry.first + max_wait);
                    }
                    None => open = false,
                },
                () = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {
                    let now = Instant::now();
                    let ready: Vec<String> =
                        due.iter().filter(|(_, d)| d.at <= now).map(|(k, _)| k.clone()).collect();
                    for id in ready {
                        due.remove(&id);
                        if let Err(e) = self.job(&id).await {
                            tracing::warn!(item_id = %id, error = %e, "board preview failed");
                        }
                    }
                }
            }
        }
    }

    /// Renders `item_id` unless its previews are current (or it is gone).
    async fn job(&self, item_id: &str) -> Result<(), ApiError> {
        let id = item_id.to_owned();
        let current = self
            .db
            .read(move |c| {
                let state = match boards::state(c, &id) {
                    Ok(s) => s,
                    Err(e) if e.is(quire_api::ErrorCode::NotFound) => return Ok(true),
                    Err(e) => return Err(e),
                };
                let rows: i64 = c.query_row(
                    "SELECT count(*) FROM board_previews WHERE item_id = ?1 AND rev = ?2",
                    rusqlite::params![id, state.rev],
                    |r| r.get(0),
                )?;
                Ok(rows as usize == state.pages)
            })
            .await?;
        if current {
            return Ok(());
        }
        match render_with(&self.db, &self.blobs, &self.permits, item_id).await {
            Err(e) if e.code == quire_api::ErrorCode::NotFound => Ok(()),
            other => other.map(drop),
        }
    }
}
