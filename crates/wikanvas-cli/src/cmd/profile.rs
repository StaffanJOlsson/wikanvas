//! The `wikanvas config show` and `wikanvas config path` commands: local, no request (owner: W8)

use serde_json::json;

use super::{Ctx, Res};
use crate::args::ConfigCmd;
use crate::{config, output};

pub fn run(ctx: &Ctx, cmd: ConfigCmd) -> Res {
    match cmd {
        ConfigCmd::Path => {
            let path = config::config_path(&ctx.env)?;
            if output::json() {
                output::print_json(&json!({ "path": path }))
            } else {
                output::print_text(&path.display().to_string())
            }
        }
        ConfigCmd::Show => {
            let r = ctx.resolved()?;
            let url = r.url.as_ref().map(config::Origin::base);
            let prefix = r
                .token
                .as_deref()
                .map(|t| wikanvas_api::token::display_prefix(t).to_owned());
            if output::json() {
                output::print_json(&json!({
                    "profile": r.profile,
                    "url": url,
                    "url_source": r.url_source,
                    "token_source": r.token_source,
                    "token_prefix": prefix,
                }))
            } else {
                let or = |v: Option<&str>| v.unwrap_or("-").to_owned();
                output::print_text(&format!(
                    "profile file: {}\nprofile: {}\nurl: {} (from {})\ntoken: {} (from {})\n",
                    r.path.display(),
                    or(r.profile.as_deref()),
                    or(url.as_deref()),
                    or(r.url_source),
                    prefix
                        .map(|p| format!("{p}…"))
                        .unwrap_or_else(|| "-".into()),
                    or(r.token_source)
                ))
            }
        }
    }
}
