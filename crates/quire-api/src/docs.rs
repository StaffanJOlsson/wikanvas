//! Document DTOs: Yjs sync, ProseMirror JSON, Markdown writes and the schema description (ARCHITECTURE §6.1, §6.4,
//! §7.2, §7.5; owner: W4). The Markdown bodies themselves are `text/markdown`, not DTOs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::common::{Timestamp, string_enum};

/// The Quire TipTap schema version this server reads and writes (§6.1).
pub const SCHEMA_VERSION: u32 = 1;
/// The Y `XmlFragment` that holds the document.
pub const YFIELD: &str = "default";

/// Query of `GET /items/{id}/doc`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct DocQuery {
    /// Base64 Yjs state vector: return only what the client is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// Response of `GET /items/{id}/doc`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocState {
    /// Base64 Yjs v1 update.
    pub update: String,
    /// Base64 Yjs state vector of the server's document.
    pub state_vector: String,
    /// The newest stored update (the document ETag is `"d{seq}"`).
    pub seq: i64,
    /// Always `default`.
    pub field: String,
    pub schema_version: u32,
}

/// Body of `POST /items/{id}/doc/updates` (up to 16 MiB).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocUpdate {
    /// Base64 Yjs v1 update.
    pub update: String,
    /// The editor's ProseMirror JSON after the update (a cache; rebuildable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub pm_json: Option<Value>,
}

/// Response of `POST /items/{id}/doc/updates`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocUpdateResult {
    pub seq: i64,
    /// Base64 Yjs state vector after the update.
    pub state_vector: String,
    pub updated_at: Timestamp,
}

/// `details` of 409 `missing_deps`: the server's state vector, so the client can send what is missing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MissingDepsDetails {
    /// Base64 Yjs state vector.
    pub state_vector: String,
}

/// Response of `GET /items/{id}/doc/prosemirror`, with `ETag: "d{seq}"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocJson {
    pub schema_version: u32,
    pub seq: i64,
    /// Normalized ProseMirror JSON `{type: "doc", content}` (the shape of `editor.getJSON()`, §6.4).
    #[schema(value_type = Object)]
    pub doc: Value,
}

/// Body of `PUT /items/{id}/doc/prosemirror` (up to 4 MiB, `If-Match` required).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocJsonPut {
    /// `1` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    /// ProseMirror JSON `{type: "doc", content}`; missing attrs get their defaults.
    #[schema(value_type = Object)]
    pub doc: Value,
}

/// Response of the Markdown and ProseMirror PUTs, with the new `ETag`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocWriteResult {
    pub seq: i64,
    /// `"d{seq}"`.
    pub etag: String,
    pub updated_at: Timestamp,
    /// `false` when the document already equalled the target (nothing was written).
    pub changed: bool,
    /// `true` when the write was merged with newer updates (`?merge=true` or `If-Match: *`).
    pub merged: bool,
    pub warnings: Vec<ConversionWarning>,
}

string_enum! {
    /// A lossy step of a conversion (§6.4 table). New codes may be added within v1.
    pub enum ConversionWarningCode {
        EmptyParagraphDropped = "empty_paragraph_dropped",
        HardBreakDropped = "hard_break_dropped",
        TaskListMixed = "task_list_mixed",
        AlertMapped = "alert_mapped",
        MarksInCodeDropped = "marks_in_code_dropped",
        HtmlAsText = "html_as_text",
        UnsafeLink = "unsafe_link",
        UnknownItem = "unknown_item",
        EmbedNotBoard = "embed_not_board",
        ImageWidthDropped = "image_width_dropped",
        ImageHoisted = "image_hoisted",
        TableCellFlattened = "table_cell_flattened",
        TableSpanDropped = "table_span_dropped",
        TableAlignmentDropped = "table_alignment_dropped",
        UnknownNode = "unknown_node",
        UnsupportedMarkdown = "unsupported_markdown",
    }
}

/// One kind of lossy step, with where it first happened and how often.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ConversionWarning {
    pub code: ConversionWarningCode,
    /// A JSON pointer, or `line:column` for Markdown input.
    pub path: Option<String>,
    pub count: u32,
}

string_enum! {
    /// `?title=` of the Markdown GET.
    pub enum MarkdownTitle {
        /// Prepend `# <title>` and a blank line.
        Include = "include",
    }
}

string_enum! {
    /// `?title=` of the Markdown PUT.
    pub enum MarkdownPutTitle {
        /// A leading level-1 heading is removed and becomes the item title.
        FromH1 = "from_h1",
    }
}

/// Query of `GET /items/{id}/doc/markdown`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct MarkdownGetQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<MarkdownTitle>,
    /// Add `Content-Disposition: attachment; filename="<slug>.md"`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub download: bool,
}

/// Query of `PUT /items/{id}/doc/markdown`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct MarkdownPutQuery {
    /// Merge with newer updates when `If-Match` is stale but still reconstructible (D50).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<MarkdownPutTitle>,
}

/// Query of `PUT /items/{id}/doc/prosemirror`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct ProseMirrorPutQuery {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge: bool,
}

/// Response of `GET /schema/doc`: schema v1 as data (`quire-core::schema`, §6.4), byte for byte
/// `fixtures/schema/v1.json`, which the web fixture writer dumps from the live TipTap schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DocSchema {
    pub schema_version: u32,
    /// The root node type (`doc`).
    pub top_node: String,
    /// In schema order (ProseMirror's own `doc` and `text` included).
    pub nodes: Vec<NodeSpec>,
    /// In rank order.
    pub marks: Vec<MarkSpec>,
}

/// A node type of the document schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NodeSpec {
    pub name: String,
    /// e.g. `block`, `inline`.
    pub group: Option<String>,
    /// The ProseMirror content expression, e.g. `inline*`; null for a leaf.
    pub content: Option<String>,
    /// The marks allowed in this node's content: null for ProseMirror's default (every mark in inline content),
    /// `""` for none, `"_"` for all, otherwise mark names.
    pub marks: Option<String>,
    pub inline: bool,
    pub atom: bool,
    pub attrs: Vec<AttrSpec>,
}

/// A mark type of the document schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MarkSpec {
    pub name: String,
    /// Position in the schema's mark order (marks are sorted by it).
    pub rank: u32,
    /// The ProseMirror `excludes` expression: null excludes only the mark itself, `_` every mark.
    pub excludes: Option<String>,
    /// Whether the mark extends to text typed at its end (null: ProseMirror's default, true).
    pub inclusive: Option<bool>,
    pub attrs: Vec<AttrSpec>,
}

/// An attribute of a node or mark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct AttrSpec {
    pub name: String,
    /// The JSON type in TypeScript notation, e.g. `integer`, `boolean`, `string | null`, `integer[] | null`,
    /// `"info" | "tip"`.
    #[serde(rename = "type")]
    pub value_type: String,
    /// The default value (`null` when the attr has none).
    #[schema(value_type = Value)]
    pub default: Value,
}
