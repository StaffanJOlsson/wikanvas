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

    /// Reads a row selected with [`ItemRow::COLUMNS`] (from column 0).
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<ItemRow> {
        let kind: String = row.get(4)?;
        Ok(ItemRow {
            id: row.get(0)?,
            url_id: row.get(1)?,
            collection_id: row.get(2)?,
            parent_id: row.get(3)?,
            kind: kind.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            title: row.get(5)?,
            icon: row.get(6)?,
            sort_key: row.get(7)?,
            is_template: row.get(8)?,
            full_width: row.get(9)?,
            created_by: row.get(10)?,
            updated_by: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
            content_at: row.get(14)?,
            published_at: row.get(15)?,
            archived_at: row.get(16)?,
            archive_root_id: row.get(17)?,
            deleted_at: row.get(18)?,
            deleted_by: row.get(19)?,
            trash_root_id: row.get(20)?,
            meta_version: row.get(21)?,
        })
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
pub fn touch_content(tx: &Transaction<'_>, item_id: &str, actor_id: &str, now: i64) -> Result<()> {
    let n = tx.execute(
        "UPDATE items SET content_at = ?2, updated_at = ?2, updated_by = ?3 WHERE id = ?1",
        params![item_id, now, actor_id],
    )?;
    if n == 0 {
        return Err(Error::not_found("no such item"));
    }
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
    if row.deleted_at.is_some() || row.archived_at.is_some() {
        return Err(Error::new(
            ErrorCode::ReadOnly,
            "the item is in the trash or archived",
        ));
    }
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

/// A new UUIDv7 primary key (D4).
pub(crate) fn new_id() -> String {
    Uuid::now_v7().to_string()
}

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// `n` random base62 characters (uniform: bytes ≥ 248 are redrawn).
pub(crate) fn random_base62(n: usize) -> Result<String> {
    let mut out = String::with_capacity(n);
    let mut buf = [0u8; 64];
    while out.len() < n {
        getrandom::fill(&mut buf)
            .map_err(|e| Error::new(ErrorCode::Internal, format!("getrandom: {e}")))?;
        for &b in &buf {
            if out.len() == n {
                break;
            }
            if b < 248 {
                out.push(char::from(BASE62[usize::from(b % 62)]));
            }
        }
    }
    Ok(out)
}

/// A new `url_id`: 10 random base62 characters (D4).
pub(crate) fn new_url_id() -> Result<String> {
    random_base62(10)
}

/// The sort key for a new row among `siblings` (`(id, sort_key)` in key order), at `position` (both ids omitted:
/// last). An id that is not a sibling gives 422 `validation` (`field: "position"`).
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
    rank::key_between(lo, hi)
}

/// Fractional-index keys over base62 (D6). A port of the CC0 `fractional-indexing` algorithm (rocicorp,
/// `generateKeyBetween`); minimal W0b copy until W1's `quire_core::rank::key_between` replaces it.
pub(crate) mod rank {
    use crate::error::{Error, Result};
    use quire_core::api::ErrorCode;

    const DIGITS: &[u8; 62] = super::BASE62;
    const SMALLEST_INT: &str = "A00000000000000000000000000";

    fn bad(msg: String) -> Error {
        Error::new(ErrorCode::Internal, msg)
    }

    fn digit(c: u8) -> Result<usize> {
        DIGITS
            .iter()
            .position(|&d| d == c)
            .ok_or_else(|| bad(format!("invalid sort key digit {:?}", char::from(c))))
    }

    fn int_len(head: u8) -> Result<usize> {
        match head {
            b'a'..=b'z' => Ok(usize::from(head - b'a') + 2),
            b'A'..=b'Z' => Ok(usize::from(b'Z' - head) + 2),
            _ => Err(bad(format!("invalid sort key head {:?}", char::from(head)))),
        }
    }

    fn int_part(key: &str) -> Result<&str> {
        let len = int_len(
            *key.as_bytes()
                .first()
                .ok_or_else(|| bad("empty sort key".into()))?,
        )?;
        key.get(..len)
            .ok_or_else(|| bad(format!("invalid sort key {key:?}")))
    }

    fn validate(key: &str) -> Result<()> {
        if key == SMALLEST_INT {
            return Err(bad(format!("invalid sort key {key:?}")));
        }
        let i = int_part(key)?;
        let f = &key[i.len()..];
        if f.as_bytes().last() == Some(&DIGITS[0]) {
            return Err(bad(format!("invalid sort key {key:?}")));
        }
        for &c in key.as_bytes()[1..].iter() {
            digit(c)?;
        }
        Ok(())
    }

    /// A key strictly between the fractional parts `a` and `b` (`b == None`: unbounded above).
    fn midpoint(a: &[u8], b: Option<&[u8]>) -> Result<Vec<u8>> {
        if let Some(b) = b {
            let mut n = 0;
            while n < b.len() && a.get(n).copied().unwrap_or(DIGITS[0]) == b[n] {
                n += 1;
            }
            if n > 0 {
                let mut out = b[..n].to_vec();
                let rest_a = if n <= a.len() { &a[n..] } else { &[][..] };
                out.extend(midpoint(rest_a, Some(&b[n..]))?);
                return Ok(out);
            }
        }
        let da = match a.first() {
            Some(&c) => digit(c)?,
            None => 0,
        };
        let db = match b {
            Some(b) => digit(
                *b.first()
                    .ok_or_else(|| bad("sort keys out of order".into()))?,
            )?,
            None => DIGITS.len(),
        };
        if db - da > 1 {
            Ok(vec![DIGITS[(da + db).div_ceil(2)]])
        } else if let Some(b) = b.filter(|b| b.len() > 1) {
            Ok(vec![b[0]])
        } else {
            let mut out = vec![DIGITS[da]];
            let rest = if a.is_empty() { &[][..] } else { &a[1..] };
            out.extend(midpoint(rest, None)?);
            Ok(out)
        }
    }

    fn increment(x: &str) -> Result<Option<String>> {
        let b = x.as_bytes();
        let head = b[0];
        let mut digs = b[1..].to_vec();
        let mut carry = true;
        for d in digs.iter_mut().rev() {
            if !carry {
                break;
            }
            let v = digit(*d)? + 1;
            if v == DIGITS.len() {
                *d = DIGITS[0];
            } else {
                *d = DIGITS[v];
                carry = false;
            }
        }
        if carry {
            if head == b'Z' {
                return Ok(Some(format!("a{}", char::from(DIGITS[0]))));
            }
            if head == b'z' {
                return Ok(None);
            }
            let h = head + 1;
            if h > b'a' {
                digs.push(DIGITS[0]);
            } else {
                digs.pop();
            }
            let mut out = vec![h];
            out.extend(digs);
            return Ok(Some(String::from_utf8(out).expect("ASCII")));
        }
        let mut out = vec![head];
        out.extend(digs);
        Ok(Some(String::from_utf8(out).expect("ASCII")))
    }

    fn decrement(x: &str) -> Result<Option<String>> {
        let b = x.as_bytes();
        let head = b[0];
        let mut digs = b[1..].to_vec();
        let mut borrow = true;
        for d in digs.iter_mut().rev() {
            if !borrow {
                break;
            }
            let v = digit(*d)?;
            if v == 0 {
                *d = DIGITS[DIGITS.len() - 1];
            } else {
                *d = DIGITS[v - 1];
                borrow = false;
            }
        }
        if borrow {
            if head == b'a' {
                return Ok(Some(format!("Z{}", char::from(DIGITS[DIGITS.len() - 1]))));
            }
            if head == b'A' {
                return Ok(None);
            }
            let h = head - 1;
            if h < b'Z' {
                digs.push(DIGITS[DIGITS.len() - 1]);
            } else {
                digs.pop();
            }
            let mut out = vec![h];
            out.extend(digs);
            return Ok(Some(String::from_utf8(out).expect("ASCII")));
        }
        let mut out = vec![head];
        out.extend(digs);
        Ok(Some(String::from_utf8(out).expect("ASCII")))
    }

    fn with_mid(int: &str, frac: &[u8], hi: Option<&[u8]>) -> Result<String> {
        let mut out = int.as_bytes().to_vec();
        out.extend(midpoint(frac, hi)?);
        Ok(String::from_utf8(out).expect("ASCII"))
    }

    /// A key strictly between `a` and `b` (`None`: unbounded). `a < b` is required.
    pub(crate) fn key_between(a: Option<&str>, b: Option<&str>) -> Result<String> {
        if let Some(a) = a {
            validate(a)?;
        }
        if let Some(b) = b {
            validate(b)?;
        }
        match (a, b) {
            (Some(a), Some(b)) if a >= b => Err(bad(format!(
                "sort keys {a:?} >= {b:?}: the sibling group needs a rebalance"
            ))),
            (None, None) => Ok(format!("a{}", char::from(DIGITS[0]))),
            (None, Some(b)) => {
                let ib = int_part(b)?;
                let fb = &b.as_bytes()[ib.len()..];
                if ib == SMALLEST_INT {
                    return with_mid(ib, &[], Some(fb));
                }
                if ib < b {
                    return Ok(ib.to_owned());
                }
                decrement(ib)?.ok_or_else(|| bad("cannot decrement the sort key any more".into()))
            }
            (Some(a), None) => {
                let ia = int_part(a)?;
                let fa = &a.as_bytes()[ia.len()..];
                match increment(ia)? {
                    Some(i) => Ok(i),
                    None => with_mid(ia, fa, None),
                }
            }
            (Some(a), Some(b)) => {
                let ia = int_part(a)?;
                let fa = &a.as_bytes()[ia.len()..];
                let ib = int_part(b)?;
                let fb = &b.as_bytes()[ib.len()..];
                if ia == ib {
                    return with_mid(ia, fa, Some(fb));
                }
                let i = increment(ia)?
                    .ok_or_else(|| bad("cannot increment the sort key any more".into()))?;
                if i.as_str() < b {
                    return Ok(i);
                }
                with_mid(ia, fa, None)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::key_between;

        fn kb(a: Option<&str>, b: Option<&str>) -> String {
            key_between(a, b).unwrap()
        }

        #[test]
        fn matches_fractional_indexing_vectors() {
            // From the fractional-indexing README and test suite (CC0).
            assert_eq!(kb(None, None), "a0");
            assert_eq!(kb(None, Some("a0")), "Zz");
            assert_eq!(kb(None, Some("Zz")), "Zy");
            assert_eq!(kb(Some("a0"), None), "a1");
            assert_eq!(kb(Some("a1"), None), "a2");
            assert_eq!(kb(Some("a0"), Some("a1")), "a0V");
            assert_eq!(kb(Some("a1"), Some("a2")), "a1V");
            assert_eq!(kb(Some("a0V"), Some("a1")), "a0l");
            assert_eq!(kb(Some("Zz"), Some("a0")), "ZzV");
            assert_eq!(kb(Some("Zz"), Some("a1")), "a0");
            assert_eq!(kb(None, Some("Y00")), "Xzzz");
            assert_eq!(kb(Some("bzz"), None), "c000");
            assert_eq!(kb(Some("a0"), Some("a0V")), "a0G");
            assert_eq!(kb(Some("a0"), Some("a0G")), "a08");
            assert_eq!(kb(Some("b125"), Some("b129")), "b127");
            assert_eq!(kb(Some("a0"), Some("a1V")), "a1");
            assert_eq!(kb(Some("Zz"), Some("a01")), "a0");
            assert_eq!(kb(None, Some("a0V")), "a0");
            assert_eq!(kb(None, Some("b999")), "b99");
            assert_eq!(
                kb(None, Some("A000000000000000000000000001")),
                "A000000000000000000000000000V"
            );
            assert_eq!(
                kb(Some("zzzzzzzzzzzzzzzzzzzzzzzzzzy"), None),
                "zzzzzzzzzzzzzzzzzzzzzzzzzzz"
            );
            assert_eq!(
                kb(Some("zzzzzzzzzzzzzzzzzzzzzzzzzzz"), None),
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzV"
            );
            assert!(key_between(None, Some("A00000000000000000000000000")).is_err());
            assert!(key_between(Some("a00"), None).is_err());
            assert!(key_between(Some("a1"), Some("a0")).is_err());
        }

        #[test]
        fn appends_stay_ordered_and_short() {
            let mut keys = vec![kb(None, None)];
            for _ in 0..2000 {
                let next = kb(keys.last().map(String::as_str), None);
                assert!(next > *keys.last().unwrap());
                keys.push(next);
            }
            assert!(keys.iter().all(|k| k.len() <= 4), "{:?}", keys.last());
            let mut lo = keys[0].clone();
            let hi = keys[1].clone();
            for _ in 0..50 {
                let mid = kb(Some(&lo), Some(&hi));
                assert!(lo < mid && mid < hi);
                lo = mid;
            }
        }
    }
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
