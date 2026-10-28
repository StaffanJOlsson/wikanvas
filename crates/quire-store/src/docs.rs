//! Document storage: Yjs state and updates, compaction, Markdown and ProseMirror JSON writes (owner: W4).
//!
//! W0b provides the initializers W1 and W2 call (§6.1, PLAN W0b task 5): [`init_empty`] stores
//! `Y.encodeStateAsUpdate(new Doc)` (`[0, 0]`), [`init_with`] a given Yjs v1 state (the seed), [`copy`] the state of
//! another document (duplicate), and [`is_empty`] says whether the document holds only empty paragraphs. Each
//! initializer replaces whatever state the item had: `doc_state` starts over at seq 0 (`next_seq = 1`) with no
//! stored updates, no `pm_json` cache and no `last_token_id`. The text walk here is a minimal stand-in for W4's
//! `quire_core::ytext` (block text joined by `\n`; no links or attachment ids).

use quire_core::api::{ErrorCode, ItemKind, YFIELD};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, Out, ReadTxn, StateVector, Text, Transact, Update, XmlFragment, XmlOut};

use crate::error::{Error, Result};
use crate::shared::{item_row, set_body_text};

/// `Y.encodeStateAsUpdate(new Y.Doc())`: no structs, an empty delete set.
pub const EMPTY_YDOC: [u8; 2] = [0, 0];

/// The encoded state of a new, empty Y document (equal to [`EMPTY_YDOC`]).
pub fn empty_state() -> Vec<u8> {
    Doc::new()
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
}

/// Stores an empty document for `item_id` (a doc item), replacing any earlier state.
pub fn init_empty(tx: &Transaction<'_>, item_id: &str, now: i64) -> Result<()> {
    ensure_doc(tx, item_id)?;
    replace_state(tx, item_id, &empty_state(), 1, now)?;
    set_body_text(tx, item_id, "")
}

/// Stores `ydoc_v1`, a full Yjs v1 state (the seed's `fixtures/ydoc/getting-started.bin`), for `item_id`, replacing
/// any earlier state, and sets the indexed body text from it. An update that does not decode, or that depends on
/// structs it does not contain, is 422 `invalid_update`.
pub fn init_with(tx: &Transaction<'_>, item_id: &str, ydoc_v1: &[u8], now: i64) -> Result<()> {
    ensure_doc(tx, item_id)?;
    let doc = load(&[ydoc_v1]).map_err(|e| Error::new(ErrorCode::InvalidUpdate, e))?;
    let text = walk(&doc).text;
    replace_state(tx, item_id, ydoc_v1, 1, now)?;
    set_body_text(tx, item_id, &text)
}

/// Copies the document of `from_id` to `to_id` (duplicate): the current state (compacted state plus stored updates,
/// as one update), the schema version, the indexed body text, `item_links` and `item_attachments`. Replaces any
/// earlier state of `to_id`.
pub fn copy(tx: &Transaction<'_>, from_id: &str, to_id: &str, now: i64) -> Result<()> {
    ensure_doc(tx, from_id)?;
    ensure_doc(tx, to_id)?;
    let (state, schema_version) = current(tx, from_id)?;
    let doc = load(&[&state]).map_err(|e| Error::new(ErrorCode::Internal, e))?;
    let encoded = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    replace_state(tx, to_id, &encoded, schema_version, now)?;
    let body: String = tx.query_row(
        "SELECT body FROM item_text WHERE item_id = ?1",
        [from_id],
        |r| r.get(0),
    )?;
    set_body_text(tx, to_id, &body)?;
    tx.execute(
        "INSERT OR IGNORE INTO item_attachments (item_id, attachment_id)
         SELECT ?2, attachment_id FROM item_attachments WHERE item_id = ?1",
        params![from_id, to_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO item_links (source_id, target_id, kind)
         SELECT ?2, target_id, kind FROM item_links WHERE source_id = ?1",
        params![from_id, to_id],
    )?;
    Ok(())
}

/// §6.1: whether the document holds only empty paragraphs (y-tiptap writes one empty paragraph into an empty
/// fragment when the editor starts, so empty bytes are not the test). `not_found` when the item has no document.
pub fn is_empty(tx: &Transaction<'_>, item_id: &str) -> Result<bool> {
    let (state, _) = current(tx, item_id)?;
    let doc = load(&[&state]).map_err(|e| Error::new(ErrorCode::Internal, e))?;
    Ok(walk(&doc).only_empty_paragraphs)
}

/// The stored document of `item_id` as one full Yjs v1 update, with its schema version.
fn current(conn: &Connection, item_id: &str) -> Result<(Vec<u8>, i64)> {
    let (ydoc, schema_version): (Vec<u8>, i64) = conn
        .query_row(
            "SELECT ydoc, schema_version FROM doc_state WHERE item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::not_found("the item has no document"))?;
    let updates = conn
        .prepare("SELECT data FROM doc_updates WHERE item_id = ?1 ORDER BY seq")?
        .query_map([item_id], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if updates.is_empty() {
        return Ok((ydoc, schema_version));
    }
    let mut parts: Vec<&[u8]> = vec![&ydoc];
    parts.extend(updates.iter().map(Vec::as_slice));
    let doc = load(&parts).map_err(|e| Error::new(ErrorCode::Internal, e))?;
    let state = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    Ok((state, schema_version))
}

fn ensure_doc(conn: &Connection, item_id: &str) -> Result<()> {
    let row = item_row(conn, item_id)?;
    if row.kind != ItemKind::Doc {
        return Err(Error::validation("kind", "the item is not a document"));
    }
    Ok(())
}

fn replace_state(
    tx: &Transaction<'_>,
    item_id: &str,
    ydoc: &[u8],
    schema_version: i64,
    now: i64,
) -> Result<()> {
    for sql in [
        "DELETE FROM doc_updates WHERE item_id = ?1",
        "DELETE FROM doc_clients WHERE item_id = ?1",
        "DELETE FROM item_links WHERE source_id = ?1",
        "DELETE FROM item_attachments WHERE item_id = ?1",
    ] {
        tx.execute(sql, [item_id])?;
    }
    tx.execute(
        "INSERT INTO doc_state (item_id, ydoc, yfield, schema_version, compacted_seq, next_seq, pm_json,
           pm_json_seq, last_token_id, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, 1, NULL, NULL, NULL, ?5)
         ON CONFLICT (item_id) DO UPDATE SET ydoc = excluded.ydoc, yfield = excluded.yfield,
           schema_version = excluded.schema_version, compacted_seq = 0, next_seq = 1, pm_json = NULL,
           pm_json_seq = NULL, last_token_id = NULL, updated_at = excluded.updated_at",
        params![item_id, ydoc, YFIELD, schema_version, now],
    )?;
    Ok(())
}

/// Applies `updates` to a new document; an error names why they are not a complete, valid state.
fn load(updates: &[&[u8]]) -> std::result::Result<Doc, String> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        for bytes in updates {
            let update = Update::decode_v1(bytes)
                .map_err(|e| format!("the Yjs update does not decode: {e}"))?;
            txn.apply_update(update)
                .map_err(|e| format!("the Yjs update does not apply: {e}"))?;
        }
    }
    if doc.transact().has_missing_updates() {
        return Err("the Yjs update depends on structs it does not contain".into());
    }
    Ok(doc)
}

struct Walk {
    text: String,
    only_empty_paragraphs: bool,
}

/// Inline nodes of schema v1 (§6.1); every other element is a block, separated by `\n` in the text.
const INLINE: &[&str] = &["hardBreak", "mention", "image"];

fn walk(doc: &Doc) -> Walk {
    let frag = doc.get_or_insert_xml_fragment(YFIELD);
    let txn = doc.transact();
    let mut blocks: Vec<String> = Vec::new();
    let mut only_empty_paragraphs = true;
    for child in frag.children(&txn) {
        match &child {
            XmlOut::Element(e) => {
                if e.tag().as_ref() != "paragraph"
                    || e.children(&txn).any(|c| !is_empty_text(&c, &txn))
                {
                    only_empty_paragraphs = false;
                }
            }
            other => {
                if !is_empty_text(other, &txn) {
                    only_empty_paragraphs = false;
                }
            }
        }
        collect(&child, &txn, &mut blocks);
    }
    let text = blocks
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Walk {
        text,
        only_empty_paragraphs,
    }
}

fn is_empty_text<T: ReadTxn>(node: &XmlOut, txn: &T) -> bool {
    match node {
        XmlOut::Text(t) => Text::len(t, txn) == 0,
        _ => false,
    }
}

/// Appends the text of `node` to the last block, starting a new block for every block element.
fn collect<T: ReadTxn>(node: &XmlOut, txn: &T, blocks: &mut Vec<String>) {
    match node {
        XmlOut::Text(t) => {
            if blocks.is_empty() {
                blocks.push(String::new());
            }
            let last = blocks.last_mut().expect("pushed above");
            for d in t.diff(txn, YChange::identity) {
                if let Out::Any(Any::String(s)) = d.insert {
                    last.push_str(&s);
                }
            }
        }
        XmlOut::Element(e) => {
            let inline = INLINE.contains(&e.tag().as_ref());
            if !inline {
                blocks.push(String::new());
            }
            for c in e.children(txn) {
                collect(&c, txn, blocks);
            }
            if !inline {
                blocks.push(String::new());
            }
        }
        XmlOut::Fragment(f) => {
            for c in f.children(txn) {
                collect(&c, txn, blocks);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{XmlElementPrelim, XmlTextPrelim};

    fn doc_with(paragraphs: &[&str], extra_heading: bool) -> Vec<u8> {
        let doc = Doc::new();
        let frag = doc.get_or_insert_xml_fragment(YFIELD);
        {
            let mut txn = doc.transact_mut();
            for (i, p) in paragraphs.iter().enumerate() {
                let el = frag.insert(&mut txn, i as u32, XmlElementPrelim::empty("paragraph"));
                el.insert(&mut txn, 0, XmlTextPrelim::new(*p));
            }
            if extra_heading {
                let n = frag.len(&txn);
                let el = frag.insert(&mut txn, n, XmlElementPrelim::empty("heading"));
                el.insert(&mut txn, 0, XmlTextPrelim::new("Title"));
            }
        }
        let txn = doc.transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    }

    #[test]
    fn empty_state_is_two_zero_bytes() {
        assert_eq!(empty_state(), EMPTY_YDOC);
    }

    #[test]
    fn walk_text_and_emptiness() {
        let w = walk(&load(&[&EMPTY_YDOC]).unwrap());
        assert!(w.only_empty_paragraphs);
        assert_eq!(w.text, "");
        let w = walk(&load(&[&doc_with(&["", ""], false)]).unwrap());
        assert!(w.only_empty_paragraphs);
        let w = walk(&load(&[&doc_with(&["one", "two"], true)]).unwrap());
        assert!(!w.only_empty_paragraphs);
        assert_eq!(w.text, "one\ntwo\nTitle");
        let w = walk(&load(&[&doc_with(&[], true)]).unwrap());
        assert!(!w.only_empty_paragraphs);
    }

    #[test]
    fn load_rejects_garbage_and_missing_deps() {
        assert!(load(&[b"\xff\xff\xff"]).is_err());
        // The second half of a two-step history alone depends on the first.
        let doc = Doc::new();
        let frag = doc.get_or_insert_xml_fragment(YFIELD);
        let first = {
            let mut txn = doc.transact_mut();
            frag.insert(&mut txn, 0, XmlTextPrelim::new("a"));
            txn.encode_update_v1()
        };
        let sv = doc.transact().state_vector();
        {
            let mut txn = doc.transact_mut();
            let t = frag.get(&txn, 0).unwrap();
            if let XmlOut::Text(t) = t {
                t.insert(&mut txn, 1, "b");
            }
        }
        let second = doc.transact().encode_diff_v1(&sv);
        assert!(load(&[&first]).is_ok());
        assert!(load(&[&second]).is_err());
        assert!(load(&[&first, &second]).is_ok());
    }
}
