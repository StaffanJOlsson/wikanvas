//! The `wikanvas search` command: full text, or titles with --titles (owner: W8)

use super::{Ctx, Res};
use crate::args::SearchArgs;
use crate::error::Failure;
use crate::http::Req;

pub fn run(ctx: &Ctx, a: SearchArgs) -> Res {
    if a.titles {
        if a.list.all || a.list.cursor.is_some() {
            return Err(Failure::usage("search --titles has no --all or --cursor"));
        }
        if a.list.limit.is_some_and(|n| n > 20) {
            return Err(Failure::usage("search --titles --limit is at most 20"));
        }
        if a.collection.is_some()
            || a.kind.is_some()
            || a.updated_after.is_some()
            || a.include_archived
            || a.no_drafts
            || a.sort.is_some()
        {
            return Err(Failure::usage("search --titles takes only --limit"));
        }
        let req = Req::get("/api/v1/search/titles")
            .query("q", a.query)
            .query_opt("limit", a.list.limit.map(|n| n.to_string()));
        return ctx.call(req, None);
    }
    let collection = match &a.collection {
        Some(c) => Some(ctx.collection_id(c)?),
        None => None,
    };
    let mut req = Req::get("/api/v1/search")
        .query("q", a.query)
        .query_opt("collection_id", collection)
        .query_opt("kind", a.kind)
        .query_opt("updated_after", a.updated_after)
        .query_opt("sort", a.sort);
    if a.include_archived {
        req = req.query("include_archived", "true");
    }
    if a.no_drafts {
        req = req.query("include_drafts", "false");
    }
    ctx.list(req, &a.list)
}
