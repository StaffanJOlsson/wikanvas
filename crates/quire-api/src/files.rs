//! Attachments and on-demand backups (ARCHITECTURE §7.2, §8 "Uploads"; owner: W2). Uploads are raw bodies sent as
//! `application/octet-stream`; the server sniffs the type.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Query of `POST /attachments`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct UploadQuery {
    /// The item the file is uploaded for (context only, never used for authorization, D36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<Uuid>,
    /// Sanitized and cut to 200 characters; default `file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// Response of `POST /attachments` (201).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Attachment {
    pub id: Uuid,
    /// `/api/v1/attachments/<id>`.
    pub url: String,
    /// 64 lowercase hex characters.
    pub sha256: String,
    /// Sniffed from the bytes.
    pub mime: String,
    pub size: u64,
    /// Images only, read from the header.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Response of `POST /admin/backup`: a file on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct BackupResult {
    pub path: String,
    pub size: u64,
}
