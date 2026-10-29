//! Board storage: the Inkboard envelope, its assets and revisions (owner: W6).
//!
//! `board_state.body` is the raw DEFLATE (RFC 1951) of the *stripped* envelope (`wikanvas_core::board::split`: every
//! `document.assets[*].data` is `""`), `body_sha256` the sha256 of the uncompressed stripped envelope (the no-op test,
//! together with the asset manifest), `raw_len` its length, and `meta` the §5 JSON ([`Meta`]). The asset bytes live in
//! the blob store, one `board_assets` row per asset id (D8, D26); identical bytes are one blob, whatever board or asset
//! id holds them.
//!
//! - Initializers (W1's create and duplicate, W2's seed): [`init_empty`], [`init_with`], [`copy`]. Each replaces
//!   whatever state the item had and stores rev 1 with `engine_version = "server"`.
//! - The PUT pipeline (§6.2, D51): [`prepare`] outside the writer (inspect, split, the free-space check, staging the
//!   assets), then [`save`] inside it (the no-op test, If-Match, `revisions::pre_api` for a token, committing the
//!   assets, the new rev, `board_assets`, meta, text, the `embed` links, `content_at`, the revision policy).
//! - Cards (§18.2, §18.8): every write of a body replaces the board's `item_links` of kind `embed`, one
//!   `(board, target, 'embed')` row per target that `wikanvas_core::board::inspect` found (canonical UUIDs, distinct,
//!   at most `EMBED_LINKS_MAX`), never the board itself. Nothing checks that a target exists; a purged board's rows go
//!   with it (`source_id` cascades), a purged target's stay (`target_id` is no foreign key).
//! - Reads: [`stored`] (one reader query) then [`assemble`] (blob reads and the splice with the title), [`state`],
//!   [`envelope_len`], [`meta`] and [`board_meta`] (the API's `BoardMeta`, with the preview state of `previews`).

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use inkboard_core::model::{BgPreset, Document};
use inkboard_core::serial;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256 as Sha256Hasher};
use wikanvas_core::api::{BoardMeta, BoardPageMeta, ErrorCode, ItemKind, RevMismatchDetails};
use wikanvas_core::board::{self, BoardError, Inspect, Split};

use crate::blobs::{BlobStore, Sha256, Staged};
use crate::error::{Error, Result};
use crate::revisions::{self, RevisionDraft};
use crate::shared::{
    WriteBy, check_free_space, ensure_writable, item_row, set_body_text, touch_content,
};

pub use wikanvas_core::board::{Meta, MetaPage};

/// `board_state.engine_version` of boards the server wrote itself (new boards, seeds, duplicates).
pub const SERVER_ENGINE: &str = "server";

/// `board_state.engine_version` of a PUT without `X-Wikanvas-Engine` (D51).
pub const API_ENGINE: &str = "api";

/// Bytes of the board text kept as `meta.text_excerpt` (§9.5).
pub const EXCERPT_BYTES: usize = board::EXCERPT_BYTES;

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

/// The ETag of a board revision: `"r{rev}"` (D28, D45).
pub fn etag(rev: i64) -> String {
    format!("\"r{rev}\"")
}

/// The `If-Match` of a board PUT (D45). A `W/` prefix is ignored; tags that name no board revision never match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IfMatch {
    /// `*`: whatever is current (a deliberate last-write-wins).
    Any,
    /// The revisions the listed `"r{n}"` tags name (empty when the header named none).
    Revs(Vec<i64>),
}

impl IfMatch {
    /// Parses an `If-Match` value: `*`, or a comma-separated list of entity tags.
    pub fn parse(value: &str) -> IfMatch {
        if value.trim() == "*" {
            return IfMatch::Any;
        }
        IfMatch::Revs(
            value
                .split(',')
                .filter_map(|t| {
                    let t = t.trim();
                    let t = t.strip_prefix("W/").unwrap_or(t);
                    t.strip_prefix("\"r")?
                        .strip_suffix('"')?
                        .parse::<i64>()
                        .ok()
                })
                .collect(),
        )
    }

    /// Whether the header allows writing over revision `rev`.
    pub fn matches(&self, rev: i64) -> bool {
        match self {
            IfMatch::Any => true,
            IfMatch::Revs(revs) => revs.contains(&rev),
        }
    }
}

/// A board body checked and staged by [`prepare`], ready for [`save`]. Dropping it deletes the staged files.
#[derive(Debug)]
pub struct Prepared {
    stripped: String,
    sha256: [u8; 32],
    format_version: u32,
    meta: Meta,
    text: String,
    /// The targets of the board's cards (§18.8).
    embeds: Vec<String>,
    staged: Vec<StagedAsset>,
    /// Asset ids whose `data` was already `""`, with their mime.
    empty: Vec<(String, String)>,
}

impl Prepared {
    /// The derived meta (§5).
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// The derived search text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The stripped envelope that [`save`] stores.
    pub fn stripped(&self) -> &str {
        &self.stripped
    }

    /// The targets of the board's cards, which [`save`] records as `embed` links (§18.8).
    pub fn embeds(&self) -> &[String] {
        &self.embeds
    }

    /// How many assets were staged (one file each).
    pub fn staged_assets(&self) -> usize {
        self.staged.len()
    }
}

#[derive(Debug)]
struct StagedAsset {
    id: String,
    mime: String,
    staged: Staged,
}

/// The result of [`save`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The board's revision after the save (unchanged for a no-op).
    pub rev: i64,
    /// `board_state.updated_at` after the save.
    pub updated_at: i64,
    /// False when the stripped body equalled the stored one: nothing was written (§6.2 step 4).
    pub changed: bool,
    pub meta: BoardMeta,
}

/// A stored board as [`stored`] reads it: everything [`assemble`] needs, read in one snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBoard {
    pub item_id: String,
    pub rev: i64,
    /// `items.title`, which replaces `document.name` (D31).
    pub title: String,
    /// The inflated stripped envelope.
    pub stripped: String,
    /// `board_assets`: asset id → blob.
    pub assets: Vec<(String, Sha256)>,
    pub updated_at: i64,
}

/// The cheap facts about a board: its revision, page count, background and last write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardState {
    pub rev: i64,
    pub pages: usize,
    /// `meta.background` (`#rrggbb`).
    pub background: String,
    pub updated_at: i64,
}

/// Stores an empty board ([`empty_envelope`]) for `item_id` (a board item), replacing any earlier state.
pub fn init_empty(tx: &Transaction<'_>, item_id: &str, dark: bool, now: i64) -> Result<()> {
    ensure_board(tx, item_id)?;
    let envelope = empty_envelope(dark);
    let inspected = inspect(&envelope)?;
    let split = split(&envelope)?;
    store_initial(tx, None, item_id, inspected, split, now)
}

/// Stores `envelope` (the seed's `fixtures/boards/getting-started.inkboard`, or any full envelope) for `item_id`,
/// replacing any earlier state: validated with `serial::from_json` (422 `invalid_board`, or `board_too_new` for a
/// newer version), its inline assets split into `blobs` (written inside the writer: meant for small seeds), stored
/// with `engine_version = "server"` and rev 1, with the `embed` links of its cards (§18.8).
pub fn init_with(
    tx: &Transaction<'_>,
    blobs: &BlobStore,
    item_id: &str,
    envelope: &str,
    now: i64,
) -> Result<()> {
    ensure_board(tx, item_id)?;
    let inspected = inspect(envelope)?;
    let split = split(envelope)?;
    store_initial(tx, Some(blobs), item_id, inspected, split, now)
}

/// Copies the board of `from_id` to `to_id` (duplicate): the stored body, its hash, meta, format and engine version,
/// the asset manifest (the blobs are shared, not copied), the indexed body text and the `embed` links (the copy's
/// cards show the same documents, §18.9; a link to `to_id` itself is left out); rev 1, no previews. Replaces any
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
    tx.execute(
        "INSERT OR IGNORE INTO item_links (source_id, target_id, kind)
         SELECT ?2, target_id, kind FROM item_links WHERE source_id = ?1 AND kind = 'embed' AND target_id <> ?2",
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

/// The stored `meta` of a board (§5).
pub fn meta(conn: &Connection, item_id: &str) -> Result<Meta> {
    let json: String = conn
        .query_row(
            "SELECT meta FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(no_board)?;
    Ok(serde_json::from_str(&json)?)
}

/// The API's `BoardMeta` (`ItemDetail.board`, the PUT response): `board_state.meta` and `rev`, plus the preview
/// state. `preview_rev` is the newest revision any stored page preview was made from (`null` before the first);
/// a page's `svg_len` is its preview's size at that revision (`null` when that page has none yet).
pub fn board_meta(conn: &Connection, item_id: &str) -> Result<BoardMeta> {
    let (rev, json): (i64, String) = conn
        .query_row(
            "SELECT rev, meta FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(no_board)?;
    let meta: Meta = serde_json::from_str(&json)?;
    let rows = conn
        .prepare_cached(
            "SELECT page_index, rev, svg_len FROM board_previews WHERE item_id = ?1 ORDER BY page_index",
        )?
        .query_map([item_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let preview_rev = rows.iter().map(|r| r.1).max();
    let pages = meta
        .pages
        .into_iter()
        .enumerate()
        .map(|(i, p)| BoardPageMeta {
            id: p.id,
            name: p.name,
            svg_len: rows
                .iter()
                .find(|r| r.0 == i as i64 && Some(r.1) == preview_rev)
                .map(|r| r.2.max(0) as u64),
        })
        .collect();
    Ok(BoardMeta {
        rev,
        pages,
        objects: meta.objects,
        background: meta.background,
        bytes: meta.bytes,
        preview_rev,
        text_excerpt: meta.text_excerpt,
    })
}

/// The revision, page count, background and `updated_at` of a board; `not_found` when the item has no board.
pub fn state(conn: &Connection, item_id: &str) -> Result<BoardState> {
    let (rev, json, updated_at): (i64, String, i64) = conn
        .query_row(
            "SELECT rev, meta, updated_at FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(no_board)?;
    let meta: Meta = serde_json::from_str(&json)?;
    Ok(BoardState {
        rev,
        pages: meta.pages.len(),
        background: meta.background,
        updated_at,
    })
}

/// An upper estimate of the length of the full envelope a GET sends: the stored (stripped) body, the base64 of every
/// asset's blob, and the title. Read before the envelope is assembled, to reserve memory for the response.
/// `not_found` when the item has no board.
pub fn envelope_len(conn: &Connection, item_id: &str) -> Result<u64> {
    let (raw_len, title_len, assets): (i64, i64, i64) = conn
        .query_row(
            "SELECT s.raw_len, length(CAST(i.title AS BLOB)),
               (SELECT coalesce(sum((b.size + 2) / 3 * 4), 0)
                FROM board_assets a JOIN blobs b ON b.sha256 = a.sha256 WHERE a.item_id = s.item_id)
             FROM board_state s JOIN items i ON i.id = s.item_id WHERE s.item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(no_board)?;
    Ok([raw_len, title_len, assets]
        .iter()
        .map(|n| u64::try_from(*n).unwrap_or(0))
        .sum())
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
        .ok_or_else(no_board)?;
    inflate(&body)
}

/// Everything a GET needs, in one snapshot: the stripped envelope, the rev, the title and the asset manifest.
/// `not_found` when the item does not exist or is not a board.
pub fn stored(conn: &Connection, item_id: &str) -> Result<StoredBoard> {
    let (rev, body, updated_at, title): (i64, Vec<u8>, i64, String) = conn
        .query_row(
            "SELECT s.rev, s.body, s.updated_at, i.title FROM board_state s JOIN items i ON i.id = s.item_id
             WHERE s.item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(no_board)?;
    let assets = conn
        .prepare_cached(
            "SELECT asset_id, sha256 FROM board_assets WHERE item_id = ?1 ORDER BY asset_id",
        )?
        .query_map([item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, sha)| {
            Sha256::parse(&sha)
                .map(|s| (id, s))
                .map_err(|e| Error::internal("board_assets.sha256", e))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StoredBoard {
        item_id: item_id.to_owned(),
        rev,
        title,
        stripped: inflate(&body)?,
        assets,
        updated_at,
    })
}

/// The full envelope of a stored board: the blob bytes spliced back and `document.name` set to the title (§6.2
/// "Load"). An asset whose blob file is missing stays `""` (logged). Reads files: call it outside the reader.
pub fn assemble(blobs: &BlobStore, board: &StoredBoard) -> Result<String> {
    let mut bytes: HashMap<String, Vec<u8>> = HashMap::with_capacity(board.assets.len());
    for (id, sha) in &board.assets {
        match blobs.read(sha) {
            Ok(data) => {
                bytes.insert(id.clone(), data);
            }
            Err(e) => {
                tracing::warn!(item_id = %board.item_id, asset_id = %id, sha256 = %sha, error = %e,
                    "a board asset's blob is missing; it is sent without data");
            }
        }
    }
    board::splice(&board.stripped, &bytes, &board.title)
        .map_err(|e| Error::internal("the stored board does not splice", e))
}

/// Step 3 of the PUT pipeline (§6.2), outside the writer: [`board::inspect`] the full body (422 `invalid_board` /
/// `board_too_new`), [`board::split`] it, check the free space (507 `insufficient_storage`, D33), then stage each
/// asset in the blob store (one file per asset; identical bytes are deduplicated by [`BlobStore::commit`]).
pub fn prepare(blobs: &BlobStore, body: &str, min_free_mb: u64) -> Result<Prepared> {
    prepare_with(blobs, body, || check_free_space(blobs.root(), min_free_mb))
}

/// [`prepare`] with the free-space check given by the caller (the server's injectable probe, D33).
pub fn prepare_with(
    blobs: &BlobStore,
    body: &str,
    free_space: impl FnOnce() -> Result<()>,
) -> Result<Prepared> {
    let inspected = inspect(body)?;
    let split = split(body)?;
    free_space()?;
    let mut staged = Vec::new();
    let mut empty = Vec::new();
    for a in split.assets {
        if a.bytes.is_empty() {
            empty.push((a.id, a.mime));
            continue;
        }
        let file = blobs.stage(&a.bytes)?;
        staged.push(StagedAsset {
            id: a.id,
            mime: a.mime,
            staged: file,
        });
    }
    let sha256: [u8; 32] = Sha256Hasher::digest(split.stripped.as_bytes()).into();
    Ok(Prepared {
        stripped: split.stripped,
        sha256,
        format_version: split.format_version,
        meta: inspected.meta,
        text: inspected.text,
        embeds: inspected.embeds,
        staged,
        empty,
    })
}

/// Steps 4 and 5 of the PUT pipeline (§6.2), inside the writer transaction:
/// - `not_found`; 422 `validation` (`field: "kind"`) for a document; 409 `read_only` when trashed or archived;
/// - when sha256(stripped) equals `body_sha256` and every asset sent with data has the bytes the board already holds
///   under its id (in a blob file that exists), nothing is written (the staged files are dropped) and the current rev
///   is returned with `changed: false`;
/// - otherwise `if_match` must allow the current rev (412 `rev_mismatch` with `details {etag, rev}`); a token's write
///   first runs [`revisions::pre_api`] (D53); the staged assets are committed, the body stored with `rev + 1`,
///   `engine_version` and `last_token_id` (NULL for a session); `board_assets`, `meta`, `item_text.body`, the `embed`
///   links (§18.8), `content_at`, `updated_at` and `updated_by` are replaced; and the revision policy runs.
///
/// A no-op save leaves the links as they are: they derive from the body, which did not change.
///
/// An asset sent with `data: ""` keeps the blob the board already had under that id, if any (an API client may
/// resend a body without re-sending its images).
#[allow(clippy::too_many_arguments)]
pub fn save(
    tx: &Transaction<'_>,
    blobs: &BlobStore,
    item_id: &str,
    prepared: Prepared,
    if_match: &IfMatch,
    by: &WriteBy<'_>,
    engine: &str,
    now: i64,
) -> Result<Saved> {
    ensure_writable(tx, item_id, ItemKind::Board)?;
    let (rev, sha, updated_at): (i64, Vec<u8>, i64) = tx
        .query_row(
            "SELECT rev, body_sha256, updated_at FROM board_state WHERE item_id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(no_board)?;
    let previous: HashMap<String, String> = tx
        .prepare_cached("SELECT asset_id, sha256 FROM board_assets WHERE item_id = ?1")?
        .query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // The stripped body has no asset bytes: an asset whose bytes changed under the same id, or that is sent again
    // after it was stored without data (or with a lost blob file), is a change too.
    let same_assets = || {
        prepared.staged.iter().all(|a| {
            previous.get(&a.id).map(String::as_str) == Some(a.staged.sha256().as_str())
                && blobs.path(a.staged.sha256()).exists()
        })
    };
    if sha == prepared.sha256 && same_assets() {
        drop(prepared);
        return Ok(Saved {
            rev,
            updated_at,
            changed: false,
            meta: board_meta(tx, item_id)?,
        });
    }
    if !if_match.matches(rev) {
        return Err(rev_mismatch(rev));
    }
    revisions::pre_api(tx, item_id, ItemKind::Board, by, now)?;

    let Prepared {
        stripped,
        sha256,
        format_version,
        meta,
        text,
        embeds,
        staged,
        empty,
    } = prepared;
    // asset id → (sha256, mime)
    let mut rows: BTreeMap<String, (String, String)> = BTreeMap::new();
    for asset in staged {
        let blob = blobs.commit(tx, asset.staged, now)?;
        rows.insert(asset.id, (blob.sha256.as_str().to_owned(), asset.mime));
    }
    for (id, mime) in empty {
        if let Some(sha) = previous.get(&id) {
            rows.entry(id).or_insert_with(|| (sha.clone(), mime));
        }
    }

    let body = deflate(stripped.as_bytes())?;
    let new_rev = rev + 1;
    tx.execute(
        "UPDATE board_state SET body = ?2, body_sha256 = ?3, raw_len = ?4, format_version = ?5, engine_version = ?6,
           rev = ?7, meta = ?8, last_token_id = ?9, updated_at = ?10
         WHERE item_id = ?1",
        params![
            item_id,
            body,
            &sha256[..],
            stripped.len() as i64,
            format_version,
            engine,
            new_rev,
            serde_json::to_string(&meta)?,
            by.token_id,
            now
        ],
    )?;
    tx.execute("DELETE FROM board_assets WHERE item_id = ?1", [item_id])?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO board_assets (item_id, asset_id, sha256, mime) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (id, (sha, mime)) in &rows {
            insert.execute(params![item_id, id, sha, mime])?;
        }
    }
    set_body_text(tx, item_id, &text)?;
    set_embed_links(tx, item_id, &embeds)?;
    touch_content(tx, item_id, by.user_id, now)?;
    let assets: BTreeMap<String, String> =
        rows.into_iter().map(|(id, (sha, _))| (id, sha)).collect();
    revisions::maybe_auto(
        tx,
        item_id,
        RevisionDraft::inkboard(
            body,
            i64::from(format_version),
            new_rev,
            assets,
            Some(by.user_id),
        ),
        now,
    )?;
    Ok(Saved {
        rev: new_rev,
        updated_at: now,
        changed: true,
        meta: board_meta(tx, item_id)?,
    })
}

/// Migration 0003's step (D152, §12): takes the cards' labels out of the search text and `meta.text_excerpt` of the
/// boards an earlier build derived them for. A card's label is the title snapshot of a document that someone who may
/// read the board may not be allowed to read, and the text is searched and shown (`GET …/text`, search snippets,
/// the excerpt) without a per-reader check. Only boards that hold a card are read again; one board in memory at a
/// time (D25). Returns how many boards changed.
pub fn take_cards_out_of_text(tx: &Transaction<'_>) -> Result<u64> {
    let ids: Vec<String> = tx
        .prepare("SELECT item_id FROM board_state ORDER BY item_id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = 0;
    for id in ids {
        let body: Vec<u8> = tx.query_row(
            "SELECT body FROM board_state WHERE item_id = ?1",
            [&id],
            |r| r.get(0),
        )?;
        let Some(text) = board::text_without_cards(&inflate(&body)?) else {
            continue;
        };
        drop(body);
        set_body_text(tx, &id, &text)?;
        tx.execute(
            "UPDATE board_state SET meta = json_set(meta, '$.text_excerpt', ?2) WHERE item_id = ?1",
            params![id, board::excerpt(&text)],
        )?;
        changed += 1;
    }
    Ok(changed)
}

/// 412 `rev_mismatch` with `details {etag, rev}` of the current revision (§7.1).
pub fn rev_mismatch(rev: i64) -> Error {
    Error::new(
        ErrorCode::RevMismatch,
        format!(
            "the board changed: its current revision is {rev}; load it again or send If-Match: *"
        ),
    )
    .with_details(&RevMismatchDetails {
        etag: etag(rev),
        rev: Some(rev),
        mergeable: None,
    })
}

/// Raw DEFLATE, the `board_state.body`, inkboard `revisions.data` and `board_previews.svg` encoding.
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

/// The store error of a refused envelope: 422 `invalid_board` or `board_too_new`.
pub fn board_error(e: BoardError) -> Error {
    Error::new(e.code(), e.to_string())
}

fn inspect(envelope: &str) -> Result<Inspect> {
    board::inspect(envelope).map_err(board_error)
}

fn split(envelope: &str) -> Result<Split> {
    board::split(envelope).map_err(board_error)
}

fn no_board() -> Error {
    Error::not_found("the item has no board")
}

fn ensure_board(conn: &Connection, item_id: &str) -> Result<()> {
    let row = item_row(conn, item_id)?;
    if row.kind != ItemKind::Board {
        return Err(Error::validation("kind", "the item is not a board"));
    }
    Ok(())
}

fn clear(tx: &Transaction<'_>, item_id: &str) -> Result<()> {
    for sql in [
        "DELETE FROM board_assets WHERE item_id = ?1",
        "DELETE FROM board_previews WHERE item_id = ?1",
        "DELETE FROM board_state WHERE item_id = ?1",
        "DELETE FROM item_links WHERE source_id = ?1 AND kind = 'embed'",
    ] {
        tx.execute(sql, [item_id])?;
    }
    Ok(())
}

/// Replaces the board's `embed` links (§18.8): one `(board, target, 'embed')` row per target of its cards, the
/// board's own id left out. The targets come from [`board::inspect`] (canonical UUIDs, distinct, at most
/// `EMBED_LINKS_MAX`); whether they exist is not checked.
fn set_embed_links(tx: &Transaction<'_>, item_id: &str, targets: &[String]) -> Result<()> {
    tx.execute(
        "DELETE FROM item_links WHERE source_id = ?1 AND kind = 'embed'",
        [item_id],
    )?;
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO item_links (source_id, target_id, kind) VALUES (?1, ?2, 'embed')",
    )?;
    for target in targets.iter().filter(|t| t.as_str() != item_id) {
        insert.execute(params![item_id, target])?;
    }
    Ok(())
}

/// Stores a validated, split envelope as rev 1 of `item_id`, written by the server. Asset bytes need `blobs`.
fn store_initial(
    tx: &Transaction<'_>,
    blobs: Option<&BlobStore>,
    item_id: &str,
    inspected: Inspect,
    split: Split,
    now: i64,
) -> Result<()> {
    let body = deflate(split.stripped.as_bytes())?;
    let sha: [u8; 32] = Sha256Hasher::digest(split.stripped.as_bytes()).into();
    clear(tx, item_id)?;
    tx.execute(
        "INSERT INTO board_state (item_id, body, body_sha256, raw_len, format_version, engine_version, rev, seq,
           meta, last_token_id, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, ?7, NULL, ?8)",
        params![
            item_id,
            body,
            &sha[..],
            split.stripped.len() as i64,
            split.format_version,
            SERVER_ENGINE,
            serde_json::to_string(&inspected.meta)?,
            now
        ],
    )?;
    for a in split.assets {
        if a.bytes.is_empty() {
            continue;
        }
        let blobs = blobs.ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                "an initial board with assets needs the blob store",
            )
        })?;
        let blob = blobs.put(tx, &a.bytes, now)?;
        tx.execute(
            "INSERT INTO board_assets (item_id, asset_id, sha256, mime) VALUES (?1, ?2, ?3, ?4)",
            params![item_id, a.id, blob.sha256.as_str(), a.mime],
        )?;
    }
    set_body_text(tx, item_id, &inspected.text)?;
    set_embed_links(tx, item_id, &inspected.embeds)
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

    #[test]
    fn if_match_parsing() {
        assert_eq!(IfMatch::parse("*"), IfMatch::Any);
        assert_eq!(IfMatch::parse(" * "), IfMatch::Any);
        assert_eq!(IfMatch::parse("\"r3\""), IfMatch::Revs(vec![3]));
        assert_eq!(IfMatch::parse("W/\"r3\""), IfMatch::Revs(vec![3]));
        assert_eq!(
            IfMatch::parse("\"r1\", W/\"r2\", \"d4\", junk"),
            IfMatch::Revs(vec![1, 2])
        );
        assert_eq!(IfMatch::parse("\"d4\""), IfMatch::Revs(vec![]));
        assert!(IfMatch::parse("\"r3\"").matches(3));
        assert!(!IfMatch::parse("\"r3\"").matches(4));
        assert!(!IfMatch::parse("\"m3\"").matches(3));
        assert!(IfMatch::Any.matches(99));
        assert_eq!(etag(7), "\"r7\"");
    }

    #[test]
    fn rev_mismatch_details() {
        let e = rev_mismatch(5);
        assert!(e.is(ErrorCode::RevMismatch));
        assert_eq!(
            e.details,
            Some(serde_json::json!({"etag": "\"r5\"", "rev": 5}))
        );
    }
}
