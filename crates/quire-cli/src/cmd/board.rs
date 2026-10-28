//! The `quire board` commands: the native Inkboard envelope, and page previews (owner: W8)
//!
//! Boards are read and written as the envelope the server stores (§6.2, §7.5): the CLI never parses it, so unknown
//! fields survive both ways.

use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::{Ctx, Res, print_json_text};
use crate::args::BoardCmd;
use crate::error::Failure;
use crate::http::{self, Req};
use crate::output;

pub fn run(ctx: &Ctx, cmd: BoardCmd) -> Res {
    match cmd {
        BoardCmd::Get(a) => {
            let id = ctx.item_id(&a.item)?;
            let req = Req::get(format!("/api/v1/items/{id}/board")).transfer();
            let note_etag = |etag: &Option<String>| {
                if !output::json()
                    && let Some(e) = etag
                {
                    output::note(&format!("etag: {e}"));
                }
            };
            // With --out the envelope streams to the file; without it, it is read into memory once (a GET is
            // then retried when the body fails in transit).
            if let Some(path) = a.out {
                let resp = ctx.client()?.send(&req)?;
                let etag = http::header(resp.headers(), "etag");
                note_etag(&etag);
                let mut file = std::fs::File::create(&path)
                    .map_err(|e| Failure::io(&format!("creating {}", path.display()), &e))?;
                let w = http::stream_to(resp, &mut file, &format!("writing {}", path.display()))?;
                return if output::json() {
                    output::print_json(&json!({
                        "item_id": id, "format": "inkboard", "etag": etag, "path": path,
                        "bytes": w.bytes, "sha256": w.sha256,
                    }))
                } else {
                    output::note(&format!("wrote {} bytes to {}", w.bytes, path.display()));
                    Ok(())
                };
            }
            let t = ctx.client()?.send_text(&req)?;
            let etag = t.header("etag");
            note_etag(&etag);
            if !output::json() {
                return output::stdout_bytes(t.text.as_bytes());
            }
            let content = RawValue::from_string(t.text)
                .map_err(|e| Failure::bad_response(format!("GET …/board: not JSON: {e}")))?;
            output::print_json(&output::Wrapped {
                item_id: &id,
                format: "inkboard",
                etag: etag.as_deref(),
                content: &*content,
                warnings: None,
            })
        }
        BoardCmd::Put(a) => {
            let if_match = super::precondition(&a.precondition)?;
            let id = ctx.item_id(&a.item)?;
            let body = ctx.read_input(&a.file, "FILE")?;
            let req = Req::put(format!("/api/v1/items/{id}/board"))
                .raw(body, "application/json")
                .header("If-Match", if_match)
                .transfer();
            let text = ctx.client()?.send_text(&req)?.text;
            if output::json() {
                return print_json_text(&text);
            }
            let v: Value = serde_json::from_str(&text)
                .map_err(|e| Failure::bad_response(format!("PUT …/board: not JSON: {e}")))?;
            output::note(&format!("etag: {}", v["etag"].as_str().unwrap_or("-")));
            output::print_text(&format!("rev {}", v["rev"]))
        }
        BoardCmd::Preview(a) => {
            super::require_out_with_json(a.out.as_deref())?;
            let id = ctx.item_id(&a.item)?;
            let resp = ctx
                .client()?
                .send(&Req::get(format!("/api/v1/items/{id}/board/pages/{}", a.page)).transfer())?;
            super::download(resp, a.out.as_deref())
        }
    }
}
