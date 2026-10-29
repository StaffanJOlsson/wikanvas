//! The `wikanvas collection` commands (owner: W8; access and members: D152)
//!
//! `--access` on `create` and `update` is the collection's default access; `collection member` lists, sets and
//! removes memberships, which take a person by id, by email (through `GET /users`, the owner and admins) or by their
//! name in `GET /directory`.

use serde_json::{Map, Value, json};

use super::{Ctx, Res, print_json_text};
use crate::args::{CollectionCmd, MemberCmd};
use crate::error::Failure;
use crate::http::Req;
use crate::output;
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
                ("default_access", a.access),
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
                ("default_access", a.access),
            ] {
                if let Some(v) = v {
                    body.insert(k.into(), v.into());
                }
            }
            if body.is_empty() {
                return Err(Failure::usage(
                    "collection update: pass --name, --description, --icon, --color, --sort or --access",
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
        CollectionCmd::Member(m) => member(ctx, m),
    }
}

fn member(ctx: &Ctx, cmd: MemberCmd) -> Res {
    match cmd {
        MemberCmd::List(a) => {
            let id = ctx.collection_id(&a.collection)?;
            let text = ctx
                .client()?
                .send_text(&Req::get(format!("/api/v1/collections/{id}/members")))?
                .text;
            if output::json() {
                return print_json_text(&text);
            }
            let v: Value = serde_json::from_str(&text)
                .map_err(|e| Failure::bad_response(format!("GET …/members: not JSON: {e}")))?;
            let mut out = String::new();
            for m in v.as_array().into_iter().flatten() {
                let u = &m["user"];
                out.push_str(&format!(
                    "{}  {:<6}  {:<6}  {:<6}  {}{}\n",
                    u["id"].as_str().unwrap_or("-"),
                    m["access"].as_str().unwrap_or("-"),
                    m["effective_access"].as_str().unwrap_or("-"),
                    u["role"].as_str().unwrap_or("-"),
                    crate::human::clean(u["name"].as_str().unwrap_or("-")),
                    if m["disabled"] == true {
                        "  (disabled)"
                    } else {
                        ""
                    }
                ));
            }
            output::print_text(&out)
        }
        MemberCmd::Set(a) => {
            let id = ctx.collection_id(&a.collection)?;
            let user = super::user::person_id(ctx, &a.user)?;
            ctx.call(
                Req::put(format!("/api/v1/collections/{id}/members/{user}"))
                    .json(&json!({ "access": a.access })),
                None,
            )
        }
        MemberCmd::Remove(a) => {
            let id = ctx.collection_id(&a.collection)?;
            let user = super::user::person_id(ctx, &a.user)?;
            ctx.call(
                Req::delete(format!("/api/v1/collections/{id}/members/{user}")),
                Some(&user),
            )
        }
    }
}
