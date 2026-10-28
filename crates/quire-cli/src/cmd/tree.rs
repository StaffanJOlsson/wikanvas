//! The `quire tree` command (owner: W8)

use super::{Ctx, Res, print_json_text};
use crate::args::TreeArgs;
use crate::error::Failure;
use crate::http::Req;
use crate::{human, output};

/// `GET /collections/{id}/tree`: `{collection, items}`; the human form is indented.
pub fn run(ctx: &Ctx, a: TreeArgs) -> Res {
    let id = ctx.collection_id(&a.collection)?;
    let text = ctx
        .client()?
        .send_text(&Req::get(format!("/api/v1/collections/{id}/tree")))?
        .text;
    if output::json() {
        return print_json_text(&text);
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("GET …/tree: not JSON: {e}")))?;
    output::print_text(&human::tree(&v))
}
