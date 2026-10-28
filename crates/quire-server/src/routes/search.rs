//! Routes /api/v1/search and /api/v1/search/titles (owner: W1)
//!
//! §5 "Search query", D27, D46: full-text search with the quoted query builder, filters, snippets and cursor pages
//! (offset, at most 1,000 results, for relevance; keyset for `sort=updated`), and the title search of the quick
//! switcher and mentions.

use axum::Json;
use axum::extract::{Query, State};
use quire_api::{ErrorBody, Page, SearchHit, SearchQuery, TitleHit, TitleSearchQuery};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::extract::{Auth, Read};
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(search))
        .routes(routes!(search_titles))
}

/// Searches titles and bodies. Terms are words (the last one a prefix), `"…"` keeps a phrase, `-word` excludes;
/// nothing is FTS syntax. A query without a positive term finds nothing. Archived items only with
/// `include_archived=true`; drafts unless `include_drafts=false`.
#[utoipa::path(
    get,
    path = "/search",
    tag = "search",
    operation_id = "search",
    security(("bearer" = ["read"]), ("session" = [])),
    params(SearchQuery),
    responses(
        (status = 200, description = "One page of hits; snippets mark hits as parts with `hit: true`",
            body = Page<SearchHit>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `q` (over 500 characters), `limit`, `cursor` or `query`)",
            body = ErrorBody),
    )
)]
async fn search(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Page<SearchHit>>, ApiError> {
    let viewer = auth.viewer();
    let page = state
        .db
        .read(move |conn| quire_store::search::search(conn, &viewer, &q))
        .await?;
    Ok(Json(page))
}

/// Items whose title matches `q` (the last word a prefix, accents folded), best first; a blank `q` lists the most
/// recently updated items. Visible and unarchived only.
#[utoipa::path(
    get,
    path = "/search/titles",
    tag = "search",
    operation_id = "search_titles",
    security(("bearer" = ["read"]), ("session" = [])),
    params(TitleSearchQuery),
    responses(
        (status = 200, description = "At most `limit` items with their paths", body = Vec<TitleHit>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `q`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn search_titles(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<TitleSearchQuery>,
) -> Result<Json<Vec<TitleHit>>, ApiError> {
    let viewer = auth.viewer();
    let hits = state
        .db
        .read(move |conn| quire_store::search::titles(conn, &viewer, &q.q, q.limit))
        .await?;
    Ok(Json(hits))
}
