//! Item views: per-user view counts and last-viewed times, for `GET /recent` (owner: W1).

use rusqlite::{Transaction, params};

use crate::error::Result;
use crate::shared::item_row;

/// Records a view of the item by `user_id` (`POST /items/{id}/view`): inserts the `item_views` row, or bumps its
/// count and `last_viewed_at`. `not_found` when the item does not exist.
pub fn record(tx: &Transaction<'_>, user_id: &str, item_id: &str, now: i64) -> Result<()> {
    item_row(tx, item_id)?;
    tx.execute(
        "INSERT INTO item_views (user_id, item_id, view_count, first_viewed_at, last_viewed_at)
         VALUES (?1, ?2, 1, ?3, ?3)
         ON CONFLICT (user_id, item_id) DO UPDATE SET view_count = view_count + 1,
           last_viewed_at = max(last_viewed_at, excluded.last_viewed_at)",
        params![user_id, item_id, now],
    )?;
    Ok(())
}
