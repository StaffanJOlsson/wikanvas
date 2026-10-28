//! Helpers shared across workstreams: visible_items, WriteBy, the free-space check (owner: W1).
//!
//! Conventions of every store function: `tx: &rusqlite::Transaction` for writes (run inside [`crate::Db::write`]),
//! `conn: &rusqlite::Connection` for reads, ids as `&str` (UUIDv7 text), `now` in unix milliseconds (D5).

use std::path::Path;

use quire_core::api::{ErrorCode, ItemKind, ItemSummary, Role, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use crate::error::{Error, Result};

/// Who is looking: list-shaped queries filter through [`visible_items`] with it (§8 "Authorization seam").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub user_id: String,
    pub role: Role,
}

/// The author of a content write (D53): the user, and the token when the write is token-authenticated (`None` for a
/// session, the seed or an admin command). Sets `last_token_id` and drives [`crate::revisions::pre_api`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteBy<'a> {
    pub user_id: &'a str,
    pub token_id: Option<&'a str>,
}

/// One row of `items`, every column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRow {
    pub id: String,
    pub url_id: String,
    pub collection_id: String,
    pub parent_id: Option<String>,
    pub kind: ItemKind,
    pub title: String,
    pub icon: Option<String>,
    pub sort_key: String,
    pub is_template: bool,
    pub full_width: bool,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub content_at: Option<i64>,
    pub published_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub archive_root_id: Option<String>,
    pub deleted_at: Option<i64>,
    pub deleted_by: Option<String>,
    pub trash_root_id: Option<String>,
    pub meta_version: i64,
}

impl ItemRow {
    /// The column list [`ItemRow::from_row`] reads, for `SELECT {ItemRow::COLUMNS} FROM items i …`.
    pub const COLUMNS: &'static str = "i.id, i.url_id, i.collection_id, i.parent_id, i.kind, i.title, i.icon, \
        i.sort_key, i.is_template, i.full_width, i.created_by, i.updated_by, i.created_at, i.updated_at, \
        i.content_at, i.published_at, i.archived_at, i.archive_root_id, i.deleted_at, i.deleted_by, \
        i.trash_root_id, i.meta_version";

    /// The number of columns in [`ItemRow::COLUMNS`].
    pub const COLUMN_COUNT: usize = 22;

    /// Reads a row selected with [`ItemRow::COLUMNS`] (from column 0).
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<ItemRow> {
        ItemRow::from_row_at(row, 0)
    }

    /// Reads [`ItemRow::COLUMNS`] starting at column `at` (for queries that select something before them).
    pub fn from_row_at(row: &Row<'_>, at: usize) -> rusqlite::Result<ItemRow> {
        let kind: String = row.get(at + 4)?;
        Ok(ItemRow {
            id: row.get(at)?,
            url_id: row.get(at + 1)?,
            collection_id: row.get(at + 2)?,
            parent_id: row.get(at + 3)?,
            kind: kind.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    at + 4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            title: row.get(at + 5)?,
            icon: row.get(at + 6)?,
            sort_key: row.get(at + 7)?,
            is_template: row.get(at + 8)?,
            full_width: row.get(at + 9)?,
            created_by: row.get(at + 10)?,
            updated_by: row.get(at + 11)?,
            created_at: row.get(at + 12)?,
            updated_at: row.get(at + 13)?,
            content_at: row.get(at + 14)?,
            published_at: row.get(at + 15)?,
            archived_at: row.get(at + 16)?,
            archive_root_id: row.get(at + 17)?,
            deleted_at: row.get(at + 18)?,
            deleted_by: row.get(at + 19)?,
            trash_root_id: row.get(at + 20)?,
            meta_version: row.get(at + 21)?,
        })
    }

    /// Whether the row is in the trash.
    pub fn trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Whether the row is archived.
    pub fn archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// 409 `read_only` unless the item is live and unarchived (§7.1).
    pub fn ensure_live(&self) -> Result<()> {
        if self.trashed() || self.archived() {
            return Err(Error::new(
                ErrorCode::ReadOnly,
                "the item is in the trash or archived",
            ));
        }
        Ok(())
    }

    /// The API shape, with the number of visible children the caller counted.
    pub fn summary(&self, child_count: u64) -> Result<ItemSummary> {
        Ok(ItemSummary {
            id: parse_uuid(&self.id)?,
            url_id: self.url_id.clone(),
            collection_id: parse_uuid(&self.collection_id)?,
            parent_id: self.parent_id.as_deref().map(parse_uuid).transpose()?,
            kind: self.kind,
            title: self.title.clone(),
            sort_key: self.sort_key.clone(),
            child_count,
            published: self.published_at.is_some(),
            archived: self.archived_at.is_some(),
            updated_at: Timestamp(self.updated_at),
            content_at: self.content_at.map(Timestamp),
            created_at: Timestamp(self.created_at),
        })
    }
}

/// Sets `content_at`, `updated_at` and `updated_by` after a body save. `not_found` when the item does not exist.
/// When the item's collection sorts its tree by `updated`, the save reorders the tree, so its `tree_version` (the
/// `/tree` ETag) is bumped too; other orders leave it, and the tree's `updated_at` fields may lag until the next
/// structural change.
pub fn touch_content(tx: &Transaction<'_>, item_id: &str, actor_id: &str, now: i64) -> Result<()> {
    let n = tx.execute(
        "UPDATE items SET content_at = ?2, updated_at = ?2, updated_by = ?3 WHERE id = ?1",
        params![item_id, now, actor_id],
    )?;
    if n == 0 {
        return Err(Error::not_found("no such item"));
    }
    tx.execute(
        "UPDATE collections SET tree_version = tree_version + 1
         WHERE child_order = 'updated' AND id = (SELECT collection_id FROM items WHERE id = ?1)",
        [item_id],
    )?;
    Ok(())
}

/// Sets the indexed body text of an item (`item_text.body`); a no-op when it is unchanged (§5 "Body text").
pub fn set_body_text(tx: &Transaction<'_>, item_id: &str, body: &str) -> Result<()> {
    tx.execute(
        "UPDATE item_text SET body = ?2 WHERE item_id = ?1 AND body IS NOT ?2",
        params![item_id, body],
    )?;
    Ok(())
}

/// The item, if a content write of `kind` may change it: `not_found` when it does not exist, 422 `validation`
/// (`field: "kind"`) when it is of the other kind, 409 `read_only` when it is trashed or archived.
pub fn ensure_writable(tx: &Transaction<'_>, item_id: &str, kind: ItemKind) -> Result<ItemRow> {
    let row = item_row(tx, item_id)?;
    if row.kind != kind {
        return Err(Error::validation(
            "kind",
            format!("the item is a {}, not a {kind}", row.kind),
        ));
    }
    row.ensure_live()?;
    Ok(row)
}

/// The `items` row; `not_found` when there is none (whatever its state).
pub fn item_row(conn: &Connection, item_id: &str) -> Result<ItemRow> {
    conn.query_row(
        &format!("SELECT {} FROM items i WHERE i.id = ?1", ItemRow::COLUMNS),
        [item_id],
        ItemRow::from_row,
    )
    .optional()?
    .ok_or_else(|| Error::not_found("no such item"))
}

/// The SQL condition every list, tree, sidebar, search and backlink query selects items with (§5 "Visibility"). The
/// query must name `items` as `i` and the item's collection as `c`. M1: a live item in a live collection; the tree
/// and default search add `i.archived_at IS NULL` themselves. Multi-user later changes only this function (§15).
pub fn visible_items(_viewer: &Viewer) -> &'static str {
    "(c.deleted_at IS NULL AND i.deleted_at IS NULL)"
}

/// Whether the item and every ancestor have `published_at` (D23, §5 "Effective publication"). `not_found` when the
/// item does not exist.
pub fn effectively_published(conn: &Connection, item_id: &str) -> Result<bool> {
    let (found, drafts): (i64, i64) = conn.query_row(
        "WITH RECURSIVE anc(id, parent_id, published_at) AS (
           SELECT id, parent_id, published_at FROM items WHERE id = ?1
           UNION
           SELECT i.id, i.parent_id, i.published_at FROM items i JOIN anc ON i.id = anc.parent_id
         )
         SELECT count(*), count(*) FILTER (WHERE published_at IS NULL) FROM anc",
        [item_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if found == 0 {
        return Err(Error::not_found("no such item"));
    }
    Ok(drafts == 0)
}

/// D33: 507 `insufficient_storage` when the free space of the file system that holds `data_dir` is below the smaller
/// of `min_free_mb` MiB and 10% of the file system.
pub fn check_free_space(data_dir: &Path, min_free_mb: u64) -> Result<()> {
    let st = rustix::fs::statvfs(data_dir)
        .map_err(|e| Error::internal("statvfs", std::io::Error::from(e)))?;
    let unit = if st.f_frsize > 0 {
        st.f_frsize
    } else {
        st.f_bsize
    };
    let free = st.f_bavail.saturating_mul(unit);
    let total = st.f_blocks.saturating_mul(unit);
    let floor = min_free_mb.saturating_mul(1 << 20).min(total / 10);
    if free < floor {
        return Err(Error::new(
            ErrorCode::InsufficientStorage,
            format!(
                "only {} MiB free on the data volume (floor {} MiB)",
                free >> 20,
                floor >> 20
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- crate-internal helpers

/// Parses a stored UUID; a malformed one is `internal` (the store only writes valid ones).
pub(crate) fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| Error::internal(format!("stored id {s:?} is not a UUID"), e))
}

/// A new UUIDv7 primary key (D4), as stored.
pub(crate) fn new_id() -> String {
    quire_core::ids::new_uuid().to_string()
}

/// `n` random base62 characters ([`quire_core::ids::random_base62`]).
pub(crate) fn random_base62(n: usize) -> Result<String> {
    quire_core::ids::random_base62(n).map_err(|e| Error::internal("getrandom", e))
}

/// A new `url_id`: 10 random base62 characters (D4).
pub(crate) fn new_url_id() -> Result<String> {
    quire_core::ids::new_url_id().map_err(|e| Error::internal("getrandom", e))
}

/// A JSON array of ids, for `… IN (SELECT value FROM json_each(?))`.
pub(crate) fn json_ids(ids: &[String]) -> Result<String> {
    Ok(serde_json::to_string(ids)?)
}

/// The sort key for a new row among `siblings` (`(id, sort_key)` in key order), at `position` (both ids omitted:
/// last). An id that is not a sibling gives 422 `validation` (`field: "position"`). The caller rebalances the group
/// when the key is too long ([`quire_core::rank::needs_rebalance`]).
pub(crate) fn place(
    siblings: &[(String, String)],
    position: Option<&quire_core::api::Position>,
) -> Result<String> {
    let find = |id: &Uuid| -> Result<usize> {
        let id = id.to_string();
        siblings.iter().position(|(s, _)| *s == id).ok_or_else(|| {
            Error::validation("position", "the position names a row that is not a sibling")
        })
    };
    let after = position.and_then(|p| p.after_id.as_ref());
    let before = position.and_then(|p| p.before_id.as_ref());
    let (lo, hi) = match (after, before) {
        (None, None) => (siblings.last().map(|(_, k)| k.as_str()), None),
        (Some(a), None) => {
            let i = find(a)?;
            (
                Some(siblings[i].1.as_str()),
                siblings.get(i + 1).map(|(_, k)| k.as_str()),
            )
        }
        (None, Some(b)) => {
            let i = find(b)?;
            (
                i.checked_sub(1).map(|j| siblings[j].1.as_str()),
                Some(siblings[i].1.as_str()),
            )
        }
        (Some(a), Some(b)) => {
            let (i, j) = (find(a)?, find(b)?);
            if i >= j {
                return Err(Error::validation(
                    "position",
                    "after_id must come before before_id",
                ));
            }
            (Some(siblings[i].1.as_str()), Some(siblings[j].1.as_str()))
        }
    };
    Ok(quire_core::rank::key_between(lo, hi)?)
}

/// [`place`], and when the new key is longer than [`quire_core::rank::MAX_KEY_LEN`], a rebalance of the whole group
/// first (§5 "Sort keys"): `load` reads the group (`(id, sort_key)` in key order, without the row being placed) and
/// `rewrite` stores fresh keys for it. Returns the key for the new row.
pub(crate) fn place_rebalancing(
    load: impl Fn() -> Result<Vec<(String, String)>>,
    rewrite: impl FnOnce(&[(String, String)]) -> Result<()>,
    position: Option<&quire_core::api::Position>,
) -> Result<String> {
    let siblings = load()?;
    let key = place(&siblings, position)?;
    if !quire_core::rank::needs_rebalance(&key) {
        return Ok(key);
    }
    let fresh: Vec<(String, String)> = siblings
        .iter()
        .map(|(id, _)| id.clone())
        .zip(quire_core::rank::rebalance(siblings.len()))
        .collect();
    rewrite(&fresh)?;
    place(&load()?, position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_base62_has_the_alphabet_and_length() {
        let s = new_url_id().unwrap();
        assert_eq!(s.len(), 10);
        assert!(s.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(new_url_id().unwrap(), s);
    }

    #[test]
    fn place_between_neighbours() {
        let id = |n: u128| Uuid::from_u128(n);
        let sib = vec![
            (id(1).to_string(), "a0".to_owned()),
            (id(2).to_string(), "a1".to_owned()),
        ];
        assert_eq!(place(&sib, None).unwrap(), "a2");
        assert_eq!(place(&[], None).unwrap(), "a0");
        let after = |a: u128| quire_core::api::Position {
            after_id: Some(id(a)),
            before_id: None,
        };
        assert_eq!(place(&sib, Some(&after(1))).unwrap(), "a0V");
        assert_eq!(place(&sib, Some(&after(2))).unwrap(), "a2");
        let before = quire_core::api::Position {
            after_id: None,
            before_id: Some(id(1)),
        };
        assert_eq!(place(&sib, Some(&before)).unwrap(), "Zz");
        let e = place(&sib, Some(&after(9))).unwrap_err();
        assert!(e.is(ErrorCode::Validation));
    }
}
