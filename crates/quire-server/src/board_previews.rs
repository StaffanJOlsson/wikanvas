//! The board preview worker: PreviewQueue and spawn (owner: W6).
//!
//! §12: an mpsc queue of item ids, deduplicated and debounced by 5 s, rendering every page SVG under
//! `spawn_blocking` while holding the `board_work` permit. Until W6 lands, [`spawn`] starts no worker and
//! [`PreviewQueue::enqueue`] does nothing; board routes may call it already.

use crate::state::AppState;

/// The handle board writes use to ask for fresh previews. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct PreviewQueue {
    _private: (),
}

impl PreviewQueue {
    /// A queue with no worker behind it: [`enqueue`](Self::enqueue) does nothing. [`AppState`] starts with one, and
    /// [`spawn`] replaces it.
    pub fn noop() -> PreviewQueue {
        PreviewQueue::default()
    }

    /// Asks for the previews of `item_id` to be rendered (deduplicated and debounced by the worker).
    pub fn enqueue(&self, item_id: &str) {
        let _ = item_id;
    }
}

/// Starts the preview worker for `state` and returns its queue.
pub fn spawn(state: AppState) -> PreviewQueue {
    drop(state);
    PreviewQueue::noop()
}
