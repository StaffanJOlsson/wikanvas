//! Items: create, rename, move, duplicate, publish, archive, trash, restore and purge (owner: W1).
//!
//! W0b provides [`create`], [`rename`], [`purge_expired`] and [`summary`]; W1 completes the rest.

use std::collections::HashSet;

use quire_core::api::{ErrorCode, ItemKind, ItemSummary, NewItem};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::{Error, Result};
use crate::shared::{item_row, new_id, new_url_id, place};

/// The longest title (§5).
pub const MAX_TITLE_CHARS: usize = 1000;

/// Creates an item, published (`published_at = created_at`, D23), with its content initialized in the same
/// transaction (`docs::init_empty`, or `boards::init_empty` with the dark background when `dark`), at
/// `new.position` among its siblings (omitted: last), and bumps the collection's `tree_version`. 409 `parent_invalid`
/// when the collection is missing or deleted, or the parent is missing, trashed, archived or in another collection;
/// 422 `validation` for a title over 1000 characters (`field: "title"`) or a position naming a non-sibling.
pub fn create(
    tx: &Transaction<'_>,
    actor_id: &str,
    new: &NewItem,
    dark: bool,
    now: i64,
) -> Result<ItemSummary> {
    let title = new.title.as_deref().unwrap_or("");
    check_title(title)?;
    let collection_id = new.collection_id.to_string();
    let live: Option<bool> = tx
        .query_row(
            "SELECT deleted_at IS NULL FROM collections WHERE id = ?1",
            [&collection_id],
            |r| r.get(0),
        )
        .optional()?;
    match live {
        None => return Err(Error::new(ErrorCode::ParentInvalid, "no such collection")),
        Some(false) => {
            return Err(Error::new(
                ErrorCode::ParentInvalid,
                "the collection is in the trash; restore the collection first",
            ));
        }
        Some(true) => {}
    }
    let parent_id = new.parent_id.map(|p| p.to_string());
    if let Some(parent) = &parent_id {
        let ok: Option<bool> = tx
            .query_row(
                "SELECT collection_id = ?2 AND deleted_at IS NULL AND archived_at IS NULL FROM items WHERE id = ?1",
                params![parent, collection_id],
                |r| r.get(0),
            )
            .optional()?;
        if ok != Some(true) {
            return Err(Error::new(
                ErrorCode::ParentInvalid,
                "the parent is missing, in the trash, archived or in another collection",
            ));
        }
    }
    let siblings = tx
        .prepare(
            "SELECT id, sort_key FROM items WHERE collection_id = ?1 AND parent_id IS ?2 ORDER BY sort_key, id",
        )?
        .query_map(params![collection_id, parent_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let sort_key = place(&siblings, new.position.as_ref())?;
    let id = new_id();
    tx.execute(
        "INSERT INTO items (id, url_id, collection_id, parent_id, kind, title, sort_key, created_by, updated_by,
           created_at, updated_at, published_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?9, ?9)",
        params![
            id,
            new_url_id()?,
            collection_id,
            parent_id,
            new.kind.as_str(),
            title,
            sort_key,
            actor_id,
            now
        ],
    )?;
    match new.kind {
        ItemKind::Doc => crate::docs::init_empty(tx, &id, now)?,
        ItemKind::Board => crate::boards::init_empty(tx, &id, dark, now)?,
    }
    bump_tree_version(tx, &collection_id)?;
    summary(tx, &id)
}

/// Renames an item: sets the title, `updated_at`, `updated_by`, bumps `meta_version` (the `"m{n}"` ETag) and the
/// collection's `tree_version`. The same title again changes nothing. `not_found`; 409 `read_only` when trashed or
/// archived; 422 `validation` (`field: "title"`) over 1000 characters.
pub fn rename(
    tx: &Transaction<'_>,
    item_id: &str,
    title: &str,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    check_title(title)?;
    let row = item_row(tx, item_id)?;
    if row.deleted_at.is_some() || row.archived_at.is_some() {
        return Err(Error::new(
            ErrorCode::ReadOnly,
            "the item is in the trash or archived",
        ));
    }
    if row.title != title {
        tx.execute(
            "UPDATE items SET title = ?2, updated_at = ?3, updated_by = ?4, meta_version = meta_version + 1
             WHERE id = ?1",
            params![item_id, title, now, actor_id],
        )?;
        bump_tree_version(tx, &row.collection_id)?;
    }
    summary(tx, item_id)
}

/// Permanently deletes the Trash roots deleted before `older_than` (ms): item roots (`trash_root_id = id`) with their
/// whole subtree, then deleted collections with all their items, then the collection rows. Always leaves first, one
/// DELETE per depth level (§5 "Purge"), so the FK cascade depth limit never matters. Returns the number of items
/// deleted.
pub fn purge_expired(tx: &Transaction<'_>, older_than: i64) -> Result<u64> {
    let mut purged = 0;
    let roots = ids(
        tx,
        "SELECT id FROM items WHERE trash_root_id = id AND deleted_at < ?1 ORDER BY id",
        older_than,
    )?;
    for root in roots {
        purged += purge_subtrees(tx, vec![root])?;
    }
    let collections = ids(
        tx,
        "SELECT id FROM collections WHERE deleted_at IS NOT NULL AND deleted_at < ?1 ORDER BY id",
        older_than,
    )?;
    for collection in collections {
        let tops = tx
            .prepare("SELECT id FROM items WHERE collection_id = ?1 AND parent_id IS NULL")?
            .query_map([&collection], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        purged += purge_subtrees(tx, tops)?;
        // Rows left only by a corrupt tree (a parent in another collection); the cascade handles them.
        purged += tx.execute("DELETE FROM items WHERE collection_id = ?1", [&collection])? as u64;
        tx.execute("DELETE FROM collections WHERE id = ?1", [&collection])?;
    }
    Ok(purged)
}

/// The API shape of an item, with its count of live, unarchived children.
pub fn summary(conn: &Connection, item_id: &str) -> Result<ItemSummary> {
    let row = item_row(conn, item_id)?;
    let children: i64 = conn.query_row(
        "SELECT count(*) FROM items WHERE parent_id = ?1 AND deleted_at IS NULL AND archived_at IS NULL",
        [item_id],
        |r| r.get(0),
    )?;
    row.summary(children as u64)
}

/// Bumps a collection's `tree_version` (the `/tree` ETag).
pub fn bump_tree_version(tx: &Transaction<'_>, collection_id: &str) -> Result<()> {
    tx.execute(
        "UPDATE collections SET tree_version = tree_version + 1 WHERE id = ?1",
        [collection_id],
    )?;
    Ok(())
}

fn check_title(title: &str) -> Result<()> {
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(Error::validation(
            "title",
            "the title must be at most 1000 characters",
        ));
    }
    Ok(())
}

fn ids(conn: &Connection, sql: &str, older_than: i64) -> Result<Vec<String>> {
    Ok(conn
        .prepare(sql)?
        .query_map([older_than], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Deletes `roots` and everything below them, deepest level first. Returns the number of rows deleted.
fn purge_subtrees(tx: &Transaction<'_>, roots: Vec<String>) -> Result<u64> {
    if roots.is_empty() {
        return Ok(0);
    }
    let mut seen: HashSet<String> = roots.iter().cloned().collect();
    let mut levels = vec![roots];
    loop {
        let parents = serde_json::to_string(levels.last().expect("at least the roots"))?;
        let next: Vec<String> = tx
            .prepare_cached(
                "SELECT id FROM items WHERE parent_id IN (SELECT value FROM json_each(?1))",
            )?
            .query_map([parents], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if next.is_empty() {
            break;
        }
        levels.push(next);
    }
    let mut deleted = 0;
    for level in levels.iter().rev() {
        deleted += tx
            .prepare_cached("DELETE FROM items WHERE id IN (SELECT value FROM json_each(?1))")?
            .execute([serde_json::to_string(level)?])? as u64;
    }
    Ok(deleted)
}
