//! Collections, items, the tree, lists, stars, trash and search (ARCHITECTURE §7.1, §7.2; owner: W1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::boards::BoardMeta;
use crate::common::{ChildOrder, ItemKind, Position, Timestamp, double_option, string_enum};

// ---------------------------------------------------------------- collections

/// A collection in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Collection {
    pub id: Uuid,
    /// 10 base62 characters, used in URLs (`/collection/<slug>-<url_id>`).
    pub url_id: String,
    pub name: String,
    pub description: String,
    /// A lucide icon name; `null` shows a letter tile.
    pub icon: Option<String>,
    /// `#RRGGBB`.
    pub color: String,
    pub sort_key: String,
    pub child_order: ChildOrder,
    /// Live items in the collection, at every depth.
    pub item_count: u64,
    pub updated_at: Timestamp,
}

/// Body of `POST /collections`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NewCollection {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `#RRGGBB`; default `#8D8D8D`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Omitted: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

/// Body of `PATCH /collections/{id}`. Omitted fields stay unchanged; `icon: null` clears the icon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CollectionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[ts(as = "Option<String>")]
    #[schema(value_type = Option<String>)]
    pub icon: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_order: Option<ChildOrder>,
}

/// Body of `POST /collections/{id}/move`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MoveCollection {
    pub position: Position,
}

/// Response of `DELETE /collections/{id}` (a soft delete; the collection appears in Trash).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CollectionDeleted {
    pub trashed_items: u64,
}

/// Response of `GET /collections/{id}/tree`: every visible, unarchived item, flat, in `child_order`. ETag
/// `"t{tree_version}.{viewer_epoch}"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CollectionTree {
    pub collection: Collection,
    pub items: Vec<ItemSummary>,
}

// ---------------------------------------------------------------- items

/// An item as the tree and every list show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemSummary {
    pub id: Uuid,
    /// 10 base62 characters, used in URLs (`/doc/<slug>-<url_id>`, `/board/<slug>-<url_id>`).
    pub url_id: String,
    pub collection_id: Uuid,
    /// `null` at the top level of the collection.
    pub parent_id: Option<Uuid>,
    pub kind: ItemKind,
    pub title: String,
    pub sort_key: String,
    /// Visible children.
    pub child_count: u64,
    /// `published_at` is set. Items are published on create; only `unpublish` makes a draft (D23).
    pub published: bool,
    pub archived: bool,
    pub updated_at: Timestamp,
    /// The last body save; `null` when never edited.
    pub content_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// One step of a breadcrumb path, root first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Crumb {
    pub id: Uuid,
    pub url_id: String,
    pub kind: ItemKind,
    pub title: String,
}

/// Response of `GET /items/{id}`, with `ETag: "m{meta_version}"` (never 304).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemDetail {
    pub item: ItemSummary,
    pub collection: Collection,
    /// Ancestors, root first (the item itself excluded).
    pub breadcrumbs: Vec<Crumb>,
    pub starred: bool,
    /// The item and every ancestor are published (D23).
    pub effectively_published: bool,
    pub trashed_at: Option<Timestamp>,
    /// When a trashed item is purged.
    pub purge_at: Option<Timestamp>,
    pub archived_at: Option<Timestamp>,
    /// Boards only.
    pub board: Option<BoardMeta>,
    pub meta_version: i64,
}

/// Body of `POST /items`. The item is published on create (D23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NewItem {
    pub collection_id: Uuid,
    /// Omitted or `null`: top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Omitted: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

/// Body of `PATCH /items/{id}` (`If-Match: "m{n}"` optional, D45).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_width: Option<bool>,
}

/// Body of `POST /items/{id}/move`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct MoveItem {
    /// Omitted: the current collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<Uuid>,
    /// The new parent; `null` for the top level.
    pub parent_id: Option<Uuid>,
    /// Omitted: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

/// Body of `POST /items/{id}/publish`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct PublishRequest {
    /// Publish draft ancestors too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub with_ancestors: bool,
}

/// `?discard_empty=true` of `DELETE /items/{id}`: delete permanently if the item is new (under 24 h), childless,
/// untitled and empty, otherwise 409 `not_empty`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct DeleteItemQuery {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub discard_empty: bool,
}

/// Response of `DELETE /items/{id}`: the number of items trashed (or deleted, with `discard_empty`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemTrashed {
    pub trashed: u64,
}

/// Response of `POST /items/{id}/archive`: the number of items archived (the subtree).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemsArchived {
    pub archived: u64,
}

/// Response of restore and unarchive: the item, and whether its subtree went to the top of the collection because
/// the old parent is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RestoredItem {
    #[serde(flatten)]
    pub item: ItemSummary,
    pub moved_to_top: bool,
}

/// Response of `GET /items/{id}/text`: the derived plain text that search indexes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ItemText {
    pub kind: ItemKind,
    pub title: String,
    pub body: String,
    pub updated_at: Timestamp,
}

/// Body of `PUT /items/{id}/star`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct StarRequest {
    /// Among the stars; omitted: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

// ---------------------------------------------------------------- sidebar, lists, trash

/// Response of `GET /sidebar`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Sidebar {
    pub collections: Vec<Collection>,
    pub stars: Vec<ItemSummary>,
    /// Unpublished items (in M1 only those unpublished through the API or CLI, D23).
    pub drafts_count: u64,
}

/// An entry of a paginated item list (`/archive`, `/drafts`, `/starred`, `/recent`, `/updated`,
/// `/collections/{id}/updated`): the item, its path, and the list's own field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ListedItem {
    #[serde(flatten)]
    pub item: ItemSummary,
    /// Ancestors, root first.
    pub path: Vec<Crumb>,
    /// `/starred` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starred_at: Option<Timestamp>,
    /// `/recent` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_viewed_at: Option<Timestamp>,
    /// `/archive` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<Timestamp>,
}

/// A collection in the trash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TrashedCollection {
    #[serde(flatten)]
    pub collection: Collection,
    pub deleted_at: Timestamp,
    pub purge_at: Timestamp,
}

/// A trash root item (its trashed descendants are counted, not listed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TrashedItem {
    #[serde(flatten)]
    pub item: ItemSummary,
    pub deleted_at: Timestamp,
    pub purge_at: Timestamp,
    pub descendant_count: u64,
    /// Ancestors at the time of listing, root first.
    pub path: Vec<Crumb>,
}

/// Response of `GET /trash` (roots only, unpaginated).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TrashList {
    pub collections: Vec<TrashedCollection>,
    pub items: Vec<TrashedItem>,
}

// ---------------------------------------------------------------- search

string_enum! {
    /// Sort order of `GET /search`.
    #[derive(Default)]
    pub enum SearchSort {
        /// bm25 with the title weighted 8×; pages by offset, up to 1,000 results.
        #[default]
        Relevance = "relevance",
        Updated = "updated",
    }
}

/// Query of `GET /search`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct SearchQuery {
    /// At most 500 characters. `"…"` keeps a phrase, `-word` excludes; a query with no positive term finds nothing.
    pub q: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_after: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_archived: bool,
    /// Default `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_drafts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<SearchSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[param(minimum = 1, maximum = 100)]
    pub limit: Option<u32>,
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SearchHit {
    pub item: ItemSummary,
    /// Ancestors, root first.
    pub path: Vec<Crumb>,
    /// The snippet in parts; `hit` parts are the matched terms. Render as text nodes, never as HTML.
    pub snippet: Vec<SnippetPart>,
    pub score: f64,
}

/// A part of a search snippet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SnippetPart {
    pub text: String,
    pub hit: bool,
}

/// Query of `GET /search/titles` (quick switcher and mentions).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct TitleSearchQuery {
    pub q: String,
    /// 1–20, default 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[param(minimum = 1, maximum = 20)]
    pub limit: Option<u32>,
}

/// One result of `GET /search/titles`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TitleHit {
    pub item: ItemSummary,
    /// Ancestors, root first.
    pub path: Vec<Crumb>,
}
