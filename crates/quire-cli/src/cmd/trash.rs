//! The `quire trash` commands (owner: W8)

use super::{Ctx, Res};
use crate::args::TrashCmd;
use crate::http::Req;

/// `trash list`, and `trash purge (<i> | --collection <c> | --all) --yes` (clap refuses a purge without `--yes`:
/// exit 2, before any request).
pub fn run(ctx: &Ctx, cmd: TrashCmd) -> Res {
    match cmd {
        TrashCmd::List => ctx.call(Req::get("/api/v1/trash"), None),
        TrashCmd::Purge(a) => {
            if let Some(item) = &a.item {
                let id = ctx.item_id(item)?;
                ctx.call(Req::delete(format!("/api/v1/trash/items/{id}")), Some(&id))
            } else if let Some(c) = &a.collection {
                let id = ctx.collection_id(c)?;
                ctx.call(
                    Req::delete(format!("/api/v1/trash/collections/{id}")),
                    Some(&id),
                )
            } else {
                ctx.call(Req::delete("/api/v1/trash"), None)
            }
        }
    }
}
