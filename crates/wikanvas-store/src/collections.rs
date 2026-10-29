//! Collections: create, edit, reorder, soft-delete and restore (owner: W1).
//!
//! A deleted collection exists only in Trash (§5 "Collection delete"): [`get_live`], [`update`], [`move_to`] and the
//! tree answer 404 for it, item creates and moves into it answer 409 `parent_invalid`, and only [`restore`] and
//! [`purge`] act on it. Deleting a collection marks its live items with the collection as their trash root; restoring
//! it clears exactly those rows, so items trashed earlier stay in Trash.
//!
//! Every [`Collection`] a function here returns carries the viewer's own access (`my_access`, D152), so each takes
//! the [`Viewer`]; which collections a viewer may see or change is `crate::access` and `shared::readable_collections`.
//! The writes take the viewer as the actor.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use wikanvas_core::api::{
    ChildOrder, Collection, CollectionAccess, CollectionPatch, DefaultAccess, ErrorCode,
    MemberAccess, NewCollection, Position, Timestamp,
};

use crate::error::{Error, Result};
use crate::shared::{Viewer, new_id, new_url_id, place_rebalancing, readable_collections};

/// The default collection colour (§5).
pub const DEFAULT_COLOR: &str = "#8D8D8D";
/// The longest collection name, after trimming (§5).
pub const MAX_NAME_CHARS: usize = 100;
/// The longest description (§5).
pub const MAX_DESCRIPTION_CHARS: usize = 500;

/// The columns [`from_row`] reads, for a query that names the collection `c`. `item_count` is the live, unarchived
/// items of a live collection, and the items its deletion trashed for a deleted one (what a restore brings back).
/// The last column is the viewer's membership: COLUMNS takes one parameter, the viewer's id, as the query's first
/// anonymous `?` (so the query uses anonymous parameters only).
const COLUMNS: &str = "c.id, c.url_id, c.name, c.description, c.icon, c.color, c.sort_key, c.child_order, c.updated_at,
    (SELECT count(*) FROM items i WHERE i.collection_id = c.id AND CASE WHEN c.deleted_at IS NULL
       THEN i.deleted_at IS NULL AND i.archived_at IS NULL ELSE i.trash_root_id = c.id END),
    c.deleted_at, c.default_access,
    (SELECT m.access FROM collection_members m WHERE m.collection_id = c.id AND m.user_id = ?)";

/// Reads a row selected with [`COLUMNS`] for `viewer` (whose id was its parameter).
fn from_row(r: &Row<'_>, viewer: &Viewer) -> rusqlite::Result<(Collection, Option<i64>)> {
    let id: String = r.get(0)?;
    let child_order: String = r.get(7)?;
    let bad = |col, e: Box<dyn std::error::Error + Send + Sync>| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, e)
    };
    let default_access: DefaultAccess = r
        .get::<_, String>(11)?
        .parse()
        .map_err(|e| bad(11, Box::new(e)))?;
    let member: Option<MemberAccess> = r
        .get::<_, Option<String>>(12)?
        .map(|a| a.parse::<MemberAccess>().map_err(|e| bad(12, Box::new(e))))
        .transpose()?;
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
            default_access,
            my_access: CollectionAccess::effective(viewer.role, default_access, member),
        },
        r.get(10)?,
    ))
}

/// Creates a collection at `new.position` among all collections (omitted: last), with `new.default_access` (omitted:
/// `write`), and gives the creator (`viewer`) a `manage` membership (D152). Validation (422, `details.field`): `name`
/// 1–100 characters after trimming, `description` at most 500, `color` `#RRGGBB`, `icon` a lucide name (1–64 of
/// `a-z0-9-`), `position` naming collections the viewer may read.
pub fn create(
    tx: &Transaction<'_>,
    viewer: &Viewer,
    new: &NewCollection,
    now: i64,
) -> Result<Collection> {
    let actor_id = viewer.user_id.as_str();
    let name = check_name(&new.name)?;
    let description = new.description.as_deref().unwrap_or("");
    check_description(description)?;
    let color = new.color.as_deref().unwrap_or(DEFAULT_COLOR);
    check_color(color)?;
    if let Some(icon) = &new.icon {
        check_icon(icon)?;
    }
    check_position(tx, viewer, new.position.as_ref())?;
    let sort_key = place(tx, None, new.position.as_ref())?;
    let id = new_id();
    let default_access = new.default_access.unwrap_or_default();
    tx.execute(
        "INSERT INTO collections (id, url_id, name, description, icon, color, sort_key, created_by, updated_by,
           created_at, updated_at, default_access)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?9, ?10)",
        params![
            id,
            new_url_id()?,
            name,
            description,
            new.icon,
            color,
            sort_key,
            actor_id,
            now,
            default_access.as_str()
        ],
    )?;
    // The creator manages it (D152). No access epoch moves: nobody had a tree of this collection to revalidate.
    tx.execute(
        "INSERT INTO collection_members (collection_id, user_id, access, created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?2, ?4, ?4)",
        params![id, actor_id, MemberAccess::Manage.as_str(), now],
    )?;
    get(tx, viewer, &id)
}

/// A collection by id, whatever its state, with the viewer's access. `not_found` when missing. It does not check that
/// the viewer may read it: the caller has (`authz::require`, or a list through `readable_collections`).
pub fn get(conn: &Connection, viewer: &Viewer, id: &str) -> Result<Collection> {
    Ok(get_with_state(conn, viewer, id)?.0)
}

/// A collection by id with its `deleted_at`. `not_found` when missing.
pub fn get_with_state(
    conn: &Connection,
    viewer: &Viewer,
    id: &str,
) -> Result<(Collection, Option<i64>)> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM collections c WHERE c.id = ?"),
        params![viewer.user_id, id],
        |r| from_row(r, viewer),
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
}

/// Whether a collection exists, and its `deleted_at`. `not_found` when missing.
fn state(conn: &Connection, id: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT deleted_at FROM collections WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
}

/// `not_found` unless the collection exists and is live.
fn ensure_live(conn: &Connection, id: &str) -> Result<()> {
    match state(conn, id)? {
        None => Ok(()),
        Some(_) => Err(Error::not_found("the collection is in the trash")),
    }
}

/// A live collection by id. `not_found` when missing or deleted (a deleted collection is only in Trash).
pub fn get_live(conn: &Connection, viewer: &Viewer, id: &str) -> Result<Collection> {
    match get_with_state(conn, viewer, id)? {
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

/// The live collections the viewer may read (D152), in sidebar order.
pub fn list(conn: &Connection, viewer: &Viewer) -> Result<Vec<Collection>> {
    let readable = readable_collections(viewer);
    let mut args = vec![Value::Text(viewer.user_id.clone())];
    args.extend(readable.args);
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM collections c WHERE c.deleted_at IS NULL AND {} ORDER BY c.sort_key, c.id",
            readable.sql
        ))?
        .query_map(params_from_iter(args.iter()), |r| Ok(from_row(r, viewer)?.0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The deleted collections the viewer may read with their `deleted_at`, most recently deleted first (Trash).
pub fn deleted(conn: &Connection, viewer: &Viewer) -> Result<Vec<(Collection, i64)>> {
    let readable = readable_collections(viewer);
    let mut args = vec![Value::Text(viewer.user_id.clone())];
    args.extend(readable.args);
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM collections c WHERE c.deleted_at IS NOT NULL AND {}
             ORDER BY c.deleted_at DESC, c.id DESC",
            readable.sql
        ))?
        .query_map(params_from_iter(args.iter()), |r| {
            let (c, deleted) = from_row(r, viewer)?;
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
/// A change sets `updated_at`/`updated_by` and bumps `tree_version` (the tree response holds the collection, and a
/// new `default_access` changes every member's and viewer's `my_access` in it); a patch that changes nothing writes
/// nothing. `not_found` when missing or deleted.
pub fn update(
    tx: &Transaction<'_>,
    viewer: &Viewer,
    id: &str,
    patch: &CollectionPatch,
    now: i64,
) -> Result<Collection> {
    let actor_id = viewer.user_id.as_str();
    let current = get_live(tx, viewer, id)?;
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
        default_access: patch.default_access.unwrap_or(current.default_access),
        ..current.clone()
    };
    if next == current {
        return Ok(current);
    }
    tx.execute(
        "UPDATE collections SET name = ?2, description = ?3, icon = ?4, color = ?5, child_order = ?6, updated_at = ?7,
           updated_by = ?8, default_access = ?9, tree_version = tree_version + 1
         WHERE id = ?1",
        params![
            id,
            next.name,
            next.description,
            next.icon,
            next.color,
            next.child_order.as_str(),
            now,
            actor_id,
            next.default_access.as_str()
        ],
    )?;
    get(tx, viewer, id)
}

/// Moves a live collection to `position` among the collections (bumping its `tree_version`: the tree response holds
/// the collection's `sort_key`). `not_found` when missing or deleted; 422 `position` for a position that names a
/// collection the viewer may not read, as for one that does not exist.
pub fn move_to(
    tx: &Transaction<'_>,
    viewer: &Viewer,
    id: &str,
    position: &Position,
    now: i64,
) -> Result<Collection> {
    ensure_live(tx, id)?;
    check_position(tx, viewer, Some(position))?;
    let key = place(tx, Some(id), Some(position))?;
    tx.execute(
        "UPDATE collections SET sort_key = ?2, updated_at = ?3, updated_by = ?4, tree_version = tree_version + 1
         WHERE id = ?1",
        params![id, key, now, viewer.user_id],
    )?;
    get(tx, viewer, id)
}

/// 422 `validation` (`field: "position"`) when the position names a collection the viewer may not read: such an id
/// is treated as one that does not exist, so a position cannot test whether a hidden collection exists.
fn check_position(conn: &Connection, viewer: &Viewer, position: Option<&Position>) -> Result<()> {
    let Some(p) = position else {
        return Ok(());
    };
    for id in [p.after_id, p.before_id].into_iter().flatten() {
        if !crate::access::collection(conn, viewer, &id.to_string())?.allows(CollectionAccess::Read)
        {
            return Err(Error::validation(
                "position",
                "the position names a row that is not a sibling",
            ));
        }
    }
    Ok(())
}

/// Soft-deletes a live collection: sets its `deleted_at` and marks its live items (archived ones included) with the
/// collection as their trash root. Returns how many items it trashed. `not_found` when missing or already deleted.
pub fn delete(tx: &Transaction<'_>, id: &str, actor_id: &str, now: i64) -> Result<u64> {
    ensure_live(tx, id)?;
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
pub fn restore(tx: &Transaction<'_>, viewer: &Viewer, id: &str, now: i64) -> Result<Collection> {
    let actor_id = viewer.user_id.as_str();
    if state(tx, id)?.is_none() {
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
    get(tx, viewer, id)
}

/// Permanently deletes a deleted collection with every item in it, leaves first (§5 "Purge"). Returns how many items
/// it deleted. `not_found` when missing; 409 `conflict` when it is not deleted.
pub fn purge(tx: &Transaction<'_>, id: &str) -> Result<u64> {
    if state(tx, id)?.is_none() {
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
