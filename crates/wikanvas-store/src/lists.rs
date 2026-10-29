//! Lists: the cursor page helper every paginated query uses, the item lists, the sidebar and the trash (owner: W1).
//!
//! D46, §7.1 "Pagination": a paginated list takes `?cursor=&limit=` (default 25, at most 100) and returns
//! `Page<T> = {items, next_cursor}` with `next_cursor = null` on the last page. [`page`] pages any query by keyset
//! `(sort value, id)`: a page continues strictly after the last row of the previous one, so inserts and deletes
//! between two requests never repeat or skip a row that existed throughout. The cursor is bound to the list's name
//! and normalized parameters ([`PageRequest`]); reused with others it is 422 `validation` (`field: "cursor"`).
//!
//! The item lists (`/recent`, `/updated`, `/collections/{id}/updated`, `/drafts`, `/starred`, `/archive`) select
//! through `shared::visible_items`; all but `/archive` leave archived items out, and `/archive` lists archive roots.

use rusqlite::types::Value;
use rusqlite::{Connection, Row, params_from_iter};
use wikanvas_core::api::page::{DEFAULT_LIMIT, MAX_LIMIT};
use wikanvas_core::api::{
    ListedItem, Page, Sidebar, Timestamp, TrashList, TrashedCollection, TrashedItem,
};
use wikanvas_core::cursor::{self, Cursor, SortValue};
use wikanvas_core::time::purge_at;

use crate::error::{Error, Result};
use crate::shared::{ItemRow, Viewer, visible_items};
use crate::tree::{CHILD_COUNT, breadcrumbs};

/// One page request: the list's name and its normalized parameters (what the cursor is bound to), the cursor of
/// the previous page and the page size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRequest {
    pub list: String,
    /// `(name, value)` pairs that select the result set, defaults filled in; never `cursor` or `limit`.
    pub params: Vec<(String, String)>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl PageRequest {
    /// A request for `list` without other parameters.
    pub fn new(list: &str, cursor: Option<String>, limit: Option<u32>) -> PageRequest {
        PageRequest {
            list: list.to_owned(),
            params: Vec::new(),
            cursor,
            limit,
        }
    }

    /// The same request with one more parameter in the cursor binding.
    pub fn param(mut self, name: &str, value: impl Into<String>) -> PageRequest {
        self.params.push((name.to_owned(), value.into()));
        self
    }

    /// The page size: 1–100, 25 when omitted; anything else is 422 `validation` (`field: "limit"`).
    pub fn page_size(&self) -> Result<u32> {
        page_limit(self.limit, MAX_LIMIT, DEFAULT_LIMIT)
    }

    /// The 8-byte hash the cursor is bound to: the list's name and the parameters.
    pub fn query_hash(&self) -> [u8; 8] {
        let mut pairs: Vec<(&str, &str)> = vec![("list", self.list.as_str())];
        pairs.extend(self.params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cursor::query_hash(&pairs)
    }

    /// The decoded cursor (422 `validation`, `field: "cursor"`, when it is malformed or from another query).
    pub fn after(&self) -> Result<Option<Cursor>> {
        Ok(self
            .cursor
            .as_deref()
            .map(|c| cursor::decode(c, self.query_hash()))
            .transpose()?)
    }

    /// Encodes the cursor that continues after `c`.
    pub fn next(&self, c: &Cursor) -> String {
        cursor::encode(c, self.query_hash())
    }
}

/// A page size from `?limit=`: `default` when omitted, else 1–`max`; otherwise 422 `validation` (`field: "limit"`).
pub fn page_limit(limit: Option<u32>, max: u32, default: u32) -> Result<u32> {
    match limit {
        None => Ok(default),
        Some(n) if (1..=max).contains(&n) => Ok(n),
        Some(_) => Err(Error::validation("limit", format!("limit is 1 to {max}"))),
    }
}

/// Keyset direction of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Smallest sort value first (star order).
    Asc,
    /// Newest first (times).
    Desc,
}

/// One keyset page of `sql`. The query selects the sort value as its first column named `sort_value` (an INTEGER or
/// TEXT, never NULL) and the row id as its second, named `row_id`, takes `args` as its anonymous `?` parameters, and
/// has no ORDER BY or LIMIT; `map` reads the remaining columns (from index 2). Rows come in `(sort_value, row_id)`
/// order, `order` for both.
pub fn page<T>(
    conn: &Connection,
    req: &PageRequest,
    sql: &str,
    mut args: Vec<Value>,
    order: Order,
    mut map: impl FnMut(&Row<'_>) -> Result<T>,
) -> Result<Page<T>> {
    let limit = req.page_size()?;
    let (cmp, dir) = match order {
        Order::Asc => (">", "ASC"),
        Order::Desc => ("<", "DESC"),
    };
    let filter = match req.after()? {
        None => "1".to_owned(),
        Some(Cursor::Keyset { sort, id }) => {
            args.push(match sort {
                SortValue::Int(v) => Value::Integer(v),
                SortValue::Text(t) => Value::Text(t),
            });
            args.push(Value::Text(id));
            format!("(p.sort_value, p.row_id) {cmp} (?, ?)")
        }
        Some(Cursor::Offset(_)) => {
            return Err(Error::validation(
                "cursor",
                "the cursor belongs to a different query",
            ));
        }
    };
    args.push(Value::Integer(i64::from(limit) + 1));
    let full = format!(
        "SELECT * FROM ({sql}) AS p WHERE {filter} ORDER BY p.sort_value {dir}, p.row_id {dir} LIMIT ?"
    );
    let mut stmt = conn.prepare_cached(&full)?;
    let mut rows = stmt.query(params_from_iter(args.iter()))?;
    let mut out: Vec<(Value, String, T)> = Vec::new();
    while let Some(r) = rows.next()? {
        out.push((r.get(0)?, r.get(1)?, map(r)?));
    }
    let next = if out.len() > limit as usize {
        out.truncate(limit as usize);
        let (sort, id, _) = out.last().expect("a full page");
        let sort = match sort {
            Value::Integer(v) => SortValue::Int(*v),
            Value::Text(t) => SortValue::Text(t.clone()),
            other => {
                return Err(Error::internal(
                    "list sort value",
                    rusqlite::Error::InvalidColumnType(0, format!("{other:?}"), other.data_type()),
                ));
            }
        };
        Some(req.next(&Cursor::Keyset {
            sort,
            id: id.clone(),
        }))
    } else {
        None
    };
    Ok(Page::new(
        out.into_iter().map(|(_, _, t)| t).collect(),
        next,
    ))
}

/// The columns an item list selects after `sort_value` and `row_id`: [`ItemRow::COLUMNS`], the child count and the
/// list's extra column (`NULL` when it has none).
fn item_columns(extra: &str) -> String {
    format!("{}, {CHILD_COUNT}, {extra}", ItemRow::COLUMNS)
}

/// Reads a row selected with [`item_columns`]: the item and its path, with the extra time.
fn listed(conn: &Connection, r: &Row<'_>) -> Result<(ListedItem, Option<i64>)> {
    let row = ItemRow::from_row_at(r, 2)?;
    let children: i64 = r.get(2 + ItemRow::COLUMN_COUNT)?;
    let extra: Option<i64> = r.get(3 + ItemRow::COLUMN_COUNT)?;
    let path = breadcrumbs(conn, &row.id)?;
    Ok((
        ListedItem {
            item: row.summary(children as u64)?,
            path,
            starred_at: None,
            last_viewed_at: None,
            archived_at: None,
        },
        extra,
    ))
}

/// `GET /recent`: the viewer's viewed items, most recently viewed first, with `last_viewed_at`.
pub fn recent(conn: &Connection, viewer: &Viewer, req: &PageRequest) -> Result<Page<ListedItem>> {
    let sql = format!(
        "SELECT v.last_viewed_at AS sort_value, i.id AS row_id, {}
         FROM item_views v JOIN items i ON i.id = v.item_id JOIN collections c ON c.id = i.collection_id
         WHERE v.user_id = ? AND {} AND i.archived_at IS NULL",
        item_columns("v.last_viewed_at"),
        visible_items(viewer)
    );
    page(
        conn,
        req,
        &sql,
        vec![Value::Text(viewer.user_id.clone())],
        Order::Desc,
        |r| {
            let (mut item, extra) = listed(conn, r)?;
            item.last_viewed_at = extra.map(Timestamp);
            Ok(item)
        },
    )
}

/// `GET /updated` (every collection) and `GET /collections/{id}/updated` (`collection_id`): items at every depth,
/// most recently updated first.
pub fn updated(
    conn: &Connection,
    viewer: &Viewer,
    collection_id: Option<&str>,
    req: &PageRequest,
) -> Result<Page<ListedItem>> {
    let sql = format!(
        "SELECT i.updated_at AS sort_value, i.id AS row_id, {}
         FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE {} AND i.archived_at IS NULL AND (? IS NULL OR i.collection_id = ?)",
        item_columns("NULL"),
        visible_items(viewer)
    );
    let c = collection_id.map_or(Value::Null, |c| Value::Text(c.to_owned()));
    page(conn, req, &sql, vec![c.clone(), c], Order::Desc, |r| {
        Ok(listed(conn, r)?.0)
    })
}

/// `GET /drafts`: unpublished items (in M1 only those unpublished through the API or CLI, D23), most recently
/// updated first.
pub fn drafts(conn: &Connection, viewer: &Viewer, req: &PageRequest) -> Result<Page<ListedItem>> {
    let sql = format!(
        "SELECT i.updated_at AS sort_value, i.id AS row_id, {}
         FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE {} AND i.archived_at IS NULL AND i.published_at IS NULL",
        item_columns("NULL"),
        visible_items(viewer)
    );
    page(conn, req, &sql, Vec::new(), Order::Desc, |r| {
        Ok(listed(conn, r)?.0)
    })
}

/// The number of items `/drafts` lists (`Sidebar.drafts_count`).
pub fn drafts_count(conn: &Connection, viewer: &Viewer) -> Result<u64> {
    let sql = format!(
        "SELECT count(*) FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE {} AND i.archived_at IS NULL AND i.published_at IS NULL",
        visible_items(viewer)
    );
    Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// `GET /starred`: the viewer's starred items in star order, with `starred_at`.
pub fn starred(conn: &Connection, viewer: &Viewer, req: &PageRequest) -> Result<Page<ListedItem>> {
    let sql = format!(
        "SELECT s.sort_key AS sort_value, i.id AS row_id, {}
         FROM stars s JOIN items i ON i.id = s.item_id JOIN collections c ON c.id = i.collection_id
         WHERE s.user_id = ? AND {} AND i.archived_at IS NULL",
        item_columns("s.created_at"),
        visible_items(viewer)
    );
    page(
        conn,
        req,
        &sql,
        vec![Value::Text(viewer.user_id.clone())],
        Order::Asc,
        |r| {
            let (mut item, extra) = listed(conn, r)?;
            item.starred_at = extra.map(Timestamp);
            Ok(item)
        },
    )
}

/// `GET /archive`: archive roots that are not in the trash, most recently archived first, with `archived_at`.
pub fn archive(conn: &Connection, viewer: &Viewer, req: &PageRequest) -> Result<Page<ListedItem>> {
    let sql = format!(
        "SELECT i.archived_at AS sort_value, i.id AS row_id, {}
         FROM items i JOIN collections c ON c.id = i.collection_id
         WHERE {} AND i.archive_root_id = i.id",
        item_columns("i.archived_at"),
        visible_items(viewer)
    );
    page(conn, req, &sql, Vec::new(), Order::Desc, |r| {
        let (mut item, extra) = listed(conn, r)?;
        item.archived_at = extra.map(Timestamp);
        Ok(item)
    })
}

/// `GET /trash` (roots only, unpaginated): the deleted collections, and every item that is its own trash root (also
/// those in a deleted collection, trashed before it), most recently deleted first, each with its purge date; an item
/// root also with the number of descendants trashed with it and its path.
pub fn trash(conn: &Connection, _viewer: &Viewer) -> Result<TrashList> {
    let collections = crate::collections::deleted(conn)?
        .into_iter()
        .map(|(collection, deleted_at)| TrashedCollection {
            collection,
            deleted_at: Timestamp(deleted_at),
            purge_at: Timestamp(purge_at(deleted_at)),
        })
        .collect();
    let sql = format!(
        "SELECT {}, {CHILD_COUNT}, (SELECT count(*) FROM items d WHERE d.trash_root_id = i.id AND d.id <> i.id)
         FROM items i WHERE i.trash_root_id = i.id ORDER BY i.deleted_at DESC, i.id DESC",
        ItemRow::COLUMNS
    );
    let rows = conn
        .prepare_cached(&sql)?
        .query_map([], |r| {
            Ok((
                ItemRow::from_row(r)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT + 1)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let items = rows
        .into_iter()
        .map(|(row, children, descendants)| {
            let deleted_at = row.deleted_at.unwrap_or_default();
            Ok(TrashedItem {
                item: row.summary(children as u64)?,
                deleted_at: Timestamp(deleted_at),
                purge_at: Timestamp(purge_at(deleted_at)),
                descendant_count: descendants as u64,
                path: breadcrumbs(conn, &row.id)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TrashList { collections, items })
}

/// `GET /sidebar`: the live collections, the viewer's stars and the drafts count.
pub fn sidebar(conn: &Connection, viewer: &Viewer) -> Result<Sidebar> {
    Ok(Sidebar {
        collections: crate::collections::list(conn, viewer)?,
        stars: crate::stars::list(conn, viewer)?,
        drafts_count: drafts_count(conn, viewer)?,
    })
}
