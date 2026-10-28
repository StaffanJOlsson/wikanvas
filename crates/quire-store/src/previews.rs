//! Board SVG previews, per page and revision (owner: W6)
//!
//! `board_previews` holds one row per rendered page of a board: the deflated SVG of `quire_core::board::Previewer`
//! (NULL for an empty page), its uncompressed length, and the board revision it was made from; a row whose `rev` is
//! not `board_state.rev` is stale (§5, D10). A render always stores page 0, then the pages in order until the board's
//! preview budget is spent: a page after that has no row at that revision and counts as [`Preview::Omitted`]. The
//! preview worker and the on-demand render of `GET …/board/pages/{n}` (quire-server `board_previews`) deflate the pages
//! outside the writer ([`PageSvg::new`]) and store them with [`store`]; the route reads them with [`state`] and [`get`].

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::boards::{deflate, inflate};
use crate::error::Result;

/// A stored page preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// The page's SVG.
    Svg(String),
    /// The page has nothing to draw (the route answers 204).
    Empty,
    /// The revision was rendered, but the board's preview budget was spent before this page.
    Omitted,
}

/// What [`state`] finds for a page, without inflating its SVG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    /// An SVG of `len` bytes (uncompressed).
    Svg {
        len: usize,
    },
    Empty,
    Omitted,
}

/// One rendered page, deflated (outside the writer) and ready for [`store`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSvg {
    deflated: Vec<u8>,
    len: usize,
}

impl PageSvg {
    /// Deflates `svg` (the `board_previews.svg` encoding).
    pub fn new(svg: &str) -> Result<PageSvg> {
        Ok(PageSvg {
            deflated: deflate(svg.as_bytes())?,
            len: svg.len(),
        })
    }

    /// The SVG's uncompressed length.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Stores the previews of `item_id` made from revision `rev`: one row per page (`pages[i]` is page `i`, `None` for an
/// empty page), and the rows from `pages.len()` on are removed (pages that no longer exist, or that the budget left
/// out). Does nothing and returns false when the board is no longer at `rev` (a newer save made the render stale), or
/// has no board state.
pub fn store(
    tx: &Transaction<'_>,
    item_id: &str,
    rev: i64,
    pages: &[Option<PageSvg>],
) -> Result<bool> {
    let current: Option<i64> = tx
        .query_row(
            "SELECT rev FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        )
        .optional()?;
    if current != Some(rev) {
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM board_previews WHERE item_id = ?1 AND page_index >= ?2",
        params![item_id, pages.len() as i64],
    )?;
    let mut upsert = tx.prepare_cached(
        "INSERT INTO board_previews (item_id, page_index, rev, svg, svg_len) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (item_id, page_index) DO UPDATE SET rev = excluded.rev, svg = excluded.svg,
           svg_len = excluded.svg_len",
    )?;
    for (i, page) in pages.iter().enumerate() {
        let (data, len) = match page {
            Some(p) => (Some(p.deflated.as_slice()), p.len as i64),
            None => (None, 0),
        };
        upsert.execute(params![item_id, i as i64, rev, data, len])?;
    }
    Ok(true)
}

/// [`store`] for SVG text: deflates inside the writer, so it is meant for small previews (tests, tools).
pub fn replace(
    tx: &Transaction<'_>,
    item_id: &str,
    rev: i64,
    pages: &[Option<String>],
) -> Result<bool> {
    let pages = pages
        .iter()
        .map(|p| p.as_deref().map(PageSvg::new).transpose())
        .collect::<Result<Vec<_>>>()?;
    store(tx, item_id, rev, &pages)
}

/// Whether revision `rev` of `item_id` has been rendered (its page 0 has a row at that revision).
pub fn rendered(conn: &Connection, item_id: &str, rev: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM board_previews WHERE item_id = ?1 AND page_index = 0 AND rev = ?2",
            params![item_id, rev],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// What is stored for page `page` at revision `rev`, without reading the SVG: `None` when that revision has not
/// been rendered (never, or the rows are stale).
pub fn state(conn: &Connection, item_id: &str, page: usize, rev: i64) -> Result<Option<PageState>> {
    let row: Option<(bool, i64)> = conn
        .query_row(
            "SELECT svg IS NULL, svg_len FROM board_previews WHERE item_id = ?1 AND page_index = ?2 AND rev = ?3",
            params![item_id, page as i64, rev],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((true, _)) => Some(PageState::Empty),
        Some((false, len)) => Some(PageState::Svg {
            len: len.max(0) as usize,
        }),
        None if page > 0 && rendered(conn, item_id, rev)? => Some(PageState::Omitted),
        None => None,
    })
}

/// The stored preview of page `page` at revision `rev`, or `None` when that revision has not been rendered (never,
/// or stale).
pub fn get(conn: &Connection, item_id: &str, page: usize, rev: i64) -> Result<Option<Preview>> {
    let row: Option<Option<Vec<u8>>> = conn
        .query_row(
            "SELECT svg FROM board_previews WHERE item_id = ?1 AND page_index = ?2 AND rev = ?3",
            params![item_id, page as i64, rev],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match row {
        None if page > 0 && rendered(conn, item_id, rev)? => Some(Preview::Omitted),
        None => None,
        Some(None) => Some(Preview::Empty),
        Some(Some(svg)) => Some(Preview::Svg(inflate(&svg)?)),
    })
}

/// The revision the stored previews of `item_id` were made from (the newest, when pages differ), if any.
pub fn rev(conn: &Connection, item_id: &str) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT max(rev) FROM board_previews WHERE item_id = ?1",
        [item_id],
        |r| r.get(0),
    )?)
}

/// gzip of `bytes` (§11: page previews are sent gzip-encoded to clients that accept it).
pub fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(
        Vec::with_capacity(bytes.len() / 4 + 64),
        Compression::fast(),
    );
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn gzip_roundtrip() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".repeat(20);
        let gz = gzip(svg.as_bytes()).unwrap();
        assert_eq!(&gz[..2], &[0x1f, 0x8b]);
        let mut back = String::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_string(&mut back)
            .unwrap();
        assert_eq!(back, svg);
    }
}
