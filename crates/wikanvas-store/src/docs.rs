//! Document storage: Yjs state and updates, compaction, Markdown and ProseMirror JSON writes (owner: W4).
//!
//! - **Initializers** (called by W1's item create/duplicate/discard and W2's seed): [`init_empty`] stores
//!   `Y.encodeStateAsUpdate(new Doc)` (`[0, 0]`), [`init_with`] a given Yjs v1 state, [`init_seed`] a seed state with
//!   its placeholder item ids replaced, [`copy`] the state of another document; [`is_empty`] says whether a document
//!   holds only empty paragraphs. Each initializer replaces whatever state the item had (seq 0, no stored updates, no
//!   `pm_json`, no `last_token_id`) and derives the body text, `item_links` and `item_attachments`
//!   (`wikanvas_core::ytext`).
//! - **Yjs sync** (§6.1): [`get`] (`?since`) and [`save_update`], the save path every writer goes through: decode,
//!   apply on the stored state, 409 `missing_deps`, the no-op rule (state vector **and** delete set), the `pre-api`
//!   recovery point (D53), the append, the derived text, links, attachments and `doc_clients`, the `pm_json` cache,
//!   the auto revision, compaction (more than 200 rows or 1 MiB), and the 16 MiB cap.
//! - **Content** (§6.4): [`prosemirror`] and [`markdown`] read; [`write_content`] is the PUT: parse and diff against
//!   the `If-Match` base (one read snapshot), then re-check and save through [`save_update`] (one writer
//!   transaction). The caller holds two `doc_work` permits around it (D25).

use std::collections::{BTreeSet, HashSet};

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wikanvas_core::api::{
    ConversionWarning, DocJson, DocState, DocWriteResult, ErrorCode, InvalidDocumentDetails,
    ItemKind, LimitDetails, MissingDepsDetails, RevMismatchDetails, SCHEMA_VERSION, Timestamp,
    YFIELD,
};
use wikanvas_core::convert::{
    self, BaseError, ItemInfo, ItemRef, Lookup, Target, Warnings, md_in, md_out, pm, ydiff,
};
use wikanvas_core::schema::{self, Invalid};
use wikanvas_core::ytext;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update, Xml, XmlFragment, XmlOut};

use crate::db::Db;
use crate::error::{Error, Result};
use crate::revisions::{self, RevisionDraft};
use crate::shared::{Viewer, WriteBy, ensure_writable, item_row, set_body_text, touch_content};

/// `Y.encodeStateAsUpdate(new Y.Doc())`: no structs, an empty delete set.
pub const EMPTY_YDOC: [u8; 2] = [0, 0];
/// Compact when more than this many updates are stored (§6.1 step 6)…
pub const COMPACT_ROWS: i64 = 200;
/// …or when they hold more than this many bytes.
pub const COMPACT_BYTES: i64 = 1024 * 1024;
/// The default cap of a stored document (`WIKANVAS_MAX_DOC_MB`, D33).
pub const DEFAULT_MAX_DOC_BYTES: usize = 16 * 1024 * 1024;

/// Placeholder item ids in `fixtures/ydoc/getting-started.bin` ("Welcome to Wikanvas"): the "Writing in Wikanvas" page
/// it mentions and the example board it embeds. [`init_seed`] replaces them with the seeded items' ids.
pub const SEED_WRITING_ID: &str = "00000000-0000-7000-8000-000000000001";
pub const SEED_BOARD_ID: &str = "00000000-0000-7000-8000-000000000002";

/// The encoded state of a new, empty Y document (equal to [`EMPTY_YDOC`]).
pub fn empty_state() -> Vec<u8> {
    Doc::new()
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
}

/// The document ETag `"d{seq}"` (§7.1).
pub fn etag(seq: i64) -> String {
    format!("\"d{seq}\"")
}

/// The seq of a document ETag (`"d{n}"`, `W/` ignored), or `None` when it is not one.
pub fn parse_etag(tag: &str) -> Option<i64> {
    let t = tag.trim();
    let t = t.strip_prefix("W/").unwrap_or(t);
    t.strip_prefix("\"d")?
        .strip_suffix('"')?
        .parse()
        .ok()
        .filter(|n: &i64| *n >= 0)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Stores an empty document for `item_id` (a doc item), replacing any earlier state.
pub fn init_empty(tx: &Transaction<'_>, item_id: &str, now: i64) -> Result<()> {
    ensure_doc(tx, item_id)?;
    replace_state(tx, item_id, &empty_state(), 1, now)?;
    derive(tx, item_id, &ytext::Walk::default(), None)
}

/// Stores `ydoc_v1`, a full Yjs v1 state (the seed's `fixtures/ydoc/getting-started.bin`), for `item_id`, replacing
/// any earlier state, and derives the body text, links and attachments from it. An update that does not decode, or
/// that depends on structs it does not contain, is 422 `invalid_update`.
pub fn init_with(tx: &Transaction<'_>, item_id: &str, ydoc_v1: &[u8], now: i64) -> Result<()> {
    ensure_doc(tx, item_id)?;
    let doc = ydiff::load(&[ydoc_v1], None).map_err(|e| Error::new(ErrorCode::InvalidUpdate, e))?;
    let walk = ytext::walk(&doc);
    replace_state(tx, item_id, ydoc_v1, 1, now)?;
    derive(tx, item_id, &walk, None)
}

/// Like [`init_with`], with item ids replaced first: every `mention.itemId`, `boardEmbed.itemId` and `link.itemId`
/// equal to a `from` becomes its `to` (the seed passes [`SEED_WRITING_ID`] and [`SEED_BOARD_ID`] with the ids of
/// the items it created). The stored state is one full update.
pub fn init_seed(
    tx: &Transaction<'_>,
    item_id: &str,
    ydoc_v1: &[u8],
    ids: &[(&str, &str)],
    now: i64,
) -> Result<()> {
    ensure_doc(tx, item_id)?;
    let doc = ydiff::load(&[ydoc_v1], None).map_err(|e| Error::new(ErrorCode::InvalidUpdate, e))?;
    {
        let frag = pm::fragment(&doc);
        let mut txn = doc.transact_mut();
        let mut stack: Vec<XmlOut> = frag.children(&txn).collect();
        while let Some(node) = stack.pop() {
            let XmlOut::Element(e) = node else { continue };
            if let Some(yrs::Out::Any(yrs::Any::String(v))) = e.get_attribute(&txn, "itemId")
                && let Some((_, to)) = ids.iter().find(|(from, _)| *from == v.as_ref())
            {
                e.insert_attribute(&mut txn, "itemId", *to);
            }
            stack.extend(e.children(&txn).collect::<Vec<_>>());
        }
    }
    let state = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let walk = ytext::walk(&doc);
    replace_state(tx, item_id, &state, 1, now)?;
    derive(tx, item_id, &walk, None)
}

/// Copies the document of `from_id` to `to_id` (duplicate): the current state (compacted state plus stored updates,
/// as one update), the schema version, the indexed body text, `item_links` and `item_attachments`. Replaces any
/// earlier state of `to_id`.
pub fn copy(tx: &Transaction<'_>, from_id: &str, to_id: &str, now: i64) -> Result<()> {
    ensure_doc(tx, from_id)?;
    ensure_doc(tx, to_id)?;
    let stored = Stored::load(tx, from_id)?;
    let doc = stored.doc()?;
    let encoded = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    replace_state(tx, to_id, &encoded, stored.schema_version, now)?;
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
pub fn is_empty(conn: &Connection, item_id: &str) -> Result<bool> {
    let stored = Stored::load(conn, item_id)?;
    Ok(ytext::walk(&stored.doc()?).is_empty)
}

/// A document as stored: the compacted state and the updates after it.
struct Stored {
    ydoc: Vec<u8>,
    updates: Vec<(i64, Vec<u8>)>,
    schema_version: i64,
    compacted_seq: i64,
    next_seq: i64,
    updated_at: i64,
}

impl Stored {
    fn load(conn: &Connection, item_id: &str) -> Result<Stored> {
        let (ydoc, schema_version, compacted_seq, next_seq, updated_at) = conn
            .query_row(
                "SELECT ydoc, schema_version, compacted_seq, next_seq, updated_at FROM doc_state WHERE item_id = ?1",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .ok_or_else(|| Error::not_found("the item has no document"))?;
        let updates = conn
            .prepare("SELECT seq, data FROM doc_updates WHERE item_id = ?1 ORDER BY seq")?
            .query_map([item_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Stored {
            ydoc,
            updates,
            schema_version,
            compacted_seq,
            next_seq,
            updated_at,
        })
    }

    /// The seq of the newest stored update (0 for a new document): the ETag's `n`.
    fn seq(&self) -> i64 {
        self.next_seq - 1
    }

    /// The parts up to seq `n` (all when `None`).
    fn parts(&self, upto: Option<i64>) -> Vec<&[u8]> {
        let mut parts: Vec<&[u8]> = vec![&self.ydoc];
        parts.extend(
            self.updates
                .iter()
                .filter(|(s, _)| upto.is_none_or(|n| *s <= n))
                .map(|(_, d)| d.as_slice()),
        );
        parts
    }

    fn bytes(&self) -> usize {
        self.ydoc.len() + self.updates.iter().map(|(_, d)| d.len()).sum::<usize>()
    }

    fn doc(&self) -> Result<Doc> {
        ydiff::load(&self.parts(None), None)
            .map_err(|e| Error::new(ErrorCode::Internal, format!("stored document: {e}")))
    }
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

/// Sets the body text, `item_links` and `item_attachments` (ids that exist in `attachments` only) from a walk. With a
/// `writer` (a save by a user), an attachment the document did not show before is only recorded when the writer may
/// read it (D152): `item_attachments` is what makes an attachment readable (D36), so naming another collection's
/// image in a document of one's own must not open it to oneself or to this document's readers.
fn derive(
    tx: &Transaction<'_>,
    item_id: &str,
    walk: &ytext::Walk,
    writer: Option<&Viewer>,
) -> Result<()> {
    let before: HashSet<String> = match writer {
        Some(_) => attachment_rows(tx, item_id)?.into_iter().collect(),
        None => HashSet::new(),
    };
    set_body_text(tx, item_id, &walk.text)?;
    tx.execute("DELETE FROM item_links WHERE source_id = ?1", [item_id])?;
    for link in &walk.links {
        tx.execute(
            "INSERT OR IGNORE INTO item_links (source_id, target_id, kind) VALUES (?1, ?2, ?3)",
            params![item_id, link.target_id, link.kind.as_str()],
        )?;
    }
    tx.execute("DELETE FROM item_attachments WHERE item_id = ?1", [item_id])?;
    for id in &walk.attachment_ids {
        if let Some(writer) = writer
            && !before.contains(id.as_str())
            && !crate::access::attachment_readable(tx, writer, id)?
        {
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO item_attachments (item_id, attachment_id)
             SELECT ?1, id FROM attachments WHERE id = ?2",
            params![item_id, id],
        )?;
    }
    Ok(())
}

/// `GET /items/{id}/doc`: the whole state, or (with `since`, a Yjs state vector) only what it lacks.
pub fn get(conn: &Connection, item_id: &str, since: Option<&[u8]>) -> Result<DocState> {
    ensure_doc(conn, item_id)?;
    let stored = Stored::load(conn, item_id)?;
    let since = since
        .map(StateVector::decode_v1)
        .transpose()
        .map_err(|_| Error::validation("since", "since is not a Yjs state vector"))?;
    let doc = stored.doc()?;
    let txn = doc.transact();
    let update = txn.encode_state_as_update_v1(&since.unwrap_or_default());
    Ok(DocState {
        update: b64(&update),
        state_vector: b64(&txn.state_vector().encode_v1()),
        seq: stored.seq(),
        field: YFIELD.to_owned(),
        schema_version: stored.schema_version as u32,
    })
}

use yrs::updates::encoder::Encode;

/// What happens to the `pm_json` cache on a save.
#[derive(Clone, Copy, Debug)]
pub enum PmJson<'a> {
    /// Left as it is (a Yjs POST without `pm_json`).
    Keep,
    /// The editor's JSON after the update.
    Set(&'a Value),
    /// The same as JSON text, already checked to be a JSON object (the Yjs POST keeps it unparsed).
    Raw(&'a str),
    /// Cleared: it no longer matches (an API write).
    Clear,
}

/// The result of [`save_update`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The newest seq after the save (the current one for a no-op).
    pub seq: i64,
    /// The state vector after the save.
    pub state_vector: Vec<u8>,
    pub updated_at: i64,
    /// False for a no-op (nothing written).
    pub changed: bool,
}

/// The §6.1 save path (steps 1–7), inside the writer transaction; the caller holds the `doc_work` permit (a Yjs
/// POST) or two (a PUT, D25). 409 `read_only` for a trashed or archived item; 422 `invalid_update`; 409
/// `missing_deps` with the server's state vector; 413 `doc_too_large` over `max_doc_bytes`.
pub fn save_update(
    tx: &Transaction<'_>,
    item_id: &str,
    update: &[u8],
    pm_json: PmJson<'_>,
    by: &WriteBy<'_>,
    max_doc_bytes: usize,
    now: i64,
) -> Result<Saved> {
    ensure_writable(tx, item_id, ItemKind::Doc)?;
    // 1. Decode.
    let decoded = Update::decode_v1(update).map_err(|e| {
        Error::new(
            ErrorCode::InvalidUpdate,
            format!("the Yjs update does not decode: {e}"),
        )
    })?;
    let update_sv = decoded.state_vector();
    // 2. Load and apply in one transaction.
    let stored = Stored::load(tx, item_id)?;
    let doc = stored.doc()?;
    let (no_op, missing, server_sv) = {
        let mut txn = doc.transact_mut();
        let server_sv = txn.state_vector();
        txn.apply_update(decoded).map_err(|e| {
            Error::new(
                ErrorCode::InvalidUpdate,
                format!("the Yjs update does not apply: {e}"),
            )
        })?;
        let missing = txn.has_missing_updates();
        let no_op = txn.before_state() == txn.after_state() && txn.delete_set().is_empty();
        (no_op, missing, server_sv)
    };
    // 3. Missing dependencies.
    if missing {
        return Err(Error::new(
            ErrorCode::MissingDeps,
            "the update depends on changes the server does not have",
        )
        .with_details(&MissingDepsDetails {
            state_vector: b64(&server_sv.encode_v1()),
        }));
    }
    let state_vector = doc.transact().state_vector().encode_v1();
    // 4. No-op: nothing new, nothing deleted.
    if no_op {
        return Ok(Saved {
            seq: stored.seq(),
            state_vector,
            updated_at: stored.updated_at,
            changed: false,
        });
    }
    // 7 (checked first, so nothing is written over the cap).
    if stored.bytes() + update.len() > max_doc_bytes {
        let full = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        if full.len() > max_doc_bytes {
            return Err(too_large(max_doc_bytes));
        }
    }
    // D53: a recovery point before a token's write.
    revisions::pre_api(tx, item_id, ItemKind::Doc, by, now)?;
    // 5. Append and derive.
    let seq = stored.next_seq;
    tx.execute(
        "INSERT INTO doc_updates (item_id, seq, data, user_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![item_id, seq, update, by.user_id, now],
    )?;
    let walk = ytext::walk(&doc);
    let writer = writer_of(tx, by.user_id)?;
    derive(tx, item_id, &walk, Some(&writer))?;
    for (client, _) in update_sv.iter() {
        tx.execute(
            "INSERT OR IGNORE INTO doc_clients (item_id, client_id, user_id, first_seen_at) VALUES (?1, ?2, ?3, ?4)",
            params![item_id, client.get() as i64, by.user_id, now],
        )?;
    }
    touch_content(tx, item_id, by.user_id, now)?;
    tx.execute(
        "UPDATE doc_state SET next_seq = ?2, last_token_id = ?3, updated_at = ?4 WHERE item_id = ?1",
        params![item_id, seq + 1, by.token_id, now],
    )?;
    match pm_json {
        PmJson::Keep => {}
        PmJson::Set(_) | PmJson::Raw(_) => {
            tx.execute(
                "UPDATE doc_state SET pm_json = ?2, pm_json_seq = ?3 WHERE item_id = ?1",
                params![item_id, pm_text(pm_json)?, seq],
            )?;
        }
        PmJson::Clear => {
            tx.execute(
                "UPDATE doc_state SET pm_json = NULL, pm_json_seq = NULL WHERE item_id = ?1",
                [item_id],
            )?;
        }
    }
    // The auto revision (§5), with the attachment ids as GC roots.
    let newest: Option<i64> = tx.query_row(
        "SELECT max(created_at) FROM revisions WHERE item_id = ?1",
        [item_id],
        |r| r.get(0),
    )?;
    let full = || {
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default())
    };
    if newest.is_none_or(|t| now - t > revisions::AUTO_INTERVAL_MS) {
        let attachments = attachment_rows(tx, item_id)?;
        let mut draft = RevisionDraft::yjs(full(), seq, attachments, Some(by.user_id));
        if matches!(pm_json, PmJson::Set(_) | PmJson::Raw(_)) {
            draft.preview = pm_text(pm_json)?;
        }
        revisions::maybe_auto(tx, item_id, draft, now)?;
    }
    // 6. Compaction.
    let (rows, bytes): (i64, i64) = tx.query_row(
        "SELECT count(*), coalesce(sum(length(data)), 0) FROM doc_updates WHERE item_id = ?1",
        [item_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if rows > COMPACT_ROWS || bytes > COMPACT_BYTES {
        tx.execute(
            "UPDATE doc_state SET ydoc = ?2, compacted_seq = ?3 WHERE item_id = ?1",
            params![item_id, full(), seq],
        )?;
        tx.execute(
            "DELETE FROM doc_updates WHERE item_id = ?1 AND seq <= ?2",
            params![item_id, seq],
        )?;
    }
    Ok(Saved {
        seq,
        state_vector,
        updated_at: now,
        changed: true,
    })
}

/// The `pm_json` text to store, if any.
fn pm_text(pm_json: PmJson<'_>) -> Result<Option<String>> {
    Ok(match pm_json {
        PmJson::Set(v) => Some(serde_json::to_string(v)?),
        PmJson::Raw(s) => Some(s.to_owned()),
        PmJson::Keep | PmJson::Clear => None,
    })
}

fn too_large(limit: usize) -> Error {
    Error::new(
        ErrorCode::DocTooLarge,
        format!("the document would be larger than {limit} bytes"),
    )
    .with_details(&LimitDetails {
        limit_bytes: limit as u64,
    })
}

/// The writer of a save as a viewer (its role is read in the writer transaction). An account that is gone has no
/// role, so it reads nothing it did not upload.
fn writer_of(tx: &Transaction<'_>, user_id: &str) -> Result<Viewer> {
    let role: Option<String> = tx
        .query_row("SELECT role FROM users WHERE id = ?1", [user_id], |r| {
            r.get(0)
        })
        .optional()?;
    let role = match role {
        Some(r) => r
            .parse()
            .map_err(|e| Error::internal("stored user role", e))?,
        None => wikanvas_core::api::Role::Viewer,
    };
    Ok(Viewer {
        user_id: user_id.to_owned(),
        role,
    })
}

fn attachment_rows(conn: &Connection, item_id: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(
            "SELECT attachment_id FROM item_attachments WHERE item_id = ?1 ORDER BY attachment_id",
        )?
        .query_map([item_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The item and attachment lookups of the converters, on a connection (§6.4 "the parser takes a lookup closure from
/// the store), for one viewer (D152): an item in a collection the viewer may not read, and an attachment the viewer
/// may not read, are looked up as missing ones, so an export prints no title of theirs ("Deleted item") and a
/// Markdown write cannot resolve them.
pub struct DbLookup<'c> {
    pub conn: &'c Connection,
    pub viewer: &'c Viewer,
    /// `WIKANVAS_PUBLIC_URL`, for absolute same-instance URLs.
    pub public_origin: Option<String>,
}

impl Lookup for DbLookup<'_> {
    fn item(&self, r: &ItemRef) -> Option<ItemInfo> {
        let (column, value) = match r {
            ItemRef::Id(id) => ("id", id.as_str()),
            ItemRef::UrlId(u) => ("url_id", u.as_str()),
        };
        let readable = crate::shared::readable_collections(self.viewer);
        let mut args = vec![rusqlite::types::Value::Text(value.to_owned())];
        args.extend(readable.args);
        self.conn
            .query_row(
                &format!(
                    "SELECT i.id, i.kind, i.title FROM items i JOIN collections c ON c.id = i.collection_id
                     WHERE i.{column} = ? AND {}",
                    readable.sql
                ),
                rusqlite::params_from_iter(args.iter()),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|(id, kind, title)| {
                Some(ItemInfo {
                    id,
                    kind: kind.parse().ok()?,
                    title,
                })
            })
    }

    fn attachment(&self, id: &str) -> bool {
        crate::access::attachment_readable(self.conn, self.viewer, id).unwrap_or(false)
    }

    fn public_origin(&self) -> Option<&str> {
        self.public_origin.as_deref()
    }
}

/// `GET …/doc/prosemirror`: the normalized document (unknown nodes and marks pass through).
pub fn prosemirror(conn: &Connection, item_id: &str) -> Result<DocJson> {
    ensure_doc(conn, item_id)?;
    let stored = Stored::load(conn, item_id)?;
    let doc =
        pm::read(&stored.doc()?).map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?;
    Ok(DocJson {
        schema_version: stored.schema_version as u32,
        seq: stored.seq(),
        doc,
    })
}

/// A printed document.
#[derive(Clone, Debug)]
pub struct MarkdownDoc {
    pub markdown: String,
    pub warnings: Vec<ConversionWarning>,
    pub seq: i64,
    /// The item's title (for the download file name).
    pub title: String,
}

/// `GET …/doc/markdown`: Wikanvas Markdown 1, with `# <title>` first when `include_title`. Mentions and board embeds
/// of items `viewer` may not read print as those of missing items (D152).
pub fn markdown(
    conn: &Connection,
    viewer: &Viewer,
    item_id: &str,
    include_title: bool,
    public_origin: Option<String>,
) -> Result<MarkdownDoc> {
    let row = item_row(conn, item_id)?;
    if row.kind != ItemKind::Doc {
        return Err(Error::validation("kind", "the item is not a document"));
    }
    let stored = Stored::load(conn, item_id)?;
    let doc =
        pm::read(&stored.doc()?).map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?;
    let lookup = DbLookup {
        conn,
        viewer,
        public_origin,
    };
    let title = include_title.then_some(row.title.as_str());
    let printed = md_out::print(&doc, &lookup, title);
    Ok(MarkdownDoc {
        markdown: printed.markdown,
        warnings: printed.warnings.into_vec(),
        seq: stored.seq(),
        title: row.title,
    })
}

/// The input of a document PUT.
#[derive(Clone, Debug)]
pub enum ContentInput {
    /// `text/markdown` (at most 1 MiB, checked by the route).
    Markdown(String),
    /// The `doc` field of a ProseMirror PUT (at most 4 MiB).
    Json(Value),
}

/// `If-Match` of a content write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfMatch {
    /// `*`: whatever is current.
    Any,
    /// `"d{n}"`.
    Seq(i64),
}

/// A document PUT (§6.4 "Writing").
#[derive(Clone, Debug)]
pub struct ContentWrite {
    pub item_id: String,
    pub input: ContentInput,
    pub if_match: IfMatch,
    pub merge: bool,
    pub title_from_h1: bool,
    /// The credential: the token id, or the session id (for the request hash).
    pub credential: String,
    /// The normalized query string (for the request hash).
    pub query: String,
    /// sha256 of the body (for the request hash).
    pub body_sha256: [u8; 32],
    pub public_origin: Option<String>,
    pub max_doc_bytes: usize,
    /// The writer, whose access the Markdown parser's lookups have (D152).
    pub viewer: Viewer,
}

/// The author of a content write, owned (for the closures of [`Db`]).
#[derive(Clone, Debug)]
pub struct Author {
    pub user_id: String,
    pub token_id: Option<String>,
}

impl Author {
    pub fn write_by(&self) -> WriteBy<'_> {
        WriteBy {
            user_id: &self.user_id,
            token_id: self.token_id.as_deref(),
        }
    }
}

fn invalid(e: Invalid) -> Error {
    Error::new(
        ErrorCode::InvalidDocument,
        format!("the document is not schema v1: {e}"),
    )
    .with_details(&InvalidDocumentDetails {
        path: e.path,
        reason: e.reason,
    })
}

fn schema_too_new() -> Error {
    Error::new(
        ErrorCode::SchemaTooNew,
        "the document holds content this server's schema does not know; it cannot be overwritten",
    )
}

fn stale(current: i64, mergeable: bool) -> Error {
    Error::new(
        ErrorCode::RevMismatch,
        "the document changed since the If-Match version",
    )
    .with_details(&RevMismatchDetails {
        etag: etag(current),
        rev: None,
        mergeable: Some(mergeable),
    })
}

/// The outcome of the read phase.
enum Prepared {
    /// The target already equals the current state.
    Unchanged { seq: i64, updated_at: i64 },
    /// An update against the base at `base_seq`.
    Write { base_seq: i64, update: Vec<u8> },
}

/// The read phase of a PUT: parse, base, comparison space and diff, on one snapshot.
fn prepare(conn: &Connection, req: &ContentWrite) -> Result<(Prepared, Warnings, Option<String>)> {
    let row = item_row(conn, &req.item_id)?;
    if row.kind != ItemKind::Doc {
        return Err(Error::validation("kind", "the item is not a document"));
    }
    if row.deleted_at.is_some() || row.archived_at.is_some() {
        return Err(Error::new(
            ErrorCode::ReadOnly,
            "the item is in the trash or archived",
        ));
    }
    let lookup = DbLookup {
        conn,
        viewer: &req.viewer,
        public_origin: req.public_origin.clone(),
    };
    // 1. Parse.
    let mut warnings = Warnings::default();
    let mut target = match &req.input {
        ContentInput::Markdown(md) => {
            let (doc, w) = md_in::parse(md, &lookup).map_err(invalid)?;
            warnings.extend(w);
            Target::Markdown(doc)
        }
        ContentInput::Json(doc) => Target::Json(
            schema::validate(doc, "/doc", convert::link_allowed, convert::image_allowed)
                .map_err(invalid)?,
        ),
    };
    let title = if req.title_from_h1 {
        match &mut target {
            Target::Markdown(d) | Target::Json(d) => convert::take_h1(d),
        }
    } else {
        None
    };
    // 2. Base.
    let stored = Stored::load(conn, &req.item_id)?;
    if stored.schema_version > i64::from(SCHEMA_VERSION) {
        return Err(schema_too_new());
    }
    let current = stored.seq();
    let title_same = title.as_ref().is_none_or(|t| *t == row.title);
    let base_seq = match req.if_match {
        IfMatch::Any => current,
        IfMatch::Seq(n) if n == current => current,
        IfMatch::Seq(n) if req.merge && n >= stored.compacted_seq && n < current => n,
        IfMatch::Seq(n) => {
            // A stale strict write: fine only when the target already is the current state (a retried PUT).
            let doc = stored.doc()?;
            let raw = raw_json(&doc)?;
            if schema::has_unknown(&raw) {
                return Err(schema_too_new());
            }
            let now_in_space = match &target {
                Target::Json(_) => schema::normalize(&raw),
                Target::Markdown(_) => convert::comparison_tree(&raw, &lookup),
            };
            if ydiff::same(&now_in_space, target.doc()) && title_same {
                return Ok((
                    Prepared::Unchanged {
                        seq: current,
                        updated_at: stored.updated_at,
                    },
                    warnings,
                    title,
                ));
            }
            return Err(stale(current, n >= stored.compacted_seq && n <= current));
        }
    };
    // 3–4. Comparison space and diff, as one update.
    let base_etag = etag(base_seq);
    let mut h = Sha256::new();
    for part in [
        req.credential.as_bytes(),
        req.item_id.as_bytes(),
        base_etag.as_bytes(),
        req.query.as_bytes(),
    ] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    h.update(req.body_sha256);
    let h: [u8; 32] = h.finalize().into();
    let parts = stored.parts(Some(base_seq));
    let written = convert::write(&parts, &target, &lookup, &h).map_err(|e| match e {
        BaseError::SchemaTooNew => schema_too_new(),
        BaseError::Corrupt(m) => Error::new(ErrorCode::Internal, format!("stored document: {m}")),
    })?;
    warnings.extend(written.warnings);
    if !written.changed {
        return Ok((
            Prepared::Unchanged {
                seq: current,
                updated_at: stored.updated_at,
            },
            warnings,
            title,
        ));
    }
    Ok((
        Prepared::Write {
            base_seq,
            update: written.update,
        },
        warnings,
        title,
    ))
}

fn raw_json(doc: &Doc) -> Result<Value> {
    let frag = pm::fragment(doc);
    let txn = doc.transact();
    pm::raw(&txn, &frag).map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))
}

/// `PUT …/doc/markdown` and `PUT …/doc/prosemirror` (§6.4 steps 1–5): the diff against the `If-Match` base on a
/// read snapshot, then, in the writer: the base re-check (412 when it moved and neither `merge` nor `If-Match: *`
/// was given), the save through [`save_update`] (the `pre-api` revision included), `pm_json = NULL`, and
/// `?title=from_h1` with `items::rename`. The caller holds two `doc_work` permits.
pub async fn write_content(
    db: &Db,
    req: ContentWrite,
    by: &WriteBy<'_>,
    now: i64,
) -> Result<DocWriteResult> {
    let author = Author {
        user_id: by.user_id.to_owned(),
        token_id: by.token_id.map(str::to_owned),
    };
    let r = req.clone();
    let (prepared, warnings, title) = db.read(move |conn| prepare(conn, &r)).await?;
    let warnings = warnings.into_vec();
    db.write(move |tx| {
        let by = author.write_by();
        ensure_writable(tx, &req.item_id, ItemKind::Doc)?;
        let mut renamed = false;
        if let Some(title) = &title {
            let before: String = tx.query_row(
                "SELECT title FROM items WHERE id = ?1",
                [&req.item_id],
                |r| r.get(0),
            )?;
            if *title != before {
                crate::items::rename(tx, &req.item_id, title, by.user_id, now)?;
                renamed = true;
            }
        }
        match prepared {
            Prepared::Unchanged { seq, updated_at } => Ok(DocWriteResult {
                seq,
                etag: etag(seq),
                updated_at: Timestamp(if renamed { now } else { updated_at }),
                changed: renamed,
                merged: false,
                warnings,
            }),
            Prepared::Write { base_seq, update } => {
                let (current, compacted): (i64, i64) = tx.query_row(
                    "SELECT next_seq - 1, compacted_seq FROM doc_state WHERE item_id = ?1",
                    [&req.item_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                if current != base_seq && !req.merge && req.if_match != IfMatch::Any {
                    return Err(stale(current, base_seq >= compacted));
                }
                let saved = save_update(
                    tx,
                    &req.item_id,
                    &update,
                    PmJson::Clear,
                    &by,
                    req.max_doc_bytes,
                    now,
                )?;
                Ok(DocWriteResult {
                    seq: saved.seq,
                    etag: etag(saved.seq),
                    updated_at: Timestamp(saved.updated_at),
                    changed: saved.changed || renamed,
                    merged: saved.changed && current > base_seq,
                    warnings,
                })
            }
        }
    })
    .await
}

/// The ids of the items a document links to, by kind (for tests and backlinks later).
pub fn links(conn: &Connection, item_id: &str) -> Result<BTreeSet<(String, String)>> {
    Ok(conn
        .prepare("SELECT target_id, kind FROM item_links WHERE source_id = ?1")?
        .query_map([item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state_is_two_zero_bytes() {
        assert_eq!(empty_state(), EMPTY_YDOC);
    }

    #[test]
    fn etags() {
        assert_eq!(etag(0), "\"d0\"");
        assert_eq!(parse_etag("\"d12\""), Some(12));
        assert_eq!(parse_etag("W/\"d3\""), Some(3));
        assert_eq!(parse_etag("\"r3\""), None);
        assert_eq!(parse_etag("\"d-1\""), None);
        assert_eq!(parse_etag("*"), None);
    }
}
