//! The `quire list` commands: recent, updated, drafts, starred, archive (owner: W8)

use super::{Ctx, Res};
use crate::args::ListCmd;
use crate::http::Req;

pub fn run(ctx: &Ctx, cmd: ListCmd) -> Res {
    let (path, list) = match cmd {
        ListCmd::Recent(l) => ("/api/v1/recent", l),
        ListCmd::Updated(l) => ("/api/v1/updated", l),
        ListCmd::Drafts(l) => ("/api/v1/drafts", l),
        ListCmd::Starred(l) => ("/api/v1/starred", l),
        ListCmd::Archive(l) => ("/api/v1/archive", l),
    };
    ctx.list(Req::get(path), &list)
}
