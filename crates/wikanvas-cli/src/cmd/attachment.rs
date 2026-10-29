//! The `wikanvas attachment` commands (owner: W8)

use std::path::Path;

use super::{Ctx, Res};
use crate::args::AttachmentCmd;
use crate::http::Req;
use crate::refs;

pub fn run(ctx: &Ctx, cmd: AttachmentCmd) -> Res {
    match cmd {
        AttachmentCmd::Upload(a) => {
            let filename = a.filename.clone().or_else(|| {
                (a.file != "-")
                    .then(|| Path::new(&a.file).file_name())
                    .flatten()
                    .map(|n| n.to_string_lossy().into_owned())
            });
            let item = match &a.item {
                Some(i) => Some(ctx.item_id(i)?),
                None => None,
            };
            let body = ctx.read_input(&a.file, "FILE")?;
            let req = Req::post("/api/v1/attachments")
                .query_opt("item_id", item)
                .query_opt("filename", filename)
                .raw(body, "application/octet-stream")
                .transfer();
            ctx.call(req, None)
        }
        AttachmentCmd::Get(a) => {
            super::require_out_with_json(a.out.as_deref())?;
            let id = refs::uuid(&a.id, "attachment")?;
            let resp = ctx
                .client()?
                .send(&Req::get(format!("/api/v1/attachments/{id}")).transfer())?;
            super::download(resp, a.out.as_deref())
        }
    }
}
