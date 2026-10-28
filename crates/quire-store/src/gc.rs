//! Attachment and blob garbage collection, and the orphan file sweep (owner: W2).
//!
//! §5, D26, D36, run by the 6-hourly job after the Trash purge:
//! 1. [`attachments`]: rows older than 24 h that no item (`item_attachments`), no revision
//!    (`json_each(revisions.attachment_ids)`) and not the workspace logo setting reference.
//! 2. [`blobs`]: rows that no attachment, board asset, avatar or revision (`json_each(revisions.assets)`) references,
//!    deleted in one writer closure that commits and then unlinks their files while it still holds the writer, so a
//!    concurrent save (whose commit renames into `files/` inside the writer) can never end with a row whose file is
//!    gone. Then the sweep, still under the writer: files under `files/` without a row and older than 1 h, and
//!    staging files under `tmp/` older than 24 h (left by a crash).

use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::{TransactionBehavior, params};

use crate::blobs::{BlobStore, Sha256};
use crate::db::Db;
use crate::error::Result;
use crate::settings::WORKSPACE_LOGO;

/// Unreferenced attachments are kept this long (D36).
pub const ATTACHMENT_GRACE_MS: i64 = 24 * 3_600_000;
/// Files under `files/` without a row are kept this long (D26).
pub const ORPHAN_FILE_AGE: Duration = Duration::from_secs(3600);
/// Staging files under `tmp/` are kept this long.
pub const STALE_TMP_AGE: Duration = Duration::from_secs(24 * 3600);

/// What a [`blobs`] run removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlobGc {
    /// Unreferenced blob rows deleted (their files unlinked).
    pub blobs: u64,
    /// Files under `files/` without a row, older than 1 h.
    pub orphan_files: u64,
    /// Staging files under `tmp/` older than 24 h.
    pub stale_tmp: u64,
}

/// Deletes the attachments created before `now - 24 h` that nothing references; returns the number deleted.
pub fn attachments(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<u64> {
    let n = tx.execute(
        "DELETE FROM attachments
         WHERE created_at < ?1
           AND id NOT IN (SELECT attachment_id FROM item_attachments)
           AND id NOT IN (SELECT j.value FROM revisions r, json_each(r.attachment_ids) j
                          WHERE r.attachment_ids IS NOT NULL)
           AND id IS NOT (SELECT json_extract(value, '$') FROM settings WHERE key = ?2)",
        params![now - ATTACHMENT_GRACE_MS, WORKSPACE_LOGO],
    )?;
    Ok(n as u64)
}

/// The blob rows nothing references (§5 "Blob GC").
const UNREFERENCED: &str = "SELECT sha256 FROM blobs
     WHERE sha256 NOT IN (SELECT sha256 FROM attachments)
       AND sha256 NOT IN (SELECT sha256 FROM board_assets)
       AND sha256 NOT IN (SELECT avatar_sha256 FROM users WHERE avatar_sha256 IS NOT NULL)
       AND sha256 NOT IN (SELECT j.value FROM revisions r, json_each(r.assets) j WHERE r.assets IS NOT NULL)";

/// Blob GC and the sweep (see the module docs), in one closure on the writer.
pub async fn blobs(db: &Db, store: &BlobStore) -> Result<BlobGc> {
    let store = store.clone();
    db.with_writer(move |conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let dead: Vec<String> = tx
            .prepare(UNREFERENCED)?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        {
            let mut delete = tx.prepare("DELETE FROM blobs WHERE sha256 = ?1")?;
            for sha in &dead {
                delete.execute([sha])?;
            }
        }
        tx.commit()?;
        let mut report = BlobGc::default();
        for sha in &dead {
            if let Ok(sha) = Sha256::parse(sha) {
                match std::fs::remove_file(store.path(&sha)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => tracing::warn!(%sha, error = %e, "blob GC: cannot unlink"),
                }
            }
            report.blobs += 1;
        }
        // Still holding the writer: no commit can rename a file into files/ while the sweep runs.
        let known = |sha: &str| -> Result<bool> {
            Ok(conn
                .prepare_cached("SELECT EXISTS (SELECT 1 FROM blobs WHERE sha256 = ?1)")?
                .query_row([sha], |r| r.get(0))?)
        };
        report.orphan_files = sweep_files(&store.files_dir(), ORPHAN_FILE_AGE, &known)?;
        report.stale_tmp = sweep_tmp(&store.tmp_dir(), STALE_TMP_AGE);
        Ok(report)
    })
    .await
}

fn older_than(meta: &std::fs::Metadata, age: Duration) -> bool {
    meta.modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|d| d >= age)
}

/// Removes files under `files/aa/bb/` whose name has no row (or is not a sha256) and that are older than `age`.
fn sweep_files(root: &Path, age: Duration, known: &dyn Fn(&str) -> Result<bool>) -> Result<u64> {
    let mut removed = 0;
    let Ok(level1) = std::fs::read_dir(root) else {
        return Ok(0);
    };
    for a in level1.flatten() {
        let Ok(level2) = std::fs::read_dir(a.path()) else {
            continue;
        };
        for b in level2.flatten() {
            let Ok(files) = std::fs::read_dir(b.path()) else {
                continue;
            };
            for f in files.flatten() {
                let Ok(meta) = f.metadata() else { continue };
                if !meta.is_file() || !older_than(&meta, age) {
                    continue;
                }
                let name = f.file_name();
                let keep = match name.to_str() {
                    Some(n) if Sha256::parse(n).is_ok() => known(n)?,
                    _ => false,
                };
                if !keep && std::fs::remove_file(f.path()).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

/// Removes staging files older than `age` directly under `tmp/`.
fn sweep_tmp(dir: &Path, age: Duration) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_file() && older_than(&meta, age) && std::fs::remove_file(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}
