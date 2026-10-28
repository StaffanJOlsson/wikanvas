//! The `quire item` commands (owner: W8)

use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

use super::{Ctx, Res, print_json_text};
use crate::args::{ItemCmd, ItemCreate};
use crate::error::Failure;
use crate::http::{self, Req};
use crate::refs::{self, Ref};
use crate::{human, output};

pub fn run(ctx: &Ctx, cmd: ItemCmd) -> Res {
    match cmd {
        ItemCmd::Get(a) => {
            // GET /items/{id} also takes a url_id.
            let key = match refs::parse(&a.item, "item")? {
                Ref::Uuid(id) | Ref::UrlId(id) => id,
            };
            ctx.call(Req::get(format!("/api/v1/items/{key}")), None)
        }
        ItemCmd::Create(a) => create(ctx, a),
        ItemCmd::Update(a) => {
            let mut body = Map::new();
            if let Some(t) = a.title {
                body.insert("title".into(), t.into());
            }
            if let Some(w) = a.full_width {
                body.insert("full_width".into(), w.into());
            }
            if body.is_empty() {
                return Err(Failure::usage("item update: pass --title or --full-width"));
            }
            let id = ctx.item_id(&a.item)?;
            let mut req = Req::patch(format!("/api/v1/items/{id}")).json(&Value::Object(body));
            if let Some(e) = &a.if_match {
                req = req.header("If-Match", super::if_match(e)?);
            }
            ctx.call(req, None)
        }
        ItemCmd::Move(a) => {
            let id = ctx.item_id(&a.item)?;
            let mut body = Map::new();
            let parent = match &a.parent {
                Some(p) => Value::from(ctx.item_id(p)?),
                None => Value::Null,
            };
            body.insert("parent_id".into(), parent);
            if let Some(c) = &a.collection {
                body.insert("collection_id".into(), ctx.collection_id(c)?.into());
            }
            if let Some(p) = ctx.position(&a.position, false)? {
                body.insert("position".into(), p);
            }
            ctx.call(
                Req::post(format!("/api/v1/items/{id}/move")).json(&Value::Object(body)),
                None,
            )
        }
        ItemCmd::Duplicate(a) => post_action(ctx, &a.item, "duplicate"),
        ItemCmd::Unpublish(a) => post_action(ctx, &a.item, "unpublish"),
        ItemCmd::Archive(a) => post_action(ctx, &a.item, "archive"),
        ItemCmd::Unarchive(a) => post_action(ctx, &a.item, "unarchive"),
        ItemCmd::Restore(a) => post_action(ctx, &a.item, "restore"),
        ItemCmd::Publish(a) => {
            let id = ctx.item_id(&a.item)?;
            ctx.call(
                Req::post(format!("/api/v1/items/{id}/publish"))
                    .json(&json!({ "with_ancestors": a.with_ancestors })),
                Some(&id),
            )
        }
        ItemCmd::Trash(a) => {
            let id = ctx.item_id(&a.item)?;
            let mut req = Req::delete(format!("/api/v1/items/{id}"));
            if a.discard_empty {
                req = req.query("discard_empty", "true");
            }
            ctx.call(req, Some(&id))
        }
        ItemCmd::Children(a) => {
            let id = ctx.item_id(&a.item)?;
            ctx.call(Req::get(format!("/api/v1/items/{id}/children")), None)
        }
        ItemCmd::Text(a) => {
            let id = ctx.item_id(&a.item)?;
            ctx.call(Req::get(format!("/api/v1/items/{id}/text")), None)
        }
    }
}

fn post_action(ctx: &Ctx, item: &str, action: &str) -> Res {
    let id = ctx.item_id(item)?;
    ctx.call(Req::post(format!("/api/v1/items/{id}/{action}")), Some(&id))
}

fn raw(text: String) -> Result<Box<RawValue>, Failure> {
    RawValue::from_string(text)
        .map_err(|e| Failure::bad_response(format!("the response is not JSON: {e}")))
}

/// `item create`. With `--md` (documents only) the item is created without a title, then filled with
/// `PUT …/doc/markdown` and `If-Match: "d0"`, then `--title` is set by `PATCH` once the PUT succeeded. If the PUT
/// fails, the new item is discarded (`DELETE ?discard_empty=true`) and the PUT's error is reported. Prints
/// `ItemSummary`, or with `--md` `{item: ItemSummary, doc: DocWriteResult}`.
fn create(ctx: &Ctx, a: ItemCreate) -> Res {
    if a.md.is_some() && a.kind != "doc" {
        return Err(Failure::usage("--md fills a document: use --kind doc"));
    }
    // Read the Markdown first, so an unreadable file never creates an item.
    let md = match &a.md {
        Some(spec) => Some(ctx.read_input(spec, "--md")?),
        None => None,
    };
    let mut body = Map::new();
    body.insert(
        "collection_id".into(),
        ctx.collection_id(&a.collection)?.into(),
    );
    body.insert("kind".into(), a.kind.clone().into());
    if let Some(p) = &a.parent {
        body.insert("parent_id".into(), ctx.item_id(p)?.into());
    }
    if md.is_none()
        && let Some(t) = &a.title
    {
        body.insert("title".into(), t.clone().into());
    }
    if let Some(p) = ctx.position(&a.position, false)? {
        body.insert("position".into(), p);
    }
    let client = ctx.client()?;
    let created =
        http::read_text(client.send(&Req::post("/api/v1/items").json(&Value::Object(body)))?)?;
    let Some(md) = md else {
        return print_json_text(&created);
    };
    let summary: Value = serde_json::from_str(&created)
        .map_err(|e| Failure::bad_response(format!("POST /items: not JSON: {e}")))?;
    let id = summary["id"]
        .as_str()
        .ok_or_else(|| Failure::bad_response("POST /items: no id in the response"))?
        .to_owned();

    let mut put = Req::put(format!("/api/v1/items/{id}/doc/markdown"))
        .header("If-Match", "\"d0\"")
        .raw(md, "text/markdown; charset=utf-8");
    if a.title_h1 {
        put = put.query("title", "from_h1");
    }
    let doc = match client.send(&put).and_then(http::read_text) {
        Ok(doc) => doc,
        Err(put_error) => {
            let discard = Req::delete(format!("/api/v1/items/{id}")).query("discard_empty", "true");
            if let Err(d) = client.send(&discard) {
                output::warn(
                    "discard_failed",
                    &format!("the new item {id} could not be discarded: {}", d.message()),
                    Some(json!({ "id": id, "error": d.line() })),
                );
            }
            return Err(put_error);
        }
    };

    let item = if let Some(title) = &a.title {
        let patch = Req::patch(format!("/api/v1/items/{id}")).json(&json!({ "title": title }));
        match client.send(&patch).and_then(http::read_text) {
            Ok(t) => t,
            Err(e) => {
                output::warn(
                    "item_created",
                    &format!("the item {id} was created and filled, but setting its title failed"),
                    Some(json!({ "id": id })),
                );
                return Err(e);
            }
        }
    } else if a.title_h1 {
        let detail = ctx.get_value(Req::get(format!("/api/v1/items/{id}")))?;
        detail["item"].to_string()
    } else {
        created
    };
    if output::json() {
        output::print_json(&json!({ "item": raw(item)?, "doc": raw(doc)? }))
    } else {
        let item: Value = serde_json::from_str(&item).unwrap_or(Value::Null);
        output::print_text(&human::table(&[item]))
    }
}
