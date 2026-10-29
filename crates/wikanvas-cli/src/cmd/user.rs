//! The `wikanvas user` and `wikanvas invite` commands: accounts and invite links (D148)
//!
//! `user` takes an account's id or its email (looked up in `GET /users`, ignoring ASCII case, as the server compares
//! emails). `invite create` prints the link once, like `token create` prints a token: `--json` gives the API's
//! `{invite, url}`, and `--url-out F` writes the link to a 0600 file instead of stdout.

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use wikanvas_api::cli::ExitCode;

use super::{Ctx, Res, print_json_text};
use crate::args::{InviteCmd, InviteCreate, UserCmd};
use crate::error::Failure;
use crate::http::Req;
use crate::{config, output, refs};

pub fn run(ctx: &Ctx, cmd: UserCmd) -> Res {
    match cmd {
        UserCmd::List => list(ctx),
        UserCmd::SetRole(a) => patch(ctx, &a.user, json!({ "role": a.role })),
        UserCmd::Disable(a) => patch(ctx, &a.user, json!({ "disabled": true })),
        UserCmd::Enable(a) => patch(ctx, &a.user, json!({ "disabled": false })),
    }
}

fn list(ctx: &Ctx) -> Res {
    let text = ctx.client()?.send_text(&Req::get("/api/v1/users"))?.text;
    if output::json() {
        return print_json_text(&text);
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("GET /users: not JSON: {e}")))?;
    let mut out = String::new();
    for u in v.as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{}  {:<6}  {:<32}  {}{}\n",
            u["id"].as_str().unwrap_or("-"),
            u["role"].as_str().unwrap_or("-"),
            u["email"].as_str().unwrap_or("-"),
            u["name"].as_str().unwrap_or("-"),
            if u["disabled_at"].is_string() {
                "  (disabled)"
            } else {
                ""
            }
        ));
    }
    output::print_text(&out)
}

/// The id of `input`: a UUID as it is, an email through `GET /users` (exit 7 when no account has it).
fn user_id(ctx: &Ctx, input: &str) -> Result<String, Failure> {
    let input = input.trim();
    if !input.contains('@') {
        return refs::uuid(input, "user");
    }
    let users = ctx.get_value(Req::get("/api/v1/users"))?;
    users
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| {
            u["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(input))
        })
        .and_then(|u| u["id"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            Failure::with_exit(
                ExitCode::NotFound,
                None,
                "not_found",
                format!("no account has the email {input:?}"),
                None,
                None,
            )
        })
}

fn patch(ctx: &Ctx, user: &str, body: Value) -> Res {
    let id = user_id(ctx, user)?;
    ctx.call(Req::patch(format!("/api/v1/users/{id}")).json(&body), None)
}

pub fn invite(ctx: &Ctx, cmd: InviteCmd) -> Res {
    match cmd {
        InviteCmd::Create(a) => create(ctx, a),
        InviteCmd::List => list_invites(ctx),
        InviteCmd::Revoke(a) => {
            let id = refs::uuid(&a.id, "invite")?;
            ctx.call(Req::delete(format!("/api/v1/invites/{id}")), Some(&id))
        }
    }
}

/// `--expires 7d` → `expires_in_days` (omitted: the server's 7 days).
pub fn invite_days(input: Option<&str>) -> Result<Option<u32>, Failure> {
    let Some(s) = input.map(str::trim) else {
        return Ok(None);
    };
    s.strip_suffix('d')
        .and_then(|n| n.parse().ok())
        .filter(|n| (1..=wikanvas_api::MAX_INVITE_DAYS).contains(n))
        .map(Some)
        .ok_or_else(|| Failure::usage(format!("--expires {s:?}: use <days>d (1-30), e.g. 7d")))
}

fn create(ctx: &Ctx, a: InviteCreate) -> Res {
    let mut body = json!({ "role": a.role });
    if let Some(days) = invite_days(a.expires.as_deref())? {
        body["expires_in_days"] = days.into();
    }
    if let Some(note) = a.note {
        body["note"] = note.into();
    }
    let text = ctx
        .client()?
        .send_text(&Req::post("/api/v1/invites").json(&body))?
        .text;
    #[derive(Deserialize)]
    struct Created<'a> {
        #[serde(borrow)]
        invite: &'a RawValue,
        url: String,
    }
    let created: Created = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("POST /invites: unexpected response: {e}")))?;
    match a.url_out {
        Some(path) => {
            config::write_secret(&path, &created.url)?;
            if output::json() {
                #[derive(serde::Serialize)]
                struct Written<'a> {
                    url_file: &'a std::path::Path,
                    invite: &'a RawValue,
                }
                output::print_json(&Written {
                    url_file: &path,
                    invite: created.invite,
                })
            } else {
                output::print_text(&format!(
                    "wrote the invite link to {} (mode 0600)",
                    path.display()
                ))
            }
        }
        None if output::json() => print_json_text(&text),
        None => {
            output::note("the link is shown once and works once: send it to the person you invite");
            output::print_text(&created.url)
        }
    }
}

fn list_invites(ctx: &Ctx) -> Res {
    let text = ctx.client()?.send_text(&Req::get("/api/v1/invites"))?.text;
    if output::json() {
        return print_json_text(&text);
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("GET /invites: not JSON: {e}")))?;
    let mut out = String::new();
    for i in v.as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{}  {:<6}  expires {}  {}\n",
            i["id"].as_str().unwrap_or("-"),
            i["role"].as_str().unwrap_or("-"),
            i["expires_at"].as_str().unwrap_or("-"),
            i["note"].as_str().unwrap_or("")
        ));
    }
    output::print_text(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry() {
        assert_eq!(invite_days(None).unwrap(), None);
        assert_eq!(invite_days(Some("7d")).unwrap(), Some(7));
        assert_eq!(invite_days(Some("30d")).unwrap(), Some(30));
        for bad in ["0d", "31d", "7", "never", "1w"] {
            assert!(invite_days(Some(bad)).is_err(), "{bad}");
        }
    }
}
