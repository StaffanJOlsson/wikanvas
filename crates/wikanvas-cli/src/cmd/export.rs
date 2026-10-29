//! The `quire export` and `quire backup` commands (owner: W8)

use super::{Ctx, Res};
use crate::args::ExportArgs;
use crate::http::Req;

/// `GET /export`: the tar, streamed to stdout or `--out`.
pub fn export(ctx: &Ctx, a: ExportArgs) -> Res {
    super::require_out_with_json(a.out.as_deref())?;
    let resp = ctx.client()?.send(&Req::get("/api/v1/export").transfer())?;
    super::download(resp, a.out.as_deref())
}

/// `POST /admin/backup`: a backup file on the server (`quire admin backup` makes a local one).
pub fn backup(ctx: &Ctx) -> Res {
    ctx.call(Req::post("/api/v1/admin/backup").transfer(), None)
}
