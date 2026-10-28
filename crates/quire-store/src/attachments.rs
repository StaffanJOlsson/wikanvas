//! Attachments: document images and the workspace logo (owner: W2).
//!
//! An attachment is a named reference to a blob (`attachments.sha256`), created by `POST /api/v1/attachments` in the
//! same writer transaction that commits the blob, so a new blob is referenced the moment its row exists (D26).
//! `item_id` is the upload context only and is never used for authorization (D36). Filenames are sanitized at upload
//! (§8: no control characters, `"`, `\` or `/`; at most 200 characters; `file` when nothing is left).

use quire_core::api::Attachment;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::blobs::BlobRef;
use crate::error::{Error, Result};
use crate::settings::attachment_url;
use crate::shared::{new_id, parse_uuid};

/// The longest stored filename, in characters.
pub const MAX_FILENAME_CHARS: usize = 200;

/// One attachment with its blob's metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentRow {
    pub id: String,
    pub item_id: Option<String>,
    pub sha256: String,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub created_by: Option<String>,
    pub created_at: i64,
}

impl AttachmentRow {
    /// The `POST /attachments` response.
    pub fn to_api(&self) -> Result<Attachment> {
        Ok(Attachment {
            id: parse_uuid(&self.id)?,
            url: attachment_url(&self.id),
            sha256: self.sha256.clone(),
            mime: self.mime.clone(),
            size: self.size,
            width: self.width,
            height: self.height,
        })
    }
}

/// The stored form of a client's filename (§8): control characters (CR and LF included), `"`, `\` and `/` removed,
/// surrounding whitespace trimmed, cut to 200 characters; `file` when nothing is left.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\\' | '/'))
        .collect();
    let trimmed: String = cleaned.trim().chars().take(MAX_FILENAME_CHARS).collect();
    let trimmed = trimmed.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "file".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Inserts an attachment for a committed `blob`. `item_id` must name an existing item (404 `not_found` otherwise);
/// `filename` is sanitized here.
pub fn create(
    tx: &Transaction<'_>,
    blob: &BlobRef,
    item_id: Option<&str>,
    filename: &str,
    created_by: &str,
    now: i64,
) -> Result<AttachmentRow> {
    if let Some(item) = item_id {
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM items WHERE id = ?1)",
            [item],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::not_found("no such item"));
        }
    }
    let id = new_id();
    let filename = sanitize_filename(filename);
    tx.prepare_cached(
        "INSERT INTO attachments (id, item_id, sha256, filename, mime, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?
    .execute(params![
        id,
        item_id,
        blob.sha256.as_str(),
        filename,
        blob.mime,
        created_by,
        now
    ])?;
    Ok(AttachmentRow {
        id,
        item_id: item_id.map(str::to_owned),
        sha256: blob.sha256.as_str().to_owned(),
        filename,
        mime: blob.mime.clone(),
        size: blob.size,
        width: blob.width,
        height: blob.height,
        created_by: Some(created_by.to_owned()),
        created_at: now,
    })
}

/// An attachment by id, with its blob's size and dimensions.
pub fn get(conn: &Connection, id: &str) -> Result<Option<AttachmentRow>> {
    Ok(conn
        .prepare_cached(
            "SELECT a.id, a.item_id, a.sha256, a.filename, a.mime, b.size, b.width, b.height, a.created_by,
                    a.created_at
             FROM attachments a JOIN blobs b ON b.sha256 = a.sha256 WHERE a.id = ?1",
        )?
        .query_row([id], |r| {
            Ok(AttachmentRow {
                id: r.get(0)?,
                item_id: r.get(1)?,
                sha256: r.get(2)?,
                filename: r.get(3)?,
                mime: r.get(4)?,
                size: r.get::<_, i64>(5)? as u64,
                width: r.get(6)?,
                height: r.get(7)?,
                created_by: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(sanitize_filename("a\r\nb\".png"), "ab.png");
        assert_eq!(sanitize_filename("../../etc/passwd"), "....etcpasswd");
        assert_eq!(sanitize_filename("C:\\x\\y.txt"), "C:xy.txt");
        assert_eq!(sanitize_filename("  \u{0}\u{7f}  "), "file");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename(&"é".repeat(300)).chars().count(), 200);
        assert_eq!(sanitize_filename("Résumé 2026.pdf"), "Résumé 2026.pdf");
    }
}
