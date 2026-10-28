//! Items: create, rename, move, duplicate, publish, archive, trash, restore and purge (owner: W1).
//!
//! The tree rules are those of `quire_core::tree` (cycle, parent, read-only, placement on restore), applied in SQL
//! inside the writer transaction; the `0001-init` triggers repeat the parent rules as a guard. Every create sets
//! `published_at = created_at` (D23); a draft exists only after [`unpublish`]. Every operation that changes the tree
//! bumps `tree_version` on each affected collection (the `/tree` ETag); every change to an item row other than a
//! content save bumps its `meta_version` (the `"m{n}"` ETag of `GET /items/{id}`).

use quire_core::api::{
    BoardMeta, BoardPageMeta, ErrorCode, ItemDetail, ItemKind, ItemPatch, ItemSummary, ItemText,
    MoveItem, NewItem, Position, RestoredItem, RevMismatchDetails, Timestamp,
};
use quire_core::time::{DISCARD_WINDOW_MS, purge_at};
use quire_core::tree::{ParentState, check_move, check_new_parent, keeps_parent};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::shared::{ItemRow, Viewer, item_row, json_ids, new_id, new_url_id};
use crate::tree;

/// The longest title (§5).
pub const MAX_TITLE_CHARS: usize = 1000;

/// The suffix [`duplicate`] adds to the copy's title.
pub const COPY_SUFFIX: &str = " (copy)";

/// The `items` row a path names: its id (a UUID) or its `url_id` (`GET /items/{id}`), whatever its state.
/// `not_found` when there is none.
pub fn find(conn: &Connection, id_or_url_id: &str) -> Result<ItemRow> {
    if Uuid::parse_str(id_or_url_id).is_ok() {
        return item_row(conn, id_or_url_id);
    }
    if !quire_core::ids::is_url_id(id_or_url_id) {
        return Err(Error::not_found("no such item"));
    }
    conn.query_row(
        &format!(
            "SELECT {} FROM items i WHERE i.url_id = ?1",
            ItemRow::COLUMNS
        ),
        [id_or_url_id],
        ItemRow::from_row,
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such item"))
}

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
    ensure_target_collection(tx, &collection_id)?;
    let parent_id = new.parent_id.map(|p| p.to_string());
    if let Some(parent) = &parent_id {
        check_new_parent(parent_state(tx, parent, &collection_id)?)?;
    }
    let sort_key = tree::place_item(
        tx,
        &collection_id,
        parent_id.as_deref(),
        None,
        new.position.as_ref(),
    )?;
    let id = insert(
        tx,
        &collection_id,
        parent_id.as_deref(),
        new.kind,
        title,
        &sort_key,
        false,
        actor_id,
        now,
    )?;
    match new.kind {
        ItemKind::Doc => crate::docs::init_empty(tx, &id, now)?,
        ItemKind::Board => crate::boards::init_empty(tx, &id, dark, now)?,
    }
    bump_tree_version(tx, &collection_id)?;
    summary(tx, &id)
}

/// Inserts an item row, published on create; returns its id.
#[allow(clippy::too_many_arguments)]
fn insert(
    tx: &Transaction<'_>,
    collection_id: &str,
    parent_id: Option<&str>,
    kind: ItemKind,
    title: &str,
    sort_key: &str,
    full_width: bool,
    actor_id: &str,
    now: i64,
) -> Result<String> {
    let id = new_id();
    tx.execute(
        "INSERT INTO items (id, url_id, collection_id, parent_id, kind, title, sort_key, full_width, created_by,
           updated_by, created_at, updated_at, published_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?10, ?10)",
        params![id, new_url_id()?, collection_id, parent_id, kind.as_str(), title, sort_key, full_width, actor_id, now],
    )?;
    Ok(id)
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
    update(
        tx,
        item_id,
        &ItemPatch {
            title: Some(title.to_owned()),
            full_width: None,
        },
        None,
        actor_id,
        now,
    )
}

/// `PATCH /items/{id}`: the title and/or `full_width`. With `if_match` (the `n` of `If-Match: "m{n}"`), a stale
/// `meta_version` is 412 `rev_mismatch` with `details.etag` the current tag. A change sets `updated_at`/`updated_by`
/// and bumps `meta_version` and `tree_version` once (the tree shows `updated_at` and may sort by it, so a
/// `full_width` change alone bumps it too). A patch that changes nothing writes nothing. `not_found`; 409
/// `read_only` when trashed or archived (and the patch sets a field); 422 `title`.
pub fn update(
    tx: &Transaction<'_>,
    item_id: &str,
    patch: &ItemPatch,
    if_match: Option<i64>,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    let row = item_row(tx, item_id)?;
    if let Some(expected) = if_match
        && expected != row.meta_version
    {
        return Err(rev_mismatch(row.meta_version));
    }
    if let Some(title) = &patch.title {
        check_title(title)?;
    }
    if patch.title.is_none() && patch.full_width.is_none() {
        return summary(tx, item_id);
    }
    row.ensure_live()?;
    let title = patch.title.as_deref().unwrap_or(&row.title);
    let full_width = patch.full_width.unwrap_or(row.full_width);
    if title == row.title && full_width == row.full_width {
        return summary(tx, item_id);
    }
    tx.execute(
        "UPDATE items SET title = ?2, full_width = ?3, updated_at = ?4, updated_by = ?5,
           meta_version = meta_version + 1
         WHERE id = ?1",
        params![item_id, title, full_width, now, actor_id],
    )?;
    // The tree shows the title and `updated_at`, and sorts by `updated_at` under the `updated` order.
    bump_tree_version(tx, &row.collection_id)?;
    summary(tx, item_id)
}

/// The 412 of a stale `If-Match` on item metadata (D45).
pub fn rev_mismatch(meta_version: i64) -> Error {
    Error::new(ErrorCode::RevMismatch, "the item changed since that ETag").with_details(
        &RevMismatchDetails {
            etag: format!("\"m{meta_version}\""),
            rev: None,
            mergeable: None,
        },
    )
}

/// Moves an item with its subtree: under `mv.parent_id` (or to the top level) in `mv.collection_id` (or its own
/// collection), at `mv.position` among the new siblings (omitted: last). Across collections the root's parent,
/// collection and key change in one statement, then the descendants' `collection_id` one depth level per statement,
/// top-down, trashed and archived descendants included; descendants keep their `sort_key`. Bumps `tree_version` on
/// both collections. 409 `read_only` (the item is trashed or archived), `parent_invalid` (§5), `cycle` (the target is
/// the item or below it); 422 `position`.
pub fn move_item(
    tx: &Transaction<'_>,
    item_id: &str,
    mv: &MoveItem,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    let row = item_row(tx, item_id)?;
    row.ensure_live()?;
    let collection = mv
        .collection_id
        .map_or_else(|| row.collection_id.clone(), |c| c.to_string());
    ensure_target_collection(tx, &collection)?;
    let parent = mv.parent_id.map(|p| p.to_string());
    if let Some(p) = &parent {
        check_new_parent(parent_state(tx, p, &collection)?)?;
        let mut chain = vec![p.clone()];
        chain.extend(tree::ancestors(tx, p)?);
        check_move(&row.id, Some(chain.as_slice()))?;
    }
    let key = tree::place_item(
        tx,
        &collection,
        parent.as_deref(),
        Some(item_id),
        mv.position.as_ref(),
    )?;
    if collection == row.collection_id {
        tx.execute(
            "UPDATE items SET parent_id = ?2, sort_key = ?3, updated_at = ?4, updated_by = ?5,
               meta_version = meta_version + 1
             WHERE id = ?1",
            params![item_id, parent, key, now, actor_id],
        )?;
        bump_tree_version(tx, &collection)?;
    } else {
        let levels = tree::levels(tx, item_id)?;
        tx.execute(
            "UPDATE items SET parent_id = ?2, collection_id = ?3, sort_key = ?4, updated_at = ?5, updated_by = ?6,
               meta_version = meta_version + 1
             WHERE id = ?1",
            params![item_id, parent, collection, key, now, actor_id],
        )?;
        let mut stmt = tx.prepare_cached(
            "UPDATE items SET collection_id = ?1, meta_version = meta_version + 1
             WHERE id IN (SELECT value FROM json_each(?2))",
        )?;
        for level in levels.iter().skip(1) {
            stmt.execute(params![collection, json_ids(level)?])?;
        }
        bump_tree_version(tx, &row.collection_id)?;
        bump_tree_version(tx, &collection)?;
    }
    summary(tx, item_id)
}

/// Duplicates a live, unarchived item right after it among its siblings: the title with [`COPY_SUFFIX`] (an empty
/// title stays empty), `full_width`, and the content through `docs::copy` / `boards::copy` (the Y state or board
/// bytes, `item_attachments`, the indexed text). Children are not copied; the copy is published (D23). 409
/// `read_only` when the item is trashed or archived.
pub fn duplicate(
    tx: &Transaction<'_>,
    item_id: &str,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    let row = item_row(tx, item_id)?;
    row.ensure_live()?;
    let title = copy_title(&row.title);
    let after = Position {
        after_id: Some(Uuid::parse_str(&row.id).map_err(|e| Error::internal("stored item id", e))?),
        before_id: None,
    };
    let key = tree::place_item(
        tx,
        &row.collection_id,
        row.parent_id.as_deref(),
        None,
        Some(&after),
    )?;
    let id = insert(
        tx,
        &row.collection_id,
        row.parent_id.as_deref(),
        row.kind,
        &title,
        &key,
        row.full_width,
        actor_id,
        now,
    )?;
    match row.kind {
        ItemKind::Doc => crate::docs::copy(tx, item_id, &id, now)?,
        ItemKind::Board => crate::boards::copy(tx, item_id, &id, now)?,
    }
    bump_tree_version(tx, &row.collection_id)?;
    summary(tx, &id)
}

/// The title of a duplicate: `title` + [`COPY_SUFFIX`], cut to [`MAX_TITLE_CHARS`]; an empty title stays empty.
pub fn copy_title(title: &str) -> String {
    if title.is_empty() {
        return String::new();
    }
    let keep = MAX_TITLE_CHARS - COPY_SUFFIX.chars().count();
    let mut out: String = title.chars().take(keep).collect();
    out.push_str(COPY_SUFFIX);
    out
}

/// Publishes a live, unarchived item (and, `with_ancestors`, every draft ancestor): `published_at = now` where it is
/// unset. Publishing a published item changes nothing. 409 `read_only` when trashed or archived.
pub fn publish(
    tx: &Transaction<'_>,
    item_id: &str,
    with_ancestors: bool,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    let row = item_row(tx, item_id)?;
    row.ensure_live()?;
    let mut ids = vec![row.id.clone()];
    if with_ancestors {
        ids.extend(tree::ancestors(tx, item_id)?);
    }
    let n = tx.execute(
        "UPDATE items SET published_at = ?2, updated_at = ?2, updated_by = ?3, meta_version = meta_version + 1
         WHERE id IN (SELECT value FROM json_each(?1)) AND published_at IS NULL",
        params![json_ids(&ids)?, now, actor_id],
    )?;
    if n > 0 {
        bump_tree_version(tx, &row.collection_id)?;
    }
    summary(tx, item_id)
}

/// Makes a live, unarchived item a draft (`published_at = NULL`), whatever its parent or children are (D23: the
/// visibility is derived, so there is nothing to enforce). Unpublishing a draft changes nothing. 409 `read_only` when
/// trashed or archived.
pub fn unpublish(
    tx: &Transaction<'_>,
    item_id: &str,
    actor_id: &str,
    now: i64,
) -> Result<ItemSummary> {
    let row = item_row(tx, item_id)?;
    row.ensure_live()?;
    let n = tx.execute(
        "UPDATE items SET published_at = NULL, updated_at = ?2, updated_by = ?3, meta_version = meta_version + 1
         WHERE id = ?1 AND published_at IS NOT NULL",
        params![item_id, now, actor_id],
    )?;
    if n > 0 {
        bump_tree_version(tx, &row.collection_id)?;
    }
    summary(tx, item_id)
}

/// Archives a live, unarchived item and every descendant that is neither archived nor trashed, with the item as
/// their archive root. Returns how many rows it archived. 409 `read_only` when trashed or already archived.
pub fn archive(tx: &Transaction<'_>, item_id: &str, now: i64) -> Result<u64> {
    let row = item_row(tx, item_id)?;
    row.ensure_live()?;
    let ids = tree::subtree(tx, item_id)?;
    let n = tx.execute(
        "UPDATE items SET archived_at = ?2, archive_root_id = ?1, meta_version = meta_version + 1
         WHERE id IN (SELECT value FROM json_each(?3)) AND archived_at IS NULL AND deleted_at IS NULL",
        params![item_id, now, json_ids(&ids)?],
    )?;
    bump_tree_version(tx, &row.collection_id)?;
    Ok(n as u64)
}

/// Unarchives the archive unit of an item (its archive root, which may be an ancestor): clears exactly the rows of
/// that root. The root keeps its parent unless the parent is gone (purged, trashed or archived); then it goes to the
/// end of the top level (`moved_to_top`). Returns the root. 409 `conflict` when the item is not archived,
/// `read_only` when it or its root is in the trash.
pub fn unarchive(tx: &Transaction<'_>, item_id: &str) -> Result<RestoredItem> {
    let row = item_row(tx, item_id)?;
    if row.trashed() {
        return Err(read_only());
    }
    let root = row
        .archive_root_id
        .clone()
        .ok_or_else(|| Error::new(ErrorCode::Conflict, "the item is not archived"))?;
    if item_row(tx, &root)?.trashed() {
        return Err(read_only());
    }
    tx.execute(
        "UPDATE items SET archived_at = NULL, archive_root_id = NULL, meta_version = meta_version + 1
         WHERE archive_root_id = ?1",
        [&root],
    )?;
    let moved_to_top = place_root(tx, &root)?;
    bump_tree_version(tx, &row.collection_id)?;
    Ok(RestoredItem {
        item: summary(tx, &root)?,
        moved_to_top,
    })
}

/// Moves an item to the trash with every descendant that is not trashed yet, with the item as their trash root
/// (archived ones included; they keep their archive state). Returns how many rows it trashed. 409 `read_only` when it
/// is already in the trash.
pub fn trash(tx: &Transaction<'_>, item_id: &str, actor_id: &str, now: i64) -> Result<u64> {
    let row = item_row(tx, item_id)?;
    if row.trashed() {
        return Err(read_only());
    }
    let ids = tree::subtree(tx, item_id)?;
    let n = tx.execute(
        "UPDATE items SET deleted_at = ?2, deleted_by = ?3, trash_root_id = ?1, meta_version = meta_version + 1
         WHERE id IN (SELECT value FROM json_each(?4)) AND deleted_at IS NULL",
        params![item_id, now, actor_id, json_ids(&ids)?],
    )?;
    bump_tree_version(tx, &row.collection_id)?;
    Ok(n as u64)
}

/// Restores the trash unit of an item (its trash root, which may be an ancestor): clears exactly the rows of that
/// root. The root keeps its parent unless the parent is gone (purged, trashed, or archived while the root is not);
/// then it goes to the end of the top level (`moved_to_top`), and if it is still archived it becomes its own archive
/// root ([`place_root`]). Returns the root. 409 `conflict` when the item is not
/// in the trash; `parent_invalid` while its collection is deleted ("restore the collection first").
pub fn restore(tx: &Transaction<'_>, item_id: &str) -> Result<RestoredItem> {
    let row = item_row(tx, item_id)?;
    let root = row
        .trash_root_id
        .clone()
        .ok_or_else(|| Error::new(ErrorCode::Conflict, "the item is not in the trash"))?;
    let collection_deleted: bool = tx.query_row(
        "SELECT deleted_at IS NOT NULL FROM collections WHERE id = ?1",
        [&row.collection_id],
        |r| r.get(0),
    )?;
    if root == row.collection_id || collection_deleted {
        return Err(Error::new(
            ErrorCode::ParentInvalid,
            "the collection is in the trash; restore the collection first",
        ));
    }
    tx.execute(
        "UPDATE items SET deleted_at = NULL, deleted_by = NULL, trash_root_id = NULL, meta_version = meta_version + 1
         WHERE trash_root_id = ?1",
        [&root],
    )?;
    let moved_to_top = place_root(tx, &root)?;
    bump_tree_version(tx, &row.collection_id)?;
    Ok(RestoredItem {
        item: summary(tx, &root)?,
        moved_to_top,
    })
}

/// The placement rule after a restore or unarchive (`quire_core::tree::keeps_parent`): moves `root` to the end of its
/// collection's top level when its parent is gone. Returns whether it moved.
///
/// A restored root that stays archived as part of an ancestor's archive unit (D5: trashing an archived item keeps
/// its archive state) leaves that ancestor's subtree when it moves, so the rows of its subtree in that unit become a
/// unit of their own, with the root as their archive root: it then shows in `/archive` and unarchives by itself, and
/// a purge of the old root cannot leave it pointing at a missing row (`quire_core::tree::Model::place`).
fn place_root(tx: &Transaction<'_>, root: &str) -> Result<bool> {
    let row = item_row(tx, root)?;
    let Some(parent) = &row.parent_id else {
        return Ok(false);
    };
    if keeps_parent(
        parent_state(tx, parent, &row.collection_id)?,
        row.archived(),
    ) {
        return Ok(false);
    }
    let key = tree::place_item(tx, &row.collection_id, None, Some(root), None)?;
    tx.execute(
        "UPDATE items SET parent_id = NULL, sort_key = ?2 WHERE id = ?1",
        params![root, key],
    )?;
    if let Some(unit) = row.archive_root_id.as_deref().filter(|&u| u != root) {
        tx.execute(
            "UPDATE items SET archive_root_id = ?1, meta_version = meta_version + 1
             WHERE archive_root_id = ?2 AND id IN (SELECT value FROM json_each(?3))",
            params![root, unit, json_ids(&tree::subtree(tx, root)?)?],
        )?;
    }
    Ok(true)
}

/// `DELETE /items/{id}?discard_empty=true`: deletes the item permanently when it was created less than 24 h ago and
/// has no children (in any state), a blank title and an empty body (`docs::is_empty` / `boards::is_empty`), whether
/// or not it is published. Otherwise 409 `not_empty`. Returns 1.
pub fn discard_empty(tx: &Transaction<'_>, item_id: &str, now: i64) -> Result<u64> {
    let row = item_row(tx, item_id)?;
    let has_children: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM items WHERE parent_id = ?1)",
        [item_id],
        |r| r.get(0),
    )?;
    let discardable = now - row.created_at < DISCARD_WINDOW_MS
        && !has_children
        && row.title.trim().is_empty()
        && match row.kind {
            ItemKind::Doc => crate::docs::is_empty(tx, item_id)?,
            ItemKind::Board => crate::boards::is_empty(tx, item_id)?,
        };
    if !discardable {
        return Err(Error::new(
            ErrorCode::NotEmpty,
            "only a new (under 24 h), childless, untitled and empty item can be discarded; move it to the trash instead",
        ));
    }
    tx.execute("DELETE FROM items WHERE id = ?1", [item_id])?;
    bump_tree_version(tx, &row.collection_id)?;
    Ok(1)
}

/// Permanently deletes an item in the trash with its whole subtree, leaves first. Returns how many rows went. 409
/// `conflict` when the item is not in the trash.
pub fn purge(tx: &Transaction<'_>, item_id: &str) -> Result<u64> {
    if !item_row(tx, item_id)?.trashed() {
        return Err(Error::new(
            ErrorCode::Conflict,
            "only an item in the trash can be purged; move it to the trash first",
        ));
    }
    purge_subtree(tx, item_id)
}

/// Empties the trash: every item root and every deleted collection, leaves first. Returns how many items went.
pub fn purge_all(tx: &Transaction<'_>) -> Result<u64> {
    purge_expired(tx, i64::MAX)
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
        purged += purge_subtree(tx, &root)?;
    }
    let collections = ids(
        tx,
        "SELECT id FROM collections WHERE deleted_at IS NOT NULL AND deleted_at < ?1 ORDER BY id",
        older_than,
    )?;
    for collection in collections {
        purged += crate::collections::purge_rows(tx, &collection)?;
    }
    Ok(purged)
}

/// Deletes `root` and everything below it, deepest level first. Returns the number of rows deleted.
pub(crate) fn purge_subtree(tx: &Transaction<'_>, root: &str) -> Result<u64> {
    let levels = tree::levels(tx, root)?;
    let mut stmt =
        tx.prepare_cached("DELETE FROM items WHERE id IN (SELECT value FROM json_each(?1))")?;
    let mut deleted = 0;
    for level in levels.iter().rev() {
        deleted += stmt.execute([json_ids(level)?])? as u64;
    }
    Ok(deleted)
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

/// `GET /items/{id}`: the item in any state (a trashed or archived item's page shows a banner), its collection,
/// breadcrumbs, star, effective publication, trash and archive dates, board meta and `meta_version`.
pub fn detail(conn: &Connection, viewer: &Viewer, row: &ItemRow) -> Result<ItemDetail> {
    Ok(ItemDetail {
        item: summary(conn, &row.id)?,
        collection: crate::collections::get(conn, &row.collection_id)?,
        breadcrumbs: tree::breadcrumbs(conn, &row.id)?,
        starred: crate::stars::is_starred(conn, &viewer.user_id, &row.id)?,
        effectively_published: crate::shared::effectively_published(conn, &row.id)?,
        trashed_at: row.deleted_at.map(Timestamp),
        purge_at: row.deleted_at.map(|d| Timestamp(purge_at(d))),
        archived_at: row.archived_at.map(Timestamp),
        board: match row.kind {
            ItemKind::Board => board_meta(conn, &row.id)?,
            ItemKind::Doc => None,
        },
        meta_version: row.meta_version,
    })
}

/// `ItemDetail.board`: `board_state.meta` (§5) with the revision, plus the stored previews: `svg_len` of each page's
/// preview and `preview_rev`, the oldest revision among them (`null` before the first preview).
pub fn board_meta(conn: &Connection, item_id: &str) -> Result<Option<BoardMeta>> {
    let Some(rev) = conn
        .query_row(
            "SELECT rev FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let meta = crate::boards::meta(conn, item_id)?;
    let previews = conn
        .prepare_cached("SELECT page_index, rev, svg_len FROM board_previews WHERE item_id = ?1")?
        .query_map([item_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let pages = meta
        .pages
        .into_iter()
        .enumerate()
        .map(|(i, p)| BoardPageMeta {
            id: p.id,
            name: p.name,
            svg_len: previews
                .iter()
                .find(|(page, _, _)| *page == i as i64)
                .map(|(_, _, len)| *len as u64),
        })
        .collect();
    Ok(Some(BoardMeta {
        rev,
        pages,
        objects: meta.objects,
        background: meta.background,
        bytes: meta.bytes,
        preview_rev: previews.iter().map(|(_, r, _)| *r).min(),
        text_excerpt: meta.text_excerpt,
    }))
}

/// `GET /items/{id}/text`: the derived plain text search indexes (§6.1, §6.2), in any state.
pub fn text(conn: &Connection, item_id: &str) -> Result<ItemText> {
    let row = item_row(conn, item_id)?;
    let (title, body): (String, String) = conn
        .query_row(
            "SELECT title, body FROM item_text WHERE item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or_else(|| (row.title.clone(), String::new()));
    Ok(ItemText {
        kind: row.kind,
        title,
        body,
        updated_at: Timestamp(row.updated_at),
    })
}

/// Bumps a collection's `tree_version` (the `/tree` ETag).
pub fn bump_tree_version(tx: &Transaction<'_>, collection_id: &str) -> Result<()> {
    tx.execute(
        "UPDATE collections SET tree_version = tree_version + 1 WHERE id = ?1",
        [collection_id],
    )?;
    Ok(())
}

/// 409 `parent_invalid` unless the collection exists and is live.
fn ensure_target_collection(conn: &Connection, collection_id: &str) -> Result<()> {
    let live: Option<bool> = conn
        .query_row(
            "SELECT deleted_at IS NULL FROM collections WHERE id = ?1",
            [collection_id],
            |r| r.get(0),
        )
        .optional()?;
    match live {
        None => Err(Error::new(ErrorCode::ParentInvalid, "no such collection")),
        Some(false) => Err(Error::new(
            ErrorCode::ParentInvalid,
            "the collection is in the trash; restore the collection first",
        )),
        Some(true) => Ok(()),
    }
}

/// The state of `parent` as a would-be parent in `collection_id`: `None` when it is missing or in another collection.
fn parent_state(
    conn: &Connection,
    parent: &str,
    collection_id: &str,
) -> Result<Option<ParentState>> {
    Ok(conn
        .query_row(
            "SELECT deleted_at IS NOT NULL, archived_at IS NOT NULL FROM items WHERE id = ?1 AND collection_id = ?2",
            params![parent, collection_id],
            |r| {
                Ok(ParentState {
                    trashed: r.get(0)?,
                    archived: r.get(1)?,
                })
            },
        )
        .optional()?)
}

fn read_only() -> Error {
    Error::new(ErrorCode::ReadOnly, "the item is in the trash or archived")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_titles() {
        assert_eq!(copy_title("Plan"), "Plan (copy)");
        assert_eq!(copy_title(""), "");
        let long = "é".repeat(MAX_TITLE_CHARS);
        let copied = copy_title(&long);
        assert_eq!(copied.chars().count(), MAX_TITLE_CHARS);
        assert!(copied.ends_with(COPY_SUFFIX));
    }
}
