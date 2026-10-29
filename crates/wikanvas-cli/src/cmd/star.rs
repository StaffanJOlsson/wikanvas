//! The `wikanvas star` commands (owner: W8)

use serde_json::json;

use super::{Ctx, Res};
use crate::args::StarCmd;
use crate::http::Req;

pub fn run(ctx: &Ctx, cmd: StarCmd) -> Res {
    match cmd {
        StarCmd::Add(a) => {
            let id = ctx.item_id(&a.item)?;
            let body = match ctx.position(&a.position, false)? {
                Some(p) => json!({ "position": p }),
                None => json!({}),
            };
            ctx.call(
                Req::put(format!("/api/v1/items/{id}/star")).json(&body),
                Some(&id),
            )
        }
        StarCmd::Remove(a) => {
            let id = ctx.item_id(&a.item)?;
            ctx.call(Req::delete(format!("/api/v1/items/{id}/star")), Some(&id))
        }
    }
}
