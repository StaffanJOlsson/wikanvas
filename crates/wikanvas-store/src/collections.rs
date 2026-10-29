//! Collections: create, edit, reorder, soft-delete and restore (owner: W1).
//!
//! A deleted collection exists only in Trash (§5 "Collection delete"): [`get_live`], [`update`], [`move_to`] and the
//! tree answer 404 for it, item creates and moves into it answer 409 `parent_invalid`, and only [`restore`] and
//! [`purge`] act on it. Deleting a collection marks its live items with the collection as their trash root; restoring
//! it clears exactly those rows, so items trashed earlier stay in Trash.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use wikanvas_core::api::{
    ChildOrder, Collection, CollectionPatch, ErrorCode, NewCollection, Position, Timestamp,
};

use crate::error::{Error, Result};
use crate::shared::{Viewer, new_id, new_url_id, place_rebalancing};

/// The default collection colour (§5).
pub const DEFAULT_COLOR: &str = "#8D8D8D";
/// The longest collection name, after trimming (§5).
pub const MAX_NAME_CHARS: usize = 100;
/// The longest description (§5).
pub const MAX_DESCRIPTION_CHARS: usize = 500;

/// The columns [`from_row`] reads, for a query that names the collection `c`. `item_count` is the live, unarchived
/// items of a live collection, and the items its deletion trashed for a deleted one (what a restore brings back).
const COLUMNS: &str = "c.id, c.url_id, c.name, c.description, c.icon, c.color, c.sort_key, c.child_order, c.updated_at,
    (SELECT count(*) FROM items i WHERE i.collection_id = c.id AND CASE WHEN c.deleted_at IS NULL
       THEN i.deleted_at IS NULL AND i.archived_at IS NULL ELSE i.trash_root_id = c.id END),
    c.deleted_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<(Collection, Option<i64>)> {
    let id: String = r.get(0)?;
    let child_order: String = r.get(7)?;
    let bad = |col, e: Box<dyn std::error::Error + Send + Sync>| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, e)
    };
    Ok((
        Collection {
            id: uuid::Uuid::parse_str(&id).map_err(|e| bad(0, Box::new(e)))?,
            url_id: r.get(1)?,
            name: r.get(2)?,
            description: r.get(3)?,
            icon: r.get(4)?,
            color: r.get(5)?,
            sort_key: r.get(6)?,
            child_order: child_order
                .parse::<ChildOrder>()
                .map_err(|e| bad(7, Box::new(e)))?,
            item_count: r.get::<_, i64>(9)? as u64,
            updated_at: Timestamp(r.get(8)?),
        },
        r.get(10)?,
    ))
}

/// Creates a collection at `new.position` among all collections (omitted: last). Validation (422, `details.field`):
/// `name` 1–100 characters after trimming, `description` at most 500, `color` `#RRGGBB`, `icon` a lucide name
/// (1–64 of `a-z0-9-`), `position` naming collections.
pub fn create(
    tx: &Transaction<'_>,
    actor_id: &str,
    new: &NewCollection,
    now: i64,
) -> Result<Collection> {
    let name = check_name(&new.name)?;
    let description = new.description.as_deref().unwrap_or("");
    check_description(description)?;
    let color = new.color.as_deref().unwrap_or(DEFAULT_COLOR);
    check_color(color)?;
    if let Some(icon) = &new.icon {
        check_icon(icon)?;
    }
    let sort_key = place(tx, None, new.position.as_ref())?;
    let id = new_id();
    tx.execute(
        "INSERT INTO collections (id, url_id, name, description, icon, color, sort_key, created_by, updated_by,
           created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?9)",
        params![id, new_url_id()?, name, description, new.icon, color, sort_key, actor_id, now],
    )?;
    get(tx, &id)
}

/// A collection by id, whatever its state. `not_found` when missing.
pub fn get(conn: &Connection, id: &str) -> Result<Collection> {
    Ok(get_with_state(conn, id)?.0)
}

/// A collection by id with its `deleted_at`. `not_found` when missing.
pub fn get_with_state(conn: &Connection, id: &str) -> Result<(Collection, Option<i64>)> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM collections c WHERE c.id = ?1"),
        [id],
        from_row,
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
}

/// A live collection by id. `not_found` when missing or deleted (a deleted collection is only in Trash).
pub fn get_live(conn: &Connection, id: &str) -> Result<Collection> {
    match get_with_state(conn, id)? {
        (c, None) => Ok(c),
        (_, Some(_)) => Err(Error::not_found("the collection is in the trash")),
    }
}

/// The id of the collection a path names: its id, or its `url_id` (`GET /collections/{id}`). `not_found` when there
/// is none.
pub fn resolve(conn: &Connection, id_or_url_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT id FROM collections WHERE id = ?1 OR url_id = ?1",
        [id_or_url_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
}

/// The live collections in sidebar order.
pub fn list(conn: &Connection, _viewer: &Viewer) -> Result<Vec<Collection>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM collections c WHERE c.deleted_at IS NULL ORDER BY c.sort_key, c.id"
        ))?
        .query_map([], |r| Ok(from_row(r)?.0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The deleted collections with their `deleted_at`, most recently deleted first (Trash).
pub fn deleted(conn: &Connection) -> Result<Vec<(Collection, i64)>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM collections c WHERE c.deleted_at IS NOT NULL ORDER BY c.deleted_at DESC, c.id DESC"
        ))?
        .query_map([], |r| {
            let (c, deleted) = from_row(r)?;
            Ok((c, deleted.unwrap_or_default()))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The tree order of a collection.
pub fn child_order(conn: &Connection, id: &str) -> Result<ChildOrder> {
    let order: String = conn
        .query_row(
            "SELECT child_order FROM collections WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::not_found("no such collection"))?;
    order
        .parse()
        .map_err(|e| Error::internal("stored child_order", e))
}

/// Edits a live collection: the fields `patch` sets (an `icon: null` clears the icon), validated as in [`create`].
/// A change sets `updated_at`/`updated_by` and bumps `tree_version` (the tree response holds the collection);
/// a patch that changes nothing writes nothing. `not_found` when missing or deleted.
pub fn update(
    tx: &Transaction<'_>,
    id: &str,
    patch: &CollectionPatch,
    actor_id: &str,
    now: i64,
) -> Result<Collection> {
    let current = get_live(tx, id)?;
    let name = patch.name.as_deref().map(check_name).transpose()?;
    if let Some(d) = &patch.description {
        check_description(d)?;
    }
    if let Some(c) = &patch.color {
        check_color(c)?;
    }
    if let Some(Some(icon)) = &patch.icon {
        check_icon(icon)?;
    }
    let next = Collection {
        name: name.map_or(current.name.clone(), str::to_owned),
        description: patch
            .description
            .clone()
            .unwrap_or(current.description.clone()),
        icon: patch.icon.clone().unwrap_or(current.icon.clone()),
        color: patch.color.clone().unwrap_or(current.color.clone()),
        child_order: patch.child_order.unwrap_or(current.child_order),
        ..current.clone()
    };
    if next == current {
        return Ok(current);
    }
    tx.execute(
        "UPDATE collections SET name = ?2, description = ?3, icon = ?4, color = ?5, child_order = ?6, updated_at = ?7,
           updated_by = ?8, tree_version = tree_version + 1
         WHERE id = ?1",
        params![
            id,
            next.name,
            next.description,
            next.icon,
            next.color,
            next.child_order.as_str(),
            now,
            actor_id
        ],
    )?;
    get(tx, id)
}

/// Moves a live collection to `position` among the collections (bumping its `tree_version`: the tree response holds
/// the collection's `sort_key`). `not_found` when missing or deleted.
pub fn move_to(
    tx: &Transaction<'_>,
    id: &str,
    position: &Position,
    actor_id: &str,
    now: i64,
) -> Result<Collection> {
    get_live(tx, id)?;
    let key = place(tx, Some(id), Some(position))?;
    tx.execute(
        "UPDATE collections SET sort_key = ?2, updated_at = ?3, updated_by = ?4, tree_version = tree_version + 1
         WHERE id = ?1",
        params![id, key, now, actor_id],
    )?;
    get(tx, id)
}

/// Soft-deletes a live collection: sets its `deleted_at` and marks its live items (archived ones included) with the
/// collection as their trash root. Returns how many items it trashed. `not_found` when missing or already deleted.
pub fn delete(tx: &Transaction<'_>, id: &str, actor_id: &str, now: i64) -> Result<u64> {
    get_live(tx, id)?;
    tx.execute(
        "UPDATE collections SET deleted_at = ?2, deleted_by = ?3, tree_version = tree_version + 1 WHERE id = ?1",
        params![id, now, actor_id],
    )?;
    let trashed = tx.execute(
        "UPDATE items SET deleted_at = ?2, deleted_by = ?3, trash_root_id = ?1, meta_version = meta_version + 1
         WHERE collection_id = ?1 AND deleted_at IS NULL",
        params![id, now, actor_id],
    )?;
    Ok(trashed as u64)
}

/// Restores a deleted collection and exactly the items its deletion trashed. `not_found` when missing; 409
/// `conflict` when it is not deleted.
pub fn restore(tx: &Transaction<'_>, id: &str, actor_id: &str, now: i64) -> Result<Collection> {
    if get_with_state(tx, id)?.1.is_none() {
        return Err(Error::new(
            ErrorCode::Conflict,
            "the collection is not in the trash",
        ));
    }
    tx.execute(
        "UPDATE items SET deleted_at = NULL, deleted_by = NULL, trash_root_id = NULL, meta_version = meta_version + 1
         WHERE trash_root_id = ?1",
        [id],
    )?;
    tx.execute(
        "UPDATE collections SET deleted_at = NULL, deleted_by = NULL, updated_at = ?2, updated_by = ?3,
           tree_version = tree_version + 1
         WHERE id = ?1",
        params![id, now, actor_id],
    )?;
    get(tx, id)
}

/// Permanently deletes a deleted collection with every item in it, leaves first (§5 "Purge"). Returns how many items
/// it deleted. `not_found` when missing; 409 `conflict` when it is not deleted.
pub fn purge(tx: &Transaction<'_>, id: &str) -> Result<u64> {
    if get_with_state(tx, id)?.1.is_none() {
        return Err(Error::new(
            ErrorCode::Conflict,
            "only a collection in the trash can be purged; delete it first",
        ));
    }
    purge_rows(tx, id)
}

/// Deletes the collection row and all its items, leaves first; no state check.
pub(crate) fn purge_rows(tx: &Transaction<'_>, id: &str) -> Result<u64> {
    let tops = tx
        .prepare_cached(
            "SELECT id FROM items WHERE collection_id = ?1 AND parent_id IS NULL ORDER BY id",
        )?
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut purged = 0;
    for top in tops {
        purged += crate::items::purge_subtree(tx, &top)?;
    }
    // Rows left only by a corrupt tree (a parent in another collection); the cascade handles them.
    purged += tx.execute("DELETE FROM items WHERE collection_id = ?1", [id])? as u64;
    tx.execute("DELETE FROM collections WHERE id = ?1", [id])?;
    Ok(purged)
}

/// The sort key for a collection at `position` (all collections, `exclude` left out), rebalancing when needed (which
/// bumps the `tree_version` of every collection whose key it rewrites).
fn place(
    tx: &Transaction<'_>,
    exclude: Option<&str>,
    position: Option<&Position>,
) -> Result<String> {
    let load = || -> Result<Vec<(String, String)>> {
        Ok(tx
            .prepare_cached(
                "SELECT id, sort_key FROM collections WHERE id IS NOT ?1 ORDER BY sort_key, id",
            )?
            .query_map([exclude], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    };
    // Each collection's tree response holds its `sort_key`, so a rewritten key bumps that collection's tree ETag.
    let rewrite = |fresh: &[(String, String)]| -> Result<()> {
        let mut stmt = tx.prepare_cached(
            "UPDATE collections SET sort_key = ?2, tree_version = tree_version + 1
             WHERE id = ?1 AND sort_key IS NOT ?2",
        )?;
        for (id, key) in fresh {
            stmt.execute(params![id, key])?;
        }
        Ok(())
    };
    place_rebalancing(load, rewrite, position)
}

fn check_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::validation(
            "name",
            "the name must be 1 to 100 characters",
        ));
    }
    Ok(name)
}

fn check_description(description: &str) -> Result<()> {
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(Error::validation(
            "description",
            "the description must be at most 500 characters",
        ));
    }
    Ok(())
}

fn check_color(color: &str) -> Result<()> {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color.bytes().skip(1).all(|b| b.is_ascii_hexdigit());
    if !ok {
        return Err(Error::validation("color", "the colour must be #RRGGBB"));
    }
    Ok(())
}

fn check_icon(icon: &str) -> Result<()> {
    let ok = !icon.is_empty()
        && icon.len() <= 64
        && icon
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(Error::validation(
            "icon",
            "the icon must be a lucide icon name",
        ));
    }
    Ok(())
}
