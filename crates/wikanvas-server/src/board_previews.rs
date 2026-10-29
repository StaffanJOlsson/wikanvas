//! The board preview worker: PreviewQueue and spawn (owner: W6).
//!
//! §12, D10: board writes [`PreviewQueue::enqueue`] the item id; one worker task deduplicates the ids and debounces
//! them (it renders an item [`DEBOUNCE`] after its last enqueue, and at most [`MAX_WAIT`] after its first, so steady
//! saving cannot starve it). A render ([`render`]) holds the `board_work` permit (D25) for the whole job. The permit is
//! owned and moves into every blocking closure (the reader, `spawn_blocking`, the writer), so a request that is
//! cancelled (the timeout layer, a client that goes away) cannot release it while the work still runs. Under it, the
//! render first checks whether the current revision was rendered meanwhile (a GET that waited for the worker gets the
//! stored page), then reads the stored board, splices its assets and renders the pages one at a time with
//! `wikanvas_core::board::Previewer`, within its per-page and per-board budgets, deflating each page outside the
//! writer. The writer only stores them (`wikanvas_store::previews::store`, which ignores a render that a newer save
//! made stale). `GET …/board/pages/{n}` calls [`render`] directly when the current revision has no preview yet. The
//! worker skips items whose previews are already current, and items that no longer exist.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wikanvas_core::board::Previewer;
use wikanvas_store::blobs::BlobStore;
use wikanvas_store::boards::{self, StoredBoard};
use wikanvas_store::db::Db;
use wikanvas_store::previews::{self, PageSvg, Preview};
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

/// One page of a board revision, as [`render`] made or found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The revision the previews were made from.
    pub rev: i64,
    /// The page asked for; `None` when that revision has no such page.
    pub page: Option<Preview>,
}

/// Page `page` of `item_id`'s current revision: rendered and stored under the `board_work` permit, unless that
/// revision was rendered while this call waited for the permit. `not_found` when the item has no board.
pub async fn render(state: &AppState, item_id: &str, page: usize) -> Result<Rendered, ApiError> {
    render_with(
        &state.db,
        &state.blobs,
        &state.board_work,
        item_id,
        Some(page),
    )
    .await
}

/// What the reader found under the permit.
enum Found {
    /// The current revision is rendered: its page (if one was asked for and exists).
    Done(i64, Option<Preview>),
    /// It is not: the stored board to render.
    Todo(StoredBoard),
}

async fn render_with(
    db: &Db,
    blobs: &Arc<BlobStore>,
    permits: &Arc<Semaphore>,
    item_id: &str,
    want: Option<usize>,
) -> Result<Rendered, ApiError> {
    let permit = Arc::clone(permits)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let id = item_id.to_owned();
    let (permit, found) = db
        .read(move |c| {
            let state = boards::state(c, &id)?;
            let found = if previews::rendered(c, &id, state.rev)? {
                let page = match want {
                    Some(p) if p < state.pages => previews::get(c, &id, p, state.rev)?,
                    _ => None,
                };
                Found::Done(state.rev, page)
            } else {
                Found::Todo(boards::stored(c, &id)?)
            };
            Ok((permit, found))
        })
        .await?;
    let stored = match found {
        Found::Done(rev, page) => return Ok(Rendered { rev, page }),
        Found::Todo(stored) => stored,
    };
    let rev = stored.rev;
    let blobs = Arc::clone(blobs);
    let (permit, pages, page) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let full = boards::assemble(&blobs, &stored)?;
        drop(stored);
        let mut previewer = Previewer::new(&full).map_err(ApiError::internal)?;
        drop(full);
        let count = previewer.pages();
        // A page the budget leaves out is `Omitted`; a rendered one replaces it below.
        let mut page = want.filter(|p| *p < count).map(|_| Preview::Omitted);
        let mut pages = Vec::with_capacity(count);
        for i in 0..count {
            if previewer.spent() {
                break;
            }
            let svg = previewer.render(i);
            pages.push(svg.as_deref().map(PageSvg::new).transpose()?);
            if want == Some(i) {
                page = Some(svg.map_or(Preview::Empty, Preview::Svg));
            }
        }
        Ok((permit, pages, page))
    })
    .await
    .map_err(ApiError::internal)??;
    let id = item_id.to_owned();
    db.write(move |tx| {
        let _permit = permit;
        previews::store(tx, &id, rev, &pages)
    })
    .await?;
    Ok(Rendered { rev, page })
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
                    Err(e) if e.is(wikanvas_api::ErrorCode::NotFound) => return Ok(true),
                    Err(e) => return Err(e),
                };
                previews::rendered(c, &id, state.rev)
            })
            .await?;
        if current {
            return Ok(());
        }
        match render_with(&self.db, &self.blobs, &self.permits, item_id, None).await {
            Err(e) if e.code == wikanvas_api::ErrorCode::NotFound => Ok(()),
            other => other.map(drop),
        }
    }
}
