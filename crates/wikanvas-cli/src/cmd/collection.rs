//! The `wikanvas collection` commands (owner: W8)

use serde_json::{Map, Value, json};

use super::{Ctx, Res};
use crate::args::CollectionCmd;
use crate::error::Failure;
use crate::http::Req;
use crate::refs::{self, Ref};

pub fn run(ctx: &Ctx, cmd: CollectionCmd) -> Res {
    match cmd {
        CollectionCmd::List => ctx.call(Req::get("/api/v1/collections"), None),
        CollectionCmd::Get(a) => {
            // GET /collections/{id} also takes a url_id.
            let key = match refs::parse(&a.collection, "collection")? {
                Ref::Uuid(id) | Ref::UrlId(id) => id,
            };
            ctx.call(Req::get(format!("/api/v1/collections/{key}")), None)
        }
        CollectionCmd::Create(a) => {
            let mut body = Map::new();
            body.insert("name".into(), a.name.into());
            for (k, v) in [
                ("description", a.description),
                ("icon", a.icon),
                ("color", a.color),
            ] {
                if let Some(v) = v {
                    body.insert(k.into(), v.into());
                }
            }
            if let Some(p) = ctx.position(&a.position, true)? {
                body.insert("position".into(), p);
            }
            ctx.call(
                Req::post("/api/v1/collections").json(&Value::Object(body)),
                None,
            )
        }
        CollectionCmd::Update(a) => {
            let mut body = Map::new();
            for (k, v) in [
                ("name", a.name),
                ("description", a.description),
                ("icon", a.icon),
                ("color", a.color),
                ("child_order", a.sort),
            ] {
                if let Some(v) = v {
                    body.insert(k.into(), v.into());
                }
            }
            if body.is_empty() {
                return Err(Failure::usage(
                    "collection update: pass --name, --description, --icon, --color or --sort",
                ));
            }
            let id = ctx.collection_id(&a.collection)?;
            ctx.call(
                Req::patch(format!("/api/v1/collections/{id}")).json(&Value::Object(body)),
                None,
            )
        }
        CollectionCmd::Move(a) => {
            let position = ctx
                .position(&a.position, true)?
                .unwrap_or_else(|| json!({}));
            let id = ctx.collection_id(&a.collection)?;
            ctx.call(
                Req::post(format!("/api/v1/collections/{id}/move"))
                    .json(&json!({ "position": position })),
                None,
            )
        }
        CollectionCmd::Delete(a) => {
            let id = ctx.collection_id(&a.collection)?;
            ctx.call(Req::delete(format!("/api/v1/collections/{id}")), Some(&id))
        }
        CollectionCmd::Restore(a) => {
            let id = ctx.collection_id(&a.collection)?;
            ctx.call(
                Req::post(format!("/api/v1/collections/{id}/restore")),
                Some(&id),
            )
        }
        CollectionCmd::Updated(a) => {
            let id = ctx.collection_id(&a.collection)?;
            ctx.list(
                Req::get(format!("/api/v1/collections/{id}/updated")),
                &a.list,
            )
        }
    }
}
