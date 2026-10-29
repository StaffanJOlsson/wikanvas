//! The `quire workspace` and `quire sidebar` commands (owner: W8)

use serde_json::{Map, Value};

use super::{Ctx, Res};
use crate::args::WorkspaceCmd;
use crate::error::Failure;
use crate::http::Req;
use crate::refs;

pub fn run(ctx: &Ctx, cmd: WorkspaceCmd) -> Res {
    match cmd {
        WorkspaceCmd::Get => ctx.call(Req::get("/api/v1/workspace"), None),
        WorkspaceCmd::Set(a) => {
            let mut body = Map::new();
            if let Some(name) = a.name {
                body.insert("name".into(), name.into());
            }
            if let Some(logo) = a.logo {
                body.insert(
                    "logo_attachment_id".into(),
                    refs::uuid(&logo, "attachment")?.into(),
                );
            } else if a.no_logo {
                body.insert("logo_attachment_id".into(), Value::Null);
            }
            if body.is_empty() {
                return Err(Failure::usage(
                    "workspace set: pass --name, --logo or --no-logo",
                ));
            }
            ctx.call(
                Req::patch("/api/v1/workspace").json(&Value::Object(body)),
                None,
            )
        }
    }
}

pub fn sidebar(ctx: &Ctx) -> Res {
    ctx.call(Req::get("/api/v1/sidebar"), None)
}
