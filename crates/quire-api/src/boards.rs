//! Board DTOs (ARCHITECTURE §6.2, §7.2, §7.5; owner: W6). The board body itself is the native Inkboard envelope
//! `{"format":"inkboard","version":2,"document":…}`, sent and returned as raw JSON, never as a DTO (D9, D51).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::common::Timestamp;

/// What the server knows about a board without loading it: `board_state.meta` plus the revision and preview state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct BoardMeta {
    /// The board revision; its ETag is `"r{rev}"`.
    pub rev: i64,
    pub pages: Vec<BoardPageMeta>,
    /// Number of objects on all pages.
    pub objects: u64,
    /// Background colour, `#rrggbb`.
    pub background: String,
    /// Size of the full envelope in bytes.
    pub bytes: u64,
    /// The revision the stored SVG previews were made from, `null` before the first preview.
    pub preview_rev: Option<i64>,
    /// The first 4 KB of the board's search text (page names, labels, table cells, LaTeX), one entry per line.
    pub text_excerpt: String,
}

/// One page of a board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct BoardPageMeta {
    /// Inkboard's page id.
    pub id: String,
    pub name: String,
    /// Size of the page's SVG preview in bytes; `null` while there is none (list views show a placeholder when it is
    /// over 2 MB).
    pub svg_len: Option<u64>,
}

/// Response of `PUT /items/{id}/board`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct BoardPutResult {
    pub rev: i64,
    /// `"r{rev}"`, the value for the next `If-Match`.
    pub etag: String,
    pub updated_at: Timestamp,
    pub meta: BoardMeta,
}

/// `?rev=` of `GET /items/{id}/board/pages/{n}`: without it, the current page (`no-cache`); with the current rev,
/// immutable; with another rev, a 307 to the current one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct BoardPreviewQuery {
    /// The board revision the page is wanted at (`BoardMeta.rev`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<i64>,
}
