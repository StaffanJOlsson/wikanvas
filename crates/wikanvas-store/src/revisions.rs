//! Revisions: the auto policy, pre-api recovery points and pruning (owner: W1).
//!
//! §5 "Revision policy": on a content save, an `auto` revision of the new state is written when the item's newest
//! revision (of any reason) is more than 10 minutes old, or there is none. D53 / §5 "Recovery points": before a
//! token's content write, [`pre_api`] stores a `pre-api` revision of the current state unless the last content write
//! came from the same token or the newest revision already holds the current state (`source_seq`). [`prune`] is the
//! daily thinning of §5.

use std::collections::BTreeMap;

use wikanvas_core::api::{ErrorCode, ItemKind};
use rusqlite::{OptionalExtension, Transaction, params};
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

use crate::error::{Error, Result};
use crate::shared::{WriteBy, new_id};

/// How long after the newest revision a save writes an `auto` revision (§5).
pub const AUTO_INTERVAL_MS: i64 = 10 * 60 * 1000;

/// The stored format of a revision (lookup table `content_formats`, D37).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentFormat {
    /// `data` is a full Yjs v1 update; `format_version` 1.
    Yjs,
    /// `data` is the deflated stripped envelope; `format_version` is the envelope version (2).
    Inkboard,
}

impl ContentFormat {
    /// The `content_formats` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ContentFormat::Yjs => "yjs",
            ContentFormat::Inkboard => "inkboard",
        }
    }
}

/// Why a revision was written (lookup table `revision_reasons`, D37).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionReason {
    Auto,
    Manual,
    PreRestore,
    Restore,
    Import,
    PreApi,
}

impl RevisionReason {
    /// The `revision_reasons` value.
    pub fn as_str(self) -> &'static str {
        match self {
            RevisionReason::Auto => "auto",
            RevisionReason::Manual => "manual",
            RevisionReason::PreRestore => "pre-restore",
            RevisionReason::Restore => "restore",
            RevisionReason::Import => "import",
            RevisionReason::PreApi => "pre-api",
        }
    }
}

/// A revision to write: the content of the `revisions` row except id, item, title (read from the item), reason,
/// size and time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionDraft {
    pub format: ContentFormat,
    pub format_version: i64,
    /// Yjs: the full encoded state; inkboard: the deflated stripped envelope.
    pub data: Vec<u8>,
    /// Yjs: `pm_json` at that time (display only).
    pub preview: Option<String>,
    /// Inkboard: `{asset_id: sha256}` (blob GC roots).
    pub assets: Option<BTreeMap<String, String>>,
    /// Yjs: the attachment ids the document references (attachment GC roots).
    pub attachment_ids: Option<Vec<String>>,
    /// The Wikanvas TipTap schema version (§6.1); 1 for boards.
    pub schema_version: i64,
    /// Yjs: the seq it captures; inkboard: the rev (D53).
    pub source_seq: Option<i64>,
    pub author_id: Option<String>,
}

impl RevisionDraft {
    /// A Yjs revision of the state `data` at `seq`.
    pub fn yjs(
        data: Vec<u8>,
        seq: i64,
        attachment_ids: Vec<String>,
        author_id: Option<&str>,
    ) -> Self {
        RevisionDraft {
            format: ContentFormat::Yjs,
            format_version: 1,
            data,
            preview: None,
            assets: None,
            attachment_ids: Some(attachment_ids),
            schema_version: 1,
            source_seq: Some(seq),
            author_id: author_id.map(str::to_owned),
        }
    }

    /// A board revision of the deflated stripped envelope `body` at `rev`.
    pub fn inkboard(
        body: Vec<u8>,
        format_version: i64,
        rev: i64,
        assets: BTreeMap<String, String>,
        author_id: Option<&str>,
    ) -> Self {
        RevisionDraft {
            format: ContentFormat::Inkboard,
            format_version,
            data: body,
            preview: None,
            assets: Some(assets),
            attachment_ids: None,
            schema_version: 1,
            source_seq: Some(rev),
            author_id: author_id.map(str::to_owned),
        }
    }
}

/// Writes a revision of `item_id` with `reason`, titled with the item's current title. Returns its id.
pub fn write(
    tx: &Transaction<'_>,
    item_id: &str,
    reason: RevisionReason,
    draft: RevisionDraft,
    now: i64,
) -> Result<String> {
    let title: String = tx
        .query_row("SELECT title FROM items WHERE id = ?1", [item_id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or_else(|| Error::not_found("no such item"))?;
    let id = new_id();
    let assets = draft
        .assets
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let attachment_ids = draft
        .attachment_ids
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    tx.execute(
        "INSERT INTO revisions (id, item_id, format, format_version, data, preview, assets, attachment_ids,
           schema_version, title, reason, source_seq, label, author_id, byte_size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, ?15)",
        params![
            id,
            item_id,
            draft.format.as_str(),
            draft.format_version,
            draft.data,
            draft.preview,
            assets,
            attachment_ids,
            draft.schema_version,
            title,
            reason.as_str(),
            draft.source_seq,
            draft.author_id,
            draft.data.len() as i64,
            now,
        ],
    )?;
    Ok(id)
}

/// The §5 auto policy, called by every content save with the new state: writes an `auto` revision when the item's
/// newest revision is more than [`AUTO_INTERVAL_MS`] old or there is none. Returns whether it wrote one.
pub fn maybe_auto(
    tx: &Transaction<'_>,
    item_id: &str,
    draft: RevisionDraft,
    now: i64,
) -> Result<bool> {
    let newest: Option<i64> = tx.query_row(
        "SELECT max(created_at) FROM revisions WHERE item_id = ?1",
        [item_id],
        |r| r.get(0),
    )?;
    if newest.is_some_and(|t| now - t <= AUTO_INTERVAL_MS) {
        return Ok(false);
    }
    write(tx, item_id, RevisionReason::Auto, draft, now)?;
    Ok(true)
}

/// D53: before a token-authenticated content write, stores a `pre-api` revision of the item's current state (with
/// `source_seq`), unless `by` has no token (session, seed, admin), the last content write came from the same token
/// (`last_token_id`), or the newest revision's `source_seq` equals the current seq (doc: `next_seq - 1`) or rev
/// (board). Returns whether it wrote one. It does not set `last_token_id`: the write that follows does.
pub fn pre_api(
    tx: &Transaction<'_>,
    item_id: &str,
    kind: ItemKind,
    by: &WriteBy<'_>,
    now: i64,
) -> Result<bool> {
    let Some(token_id) = by.token_id else {
        return Ok(false);
    };
    let author: Option<String> = tx
        .query_row(
            "SELECT updated_by FROM items WHERE id = ?1 AND kind = ?2",
            params![item_id, kind.as_str()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::not_found(format!("no such {kind}")))?;
    let newest_source: Option<i64> = tx
        .query_row(
            "SELECT source_seq FROM revisions WHERE item_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
            [item_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let draft = match kind {
        ItemKind::Doc => {
            let (last_token, next_seq, ydoc, preview): (Option<String>, i64, Vec<u8>, Option<String>) = tx
                .query_row(
                    "SELECT last_token_id, next_seq, ydoc, pm_json FROM doc_state WHERE item_id = ?1",
                    [item_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?
                .ok_or_else(|| Error::not_found("the document has no state"))?;
            let seq = next_seq - 1;
            if last_token.as_deref() == Some(token_id) || newest_source == Some(seq) {
                return Ok(false);
            }
            let mut stmt =
                tx.prepare("SELECT data FROM doc_updates WHERE item_id = ?1 ORDER BY seq")?;
            let updates = stmt
                .query_map([item_id], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let state = full_state(&ydoc, &updates)?;
            let attachment_ids = tx
                .prepare(
                    "SELECT attachment_id FROM item_attachments WHERE item_id = ?1 ORDER BY attachment_id",
                )?
                .query_map([item_id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut draft = RevisionDraft::yjs(state, seq, attachment_ids, author.as_deref());
            draft.preview = preview;
            draft
        }
        ItemKind::Board => {
            let (last_token, rev, body, format_version): (Option<String>, i64, Vec<u8>, i64) = tx
                .query_row(
                    "SELECT last_token_id, rev, body, format_version FROM board_state WHERE item_id = ?1",
                    [item_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?
                .ok_or_else(|| Error::not_found("the board has no state"))?;
            if last_token.as_deref() == Some(token_id) || newest_source == Some(rev) {
                return Ok(false);
            }
            let assets = tx
                .prepare("SELECT asset_id, sha256 FROM board_assets WHERE item_id = ?1")?
                .query_map([item_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
            RevisionDraft::inkboard(body, format_version, rev, assets, author.as_deref())
        }
    };
    write(tx, item_id, RevisionReason::PreApi, draft, now)?;
    Ok(true)
}

/// The daily revision prune (§5): per item, every revision from the last 24 h stays; older ones are thinned to the
/// newest per hour while under 7 days old, per day while under 90 days, then per week. Buckets are fixed calendar
/// spans of unix time (UTC), so a revision kept once stays kept until it reaches a coarser tier. `manual` revisions are
/// never pruned (nor counted). Returns how many revisions it deleted.
pub fn prune(tx: &Transaction<'_>, now: i64) -> Result<u64> {
    use wikanvas_core::time::{DAY_MS, HOUR_MS, WEEK_MS};
    let rows = tx
        .prepare(
            "SELECT id, item_id, created_at FROM revisions WHERE reason <> 'manual'
             ORDER BY item_id, created_at DESC, id DESC",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut kept: std::collections::HashSet<(String, u8, i64)> = std::collections::HashSet::new();
    let mut doomed = Vec::new();
    for (id, item, created_at) in rows {
        let age = now - created_at;
        let (tier, span) = if age < DAY_MS {
            continue;
        } else if age < 7 * DAY_MS {
            (1, HOUR_MS)
        } else if age < 90 * DAY_MS {
            (2, DAY_MS)
        } else {
            (3, WEEK_MS)
        };
        if !kept.insert((item, tier, created_at.div_euclid(span))) {
            doomed.push(id);
        }
    }
    let mut stmt =
        tx.prepare("DELETE FROM revisions WHERE id IN (SELECT value FROM json_each(?1))")?;
    let mut deleted = 0;
    for chunk in doomed.chunks(500) {
        deleted += stmt.execute([serde_json::to_string(chunk)?])? as u64;
    }
    Ok(deleted)
}

/// The compacted state plus the stored updates as one full Yjs v1 update.
fn full_state(ydoc: &[u8], updates: &[Vec<u8>]) -> Result<Vec<u8>> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        for bytes in std::iter::once(ydoc).chain(updates.iter().map(Vec::as_slice)) {
            let update = Update::decode_v1(bytes).map_err(|e| {
                Error::new(
                    ErrorCode::Internal,
                    format!("stored Yjs update does not decode: {e}"),
                )
            })?;
            txn.apply_update(update).map_err(|e| {
                Error::new(
                    ErrorCode::Internal,
                    format!("stored Yjs update does not apply: {e}"),
                )
            })?;
        }
    }
    let txn = doc.transact();
    Ok(txn.encode_state_as_update_v1(&StateVector::default()))
}
