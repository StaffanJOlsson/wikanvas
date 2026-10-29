//! Stars: per-user starred items with their own order (owner: W1).
//!
//! `stars.sort_key` orders a user's stars (the sidebar's STARRED section, `GET /starred`) with the same fractional
//! keys as siblings (D6), rebalanced past 64 characters. The lists show starred items that are visible and
//! unarchived; a star on a trashed or archived item stays and shows again once it is restored or unarchived.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use wikanvas_core::api::{ErrorCode, ItemSummary, Position};

use crate::error::{Error, Result};
use crate::shared::{ItemRow, Viewer, item_row, place_rebalancing, visible_items};
use crate::tree::CHILD_COUNT;

/// Stars an item for `user_id` at `position` among the user's stars (omitted: last for a new star; an existing star
/// keeps its place unless a position is given). `not_found` when the item does not exist; 409 `read_only` when it is
/// in the trash; 422 `position`.
pub fn star(
    tx: &Transaction<'_>,
    user_id: &str,
    item_id: &str,
    position: Option<&Position>,
    now: i64,
) -> Result<()> {
    if item_row(tx, item_id)?.trashed() {
        return Err(Error::new(ErrorCode::ReadOnly, "the item is in the trash"));
    }
    let existing: Option<String> = tx
        .query_row(
            "SELECT sort_key FROM stars WHERE user_id = ?1 AND item_id = ?2",
            params![user_id, item_id],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_some() && position.is_none() {
        return Ok(());
    }
    let load = || -> Result<Vec<(String, String)>> {
        Ok(tx
            .prepare_cached(
                "SELECT item_id, sort_key FROM stars WHERE user_id = ?1 AND item_id <> ?2 ORDER BY sort_key, item_id",
            )?
            .query_map(params![user_id, item_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    };
    let rewrite = |fresh: &[(String, String)]| -> Result<()> {
        let mut stmt = tx
            .prepare_cached("UPDATE stars SET sort_key = ?3 WHERE user_id = ?1 AND item_id = ?2")?;
        for (id, key) in fresh {
            stmt.execute(params![user_id, id, key])?;
        }
        Ok(())
    };
    let key = place_rebalancing(load, rewrite, position)?;
    tx.execute(
        "INSERT INTO stars (user_id, item_id, sort_key, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (user_id, item_id) DO UPDATE SET sort_key = excluded.sort_key",
        params![user_id, item_id, key, now],
    )?;
    Ok(())
}

/// Removes a star; removing one that does not exist is not an error. `not_found` when the item does not exist.
pub fn unstar(tx: &Transaction<'_>, user_id: &str, item_id: &str) -> Result<()> {
    item_row(tx, item_id)?;
    tx.execute(
        "DELETE FROM stars WHERE user_id = ?1 AND item_id = ?2",
        params![user_id, item_id],
    )?;
    Ok(())
}

/// Whether `user_id` starred the item.
pub fn is_starred(conn: &Connection, user_id: &str, item_id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stars WHERE user_id = ?1 AND item_id = ?2)",
        params![user_id, item_id],
        |r| r.get(0),
    )?)
}

/// The viewer's starred items that are visible and unarchived, in star order (the sidebar's STARRED section).
pub fn list(conn: &Connection, viewer: &Viewer) -> Result<Vec<ItemSummary>> {
    let visible = visible_items(viewer);
    let sql = format!(
        "SELECT {}, {CHILD_COUNT} FROM stars s JOIN items i ON i.id = s.item_id JOIN collections c ON c.id = i.collection_id
         WHERE s.user_id = ? AND {} AND i.archived_at IS NULL
         ORDER BY s.sort_key, s.item_id",
        ItemRow::COLUMNS,
        visible.sql
    );
    let mut args = vec![Value::Text(viewer.user_id.clone())];
    args.extend(visible.args);
    let rows = conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args.iter()), |r| {
            Ok((
                ItemRow::from_row(r)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(row, n)| row.summary(n as u64))
        .collect()
}
