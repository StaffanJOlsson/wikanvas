//! Collections: create, edit, reorder, soft-delete and restore (owner: W1).
//!
//! W0b provides [`create`] (POST /collections, the seed) and [`get`]; W1 completes the rest.

use quire_core::api::{ChildOrder, Collection, NewCollection, Timestamp};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::{Error, Result};
use crate::shared::{new_id, new_url_id, parse_uuid, place};

/// The default collection colour (§5).
pub const DEFAULT_COLOR: &str = "#8D8D8D";

/// Creates a collection at `new.position` among all collections (omitted: last). Validation (422, `details.field`):
/// `name` 1–100 characters after trimming, `description` at most 500, `color` `#RRGGBB`, `icon` a lucide name
/// (1–64 of `a-z0-9-`).
pub fn create(
    tx: &Transaction<'_>,
    actor_id: &str,
    new: &NewCollection,
    now: i64,
) -> Result<Collection> {
    let name = new.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(Error::validation(
            "name",
            "the name must be 1 to 100 characters",
        ));
    }
    let description = new.description.as_deref().unwrap_or("");
    if description.chars().count() > 500 {
        return Err(Error::validation(
            "description",
            "the description must be at most 500 characters",
        ));
    }
    let color = new.color.as_deref().unwrap_or(DEFAULT_COLOR);
    if !is_hex_color(color) {
        return Err(Error::validation("color", "the colour must be #RRGGBB"));
    }
    if let Some(icon) = &new.icon
        && (icon.is_empty()
            || icon.len() > 64
            || !icon
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    {
        return Err(Error::validation(
            "icon",
            "the icon must be a lucide icon name",
        ));
    }
    let siblings = tx
        .prepare("SELECT id, sort_key FROM collections ORDER BY sort_key, id")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let sort_key = place(&siblings, new.position.as_ref())?;
    let id = new_id();
    tx.execute(
        "INSERT INTO collections (id, url_id, name, description, icon, color, sort_key, created_by, updated_by,
           created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?9)",
        params![
            id,
            new_url_id()?,
            name,
            description,
            new.icon,
            color,
            sort_key,
            actor_id,
            now
        ],
    )?;
    get(tx, &id)
}

/// A collection by id (whatever its state), with its count of live, unarchived items. `not_found` when missing.
pub fn get(conn: &Connection, id: &str) -> Result<Collection> {
    conn.query_row(
        "SELECT c.id, c.url_id, c.name, c.description, c.icon, c.color, c.sort_key, c.child_order, c.updated_at,
           (SELECT count(*) FROM items i
             WHERE i.collection_id = c.id AND i.deleted_at IS NULL AND i.archived_at IS NULL)
         FROM collections c WHERE c.id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
            ))
        },
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
    .and_then(
        |(id, url_id, name, description, icon, color, sort_key, child_order, updated_at, count)| {
            Ok(Collection {
                id: parse_uuid(&id)?,
                url_id,
                name,
                description,
                icon,
                color,
                sort_key,
                child_order: child_order.parse::<ChildOrder>().map_err(|e| {
                    Error::internal("stored child_order", e)
                })?,
                item_count: count as u64,
                updated_at: Timestamp(updated_at),
            })
        },
    )
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}
