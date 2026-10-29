//! Tree queries: breadcrumbs, ancestors, subtree levels, the collection tree, children and sibling placement
//! (owner: W1).
//!
//! §5 "Tree queries": every walk is a recursive CTE with `UNION` (never `UNION ALL`), so it terminates even on a
//! corrupt cycle; ordering and depth are worked out in Rust from the rows the CTE returns. The tree endpoint lists a
//! collection's visible, unarchived items flat, parents before children, siblings in the collection's `child_order`
//! (D21); a non-manual order affects that output only, never `sort_key`.

use std::collections::{HashMap, HashSet};

use wikanvas_core::api::{ChildOrder, CollectionTree, Crumb, ItemSummary, Position};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::{Error, Result};
use crate::shared::{ItemRow, Viewer, item_row, parse_uuid, place_rebalancing, visible_items};

/// SQL for an item's visible child count (`ItemSummary.child_count`: live, unarchived children; children share the
/// item's collection, and a deleted collection's items are all trashed), for a query that names the item `i`.
pub const CHILD_COUNT: &str = "(SELECT count(*) FROM items ch WHERE ch.parent_id = i.id AND ch.deleted_at IS NULL AND ch.archived_at IS NULL)";

/// The ancestors of an item, its parent first, up to the top level (any state). Empty for a top-level or unknown
/// item.
pub fn ancestors(conn: &Connection, item_id: &str) -> Result<Vec<String>> {
    Ok(ancestor_rows(conn, item_id)?
        .into_iter()
        .map(|c| c.id.to_string())
        .collect())
}

/// The breadcrumbs of an item: its ancestors, root first, the item itself excluded.
pub fn breadcrumbs(conn: &Connection, item_id: &str) -> Result<Vec<Crumb>> {
    let mut chain = ancestor_rows(conn, item_id)?;
    chain.reverse();
    Ok(chain)
}

/// The ancestor chain as crumbs, parent first.
fn ancestor_rows(conn: &Connection, item_id: &str) -> Result<Vec<Crumb>> {
    let mut stmt = conn.prepare_cached(
        "WITH RECURSIVE anc(id, parent_id) AS (
           SELECT id, parent_id FROM items WHERE id = ?1
           UNION
           SELECT i.id, i.parent_id FROM items i JOIN anc ON i.id = anc.parent_id
         )
         SELECT i.id, i.parent_id, i.url_id, i.kind, i.title FROM anc JOIN items i ON i.id = anc.id",
    )?;
    let mut rows: HashMap<String, (Option<String>, Option<Crumb>)> = HashMap::new();
    let mut q = stmt.query([item_id])?;
    while let Some(r) = q.next()? {
        let id: String = r.get(0)?;
        let kind: String = r.get(3)?;
        let crumb = Crumb {
            id: parse_uuid(&id)?,
            url_id: r.get(2)?,
            kind: kind
                .parse()
                .map_err(|e| Error::internal("stored item kind", e))?,
            title: r.get(4)?,
        };
        rows.insert(id, (r.get(1)?, Some(crumb)));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::from([item_id.to_owned()]);
    let mut cur = rows.get(item_id).and_then(|(p, _)| p.clone());
    while let Some(p) = cur {
        if !seen.insert(p.clone()) {
            break; // a corrupt cycle
        }
        let Some((parent, crumb)) = rows.get_mut(&p) else {
            break;
        };
        out.extend(crumb.take());
        cur = parent.clone();
    }
    Ok(out)
}

/// `root` and every row below it (any state), level by level: `[[root], children, grandchildren, …]`. Empty when
/// `root` does not exist.
pub fn levels(conn: &Connection, root: &str) -> Result<Vec<Vec<String>>> {
    let mut stmt = conn.prepare_cached(
        "WITH RECURSIVE sub(id) AS (
           SELECT id FROM items WHERE id = ?1
           UNION
           SELECT i.id FROM items i JOIN sub ON i.parent_id = sub.id
         )
         SELECT i.id, i.parent_id FROM sub JOIN items i ON i.id = sub.id",
    )?;
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut found = false;
    let mut q = stmt.query([root])?;
    while let Some(r) = q.next()? {
        let id: String = r.get(0)?;
        let parent: Option<String> = r.get(1)?;
        found |= id == root;
        if let Some(p) = parent {
            children.entry(p).or_default().push(id);
        }
    }
    if !found {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::from([root.to_owned()]);
    let mut out = vec![vec![root.to_owned()]];
    loop {
        let next: Vec<String> = out
            .last()
            .expect("the root level")
            .iter()
            .flat_map(|p| children.get(p).into_iter().flatten())
            .filter(|c| seen.insert((*c).clone()))
            .cloned()
            .collect();
        if next.is_empty() {
            return Ok(out);
        }
        out.push(next);
    }
}

/// `root` and every row below it, parents before children.
pub fn subtree(conn: &Connection, root: &str) -> Result<Vec<String>> {
    Ok(levels(conn, root)?.into_iter().flatten().collect())
}

/// The `tree_version` of a live collection (the `/tree` ETag is `"t{tree_version}.{viewer_epoch}"`). `not_found`
/// when it is missing or deleted.
pub fn tree_version(conn: &Connection, collection_id: &str) -> Result<i64> {
    conn.query_row(
        "SELECT tree_version FROM collections WHERE id = ?1 AND deleted_at IS NULL",
        [collection_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such collection"))
}

/// The collection's visible, unarchived items, flat: parents before children, siblings in `child_order` (D21).
/// `not_found` for a missing or deleted collection.
pub fn collection_tree(
    conn: &Connection,
    viewer: &Viewer,
    collection_id: &str,
) -> Result<CollectionTree> {
    let collection = crate::collections::get_live(conn, collection_id)?;
    let sql = format!(
        "SELECT {} FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE i.collection_id = ?1 AND {} AND i.archived_at IS NULL",
        ItemRow::COLUMNS,
        visible_items(viewer)
    );
    let rows = conn
        .prepare_cached(&sql)?
        .query_map([collection_id], ItemRow::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for r in &rows {
        if let Some(p) = &r.parent_id {
            *counts.entry(p.as_str()).or_default() += 1;
        }
    }
    let ordered = order_tree(&rows, collection.child_order);
    let items = ordered
        .into_iter()
        .map(|r| r.summary(counts.get(r.id.as_str()).copied().unwrap_or(0)))
        .collect::<Result<Vec<_>>>()?;
    Ok(CollectionTree { collection, items })
}

/// Sorts siblings in a collection's `child_order`, as the web client does (`buildTree.ts` `comparator`): `manual` by
/// `sort_key`; `title` A–Z by [`wikanvas_core::tree::title_key`] (case- and accent-insensitive, numbers by value);
/// `updated` and `created` newest first. Ties go to `sort_key`, then the id.
pub fn sort_siblings(rows: &mut [&ItemRow], order: ChildOrder) {
    let tie = |a: &ItemRow, b: &ItemRow| (&a.sort_key, &a.id).cmp(&(&b.sort_key, &b.id));
    match order {
        ChildOrder::Manual => rows.sort_by(|a, b| tie(a, b)),
        ChildOrder::Title => rows.sort_by_cached_key(|r| {
            (
                wikanvas_core::tree::title_key(&r.title),
                r.sort_key.clone(),
                r.id.clone(),
            )
        }),
        ChildOrder::Updated => {
            rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| tie(a, b)))
        }
        ChildOrder::Created => {
            rows.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| tie(a, b)))
        }
    }
}

/// Depth-first pre-order of `rows` with siblings sorted by `order`. Rows whose parent is not among `rows` (only a
/// corrupt tree has them) come last, so nothing is hidden.
fn order_tree(rows: &[ItemRow], order: ChildOrder) -> Vec<&ItemRow> {
    let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let mut groups: HashMap<Option<&str>, Vec<&ItemRow>> = HashMap::new();
    for r in rows {
        let parent = r.parent_id.as_deref().filter(|p| ids.contains(p));
        groups.entry(parent).or_default().push(r);
    }
    for g in groups.values_mut() {
        sort_siblings(g, order);
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&ItemRow> = groups
        .get(&None)
        .map(|g| g.iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(r) = stack.pop() {
        if !seen.insert(r.id.as_str()) {
            continue;
        }
        out.push(r);
        if let Some(children) = groups.get(&Some(r.id.as_str())) {
            stack.extend(children.iter().rev());
        }
    }
    if out.len() < rows.len() {
        let mut rest: Vec<&ItemRow> = rows
            .iter()
            .filter(|r| !seen.contains(r.id.as_str()))
            .collect();
        sort_siblings(&mut rest, order);
        out.extend(rest);
    }
    out
}

/// The visible children of an item in its collection's `child_order`: unarchived ones, or, for an archived item, its
/// archived children. `not_found` when the item does not exist.
pub fn children(conn: &Connection, viewer: &Viewer, item_id: &str) -> Result<Vec<ItemSummary>> {
    let parent = item_row(conn, item_id)?;
    let order = crate::collections::child_order(conn, &parent.collection_id)?;
    let sql = format!(
        "SELECT {}, {CHILD_COUNT} FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE i.parent_id = ?1 AND {} AND (i.archived_at IS NULL OR ?2)",
        ItemRow::COLUMNS,
        visible_items(viewer)
    );
    let rows = conn
        .prepare_cached(&sql)?
        .query_map(params![item_id, parent.archived()], |r| {
            Ok((
                ItemRow::from_row(r)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let counts: HashMap<&str, i64> = rows.iter().map(|(r, n)| (r.id.as_str(), *n)).collect();
    let mut sorted: Vec<&ItemRow> = rows.iter().map(|(r, _)| r).collect();
    sort_siblings(&mut sorted, order);
    sorted
        .into_iter()
        .map(|r| r.summary(counts[r.id.as_str()] as u64))
        .collect()
}

/// The sibling group `(id, sort_key)` in key order: every row (any state) under `parent_id` in the collection, except
/// `exclude`.
pub(crate) fn siblings(
    conn: &Connection,
    collection_id: &str,
    parent_id: Option<&str>,
    exclude: Option<&str>,
) -> Result<Vec<(String, String)>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, sort_key FROM items WHERE collection_id = ?1 AND parent_id IS ?2 AND id IS NOT ?3
             ORDER BY sort_key, id",
        )?
        .query_map(params![collection_id, parent_id, exclude], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The sort key for a row placed at `position` among its siblings (without `exclude`, the row being moved), with
/// the §5 rebalance when the key would pass 64 characters (which bumps the collection's `tree_version`).
pub(crate) fn place_item(
    tx: &Transaction<'_>,
    collection_id: &str,
    parent_id: Option<&str>,
    exclude: Option<&str>,
    position: Option<&Position>,
) -> Result<String> {
    place_rebalancing(
        || siblings(tx, collection_id, parent_id, exclude),
        |fresh| rewrite_keys(tx, collection_id, fresh),
        position,
    )
}

/// Rewrites every key of a sibling group with fresh short keys in the current order; returns how many rows changed.
pub fn rebalance(
    tx: &Transaction<'_>,
    collection_id: &str,
    parent_id: Option<&str>,
) -> Result<u64> {
    let group = siblings(tx, collection_id, parent_id, None)?;
    let fresh: Vec<(String, String)> = group
        .iter()
        .map(|(id, _)| id.clone())
        .zip(wikanvas_core::rank::rebalance(group.len()))
        .collect();
    rewrite_keys(tx, collection_id, &fresh)?;
    Ok(fresh.len() as u64)
}

fn rewrite_keys(
    tx: &Transaction<'_>,
    collection_id: &str,
    fresh: &[(String, String)],
) -> Result<()> {
    let mut stmt = tx.prepare_cached("UPDATE items SET sort_key = ?2 WHERE id = ?1")?;
    for (id, key) in fresh {
        stmt.execute(params![id, key])?;
    }
    crate::items::bump_tree_version(tx, collection_id)
}
