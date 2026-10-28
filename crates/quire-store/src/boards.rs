//! Board storage: the Inkboard envelope, its assets and revisions (owner: W6).
//!
//! W0b provides the initializers W1 and W2 call (§6.2 "Create", PLAN W0b task 5). A new board's envelope is
//! `serial::to_json(&Document::new(""), false)` with `background.preset` `white`, or `slate` when the owner's theme is
//! dark ([`empty_envelope`]); it is stored with `engine_version = "server"` and rev 1. `board_state.body` is the
//! raw DEFLATE (RFC 1951) of the stripped envelope, `body_sha256` the sha256 of the uncompressed stripped envelope,
//! and `meta` the §5 JSON. Each initializer replaces whatever state the item had (assets, previews, revision
//! counter). W0b's [`init_with`] validates with `serial::from_json` and refuses inline asset data (W6's
//! `quire_core::board::split` splits it into blobs); its text is the page names only, until W6's `inspect`.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use inkboard_core::model::{BgPreset, Document};
use inkboard_core::serial::{self, LoadError};
use quire_core::api::{ErrorCode, ItemKind};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::blobs::BlobStore;
use crate::error::{Error, Result};
use crate::shared::{item_row, set_body_text};

/// `board_state.engine_version` of boards the server wrote itself (new boards, seeds).
pub const SERVER_ENGINE: &str = "server";

/// Characters of the board text kept as `meta.text_excerpt`.
pub const EXCERPT_CHARS: usize = 200;

/// The envelope of a new, empty board: one page "Page 1", no objects, background `white` (`slate` when `dark`).
/// Its page id is random (inkboard ids), so two calls differ only there.
pub fn empty_envelope(dark: bool) -> String {
    let mut doc = Document::new("");
    doc.background.preset = if dark {
        BgPreset::Slate
    } else {
        BgPreset::White
    };
    serial::to_json(&doc, false)
}

/// `board_state.meta` (§5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub pages: Vec<MetaPage>,
    pub objects: u64,
    /// `#rrggbb`.
    pub background: String,
    /// Size of the full envelope in bytes.
    pub bytes: u64,
    pub text_excerpt: String,
}

/// One page of [`Meta`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaPage {
    pub id: String,
    pub name: String,
}

/// Stores an empty board ([`empty_envelope`]) for `item_id` (a board item), replacing any earlier state.
pub fn init_empty(tx: &Transaction<'_>, item_id: &str, dark: bool, now: i64) -> Result<()> {
    ensure_board(tx, item_id)?;
    let envelope = empty_envelope(dark);
    let doc = parse(&envelope)?;
    store(tx, item_id, &envelope, &doc, serial::VERSION, now)
}

/// Stores `envelope` (the seed's `fixtures/boards/getting-started.inkboard`) for `item_id`, replacing any earlier
/// state: validated with `serial::from_json` (422 `invalid_board`, or `board_too_new` for a newer version), stored
/// as received with `engine_version = "server"` and rev 1. Inline asset data gives 422 `invalid_board` until W6's
/// split lands (`blobs` is where it will store them).
pub fn init_with(
    tx: &Transaction<'_>,
    _blobs: &BlobStore,
    item_id: &str,
    envelope: &str,
    now: i64,
) -> Result<()> {
    ensure_board(tx, item_id)?;
    let doc = parse(envelope)?;
    if doc.assets.values().any(|a| !a.data.is_empty()) {
        return Err(Error::new(
            ErrorCode::InvalidBoard,
            "inline asset data is not supported by the W0b board store",
        ));
    }
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let version: Version = serde_json::from_str(serial::strip_bom(envelope))
        .map_err(|e| Error::new(ErrorCode::InvalidBoard, e.to_string()))?;
    store(tx, item_id, envelope, &doc, version.version, now)
}

/// Copies the board of `from_id` to `to_id` (duplicate): the stored body, its hash, meta, format and engine version,
/// the asset manifest (the blobs are shared, not copied) and the indexed body text; rev 1, no previews. Replaces any
/// earlier state of `to_id`.
pub fn copy(tx: &Transaction<'_>, from_id: &str, to_id: &str, now: i64) -> Result<()> {
    ensure_board(tx, from_id)?;
    ensure_board(tx, to_id)?;
    let exists = tx
        .query_row(
            "SELECT 1 FROM board_state WHERE item_id = ?1",
            [from_id],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_none() {
        return Err(Error::not_found("the item has no board"));
    }
    clear(tx, to_id)?;
    tx.execute(
        "INSERT INTO board_state (item_id, body, body_sha256, raw_len, format_version, engine_version, rev, seq,
           meta, last_token_id, updated_at)
         SELECT ?2, body, body_sha256, raw_len, format_version, engine_version, 1, 0, meta, NULL, ?3
         FROM board_state WHERE item_id = ?1",
        params![from_id, to_id, now],
    )?;
    tx.execute(
        "INSERT INTO board_assets (item_id, asset_id, sha256, mime)
         SELECT ?2, asset_id, sha256, mime FROM board_assets WHERE item_id = ?1",
        params![from_id, to_id],
    )?;
    let body: String = tx.query_row(
        "SELECT body FROM item_text WHERE item_id = ?1",
        [from_id],
        |r| r.get(0),
    )?;
    set_body_text(tx, to_id, &body)
}

/// Whether the board has no objects on any page (`meta.objects == 0`). `not_found` when the item has no board.
pub fn is_empty(tx: &Transaction<'_>, item_id: &str) -> Result<bool> {
    Ok(meta(tx, item_id)?.objects == 0)
}

/// The stored `meta` of a board.
pub fn meta(conn: &Connection, item_id: &str) -> Result<Meta> {
    let json: String = conn
        .query_row(
            "SELECT meta FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::not_found("the item has no board"))?;
    Ok(serde_json::from_str(&json)?)
}

/// The stored (stripped) envelope of a board, inflated.
pub fn stored_envelope(conn: &Connection, item_id: &str) -> Result<String> {
    let body: Vec<u8> = conn
        .query_row(
            "SELECT body FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::not_found("the item has no board"))?;
    inflate(&body)
}

/// Raw DEFLATE, the `board_state.body` and inkboard `revisions.data` encoding.
pub fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(
        Vec::with_capacity(bytes.len() / 4 + 64),
        Compression::default(),
    );
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

/// The inverse of [`deflate`], as UTF-8.
pub fn inflate(body: &[u8]) -> Result<String> {
    let mut out = String::new();
    DeflateDecoder::new(body).read_to_string(&mut out)?;
    Ok(out)
}

fn ensure_board(conn: &Connection, item_id: &str) -> Result<()> {
    let row = item_row(conn, item_id)?;
    if row.kind != ItemKind::Board {
        return Err(Error::validation("kind", "the item is not a board"));
    }
    Ok(())
}

fn parse(envelope: &str) -> Result<Document> {
    serial::from_json(envelope).map_err(|e| match e {
        LoadError::TooNew(v) => Error::new(
            ErrorCode::BoardTooNew,
            format!(
                "the board has format version {v}, newer than this server's {}",
                serial::VERSION
            ),
        ),
        other => Error::new(ErrorCode::InvalidBoard, other.to_string()),
    })
}

/// The text of a parsed board (W0b: page names, one per line).
fn text_of(doc: &Document) -> String {
    doc.pages
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn meta_of(doc: &Document, bytes: usize, text: &str) -> Meta {
    Meta {
        pages: doc
            .pages
            .iter()
            .map(|p| MetaPage {
                id: p.id.as_str().to_owned(),
                name: p.name.clone(),
            })
            .collect(),
        objects: doc.pages.iter().map(|p| p.objects.len() as u64).sum(),
        background: doc.background.preset.hex(),
        bytes: bytes as u64,
        text_excerpt: text.chars().take(EXCERPT_CHARS).collect(),
    }
}

fn clear(tx: &Transaction<'_>, item_id: &str) -> Result<()> {
    for sql in [
        "DELETE FROM board_assets WHERE item_id = ?1",
        "DELETE FROM board_previews WHERE item_id = ?1",
        "DELETE FROM board_state WHERE item_id = ?1",
    ] {
        tx.execute(sql, [item_id])?;
    }
    Ok(())
}

fn store(
    tx: &Transaction<'_>,
    item_id: &str,
    envelope: &str,
    doc: &Document,
    format_version: u32,
    now: i64,
) -> Result<()> {
    let text = text_of(doc);
    let meta = serde_json::to_string(&meta_of(doc, envelope.len(), &text))?;
    let body = deflate(envelope.as_bytes())?;
    let sha: [u8; 32] = Sha256::digest(envelope.as_bytes()).into();
    clear(tx, item_id)?;
    tx.execute(
        "INSERT INTO board_state (item_id, body, body_sha256, raw_len, format_version, engine_version, rev, seq,
           meta, last_token_id, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, ?7, NULL, ?8)",
        params![
            item_id,
            body,
            &sha[..],
            envelope.len() as i64,
            format_version,
            SERVER_ENGINE,
            meta,
            now
        ],
    )?;
    set_body_text(tx, item_id, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_envelope_shape() {
        let light: serde_json::Value = serde_json::from_str(&empty_envelope(false)).unwrap();
        assert_eq!(light["format"], "inkboard");
        assert_eq!(light["version"], serial::VERSION);
        assert_eq!(light["document"]["name"], "");
        assert_eq!(light["document"]["background"]["preset"], "white");
        assert_eq!(light["document"]["pages"].as_array().unwrap().len(), 1);
        let dark: serde_json::Value = serde_json::from_str(&empty_envelope(true)).unwrap();
        assert_eq!(dark["document"]["background"]["preset"], "slate");
    }

    #[test]
    fn deflate_roundtrip() {
        let s = empty_envelope(false);
        assert_eq!(inflate(&deflate(s.as_bytes()).unwrap()).unwrap(), s);
    }
}
