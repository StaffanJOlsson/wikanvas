//! Cursor pagination: `Page<T>` and the `?cursor=&limit=` query (D46, ARCHITECTURE §7.1; owner: W1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

/// `limit` when the query has none.
pub const DEFAULT_LIMIT: u32 = 25;
/// The largest `limit` a paginated list accepts.
pub const MAX_LIMIT: u32 = 100;
/// Relevance-sorted search pages by offset up to this many results.
pub const MAX_OFFSET_RESULTS: u32 = 1000;

/// One page of a paginated list. `next_cursor` is `null` on the last page. Cursors are opaque, stable under
/// concurrent inserts and deletes, and valid only with the query parameters that produced them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// A page with these items; `next_cursor` is `None` when this is the last one.
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Page { items, next_cursor }
    }
}

/// `?cursor=&limit=` of a paginated list without other parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, IntoParams)]
#[into_params(parameter_in = Query)]
#[ts(export)]
pub struct PageQuery {
    /// The `next_cursor` of the previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Page size, 1–100 (default 25).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[param(minimum = 1, maximum = 100)]
    pub limit: Option<u32>,
}
