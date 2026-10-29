//! The `wikanvas token` commands: personal access tokens (owner: W8)

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::{Ctx, Res, print_json_text};
use crate::args::{TokenCmd, TokenCreate};
use crate::error::Failure;
use crate::http::Req;
use crate::{config, output, refs};

pub fn run(ctx: &Ctx, cmd: TokenCmd) -> Res {
    match cmd {
        TokenCmd::List => list(ctx),
        TokenCmd::Create(a) => create(ctx, a),
        TokenCmd::Revoke(a) => {
            let id = refs::uuid(&a.id, "token")?;
            ctx.call(Req::delete(format!("/api/v1/tokens/{id}")), Some(&id))
        }
    }
}

fn list(ctx: &Ctx) -> Res {
    let text = ctx.client()?.send_text(&Req::get("/api/v1/tokens"))?.text;
    if output::json() {
        return print_json_text(&text);
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("GET /tokens: not JSON: {e}")))?;
    let mut out = String::new();
    for t in v.as_array().into_iter().flatten() {
        let scopes: Vec<&str> = t["scopes"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        out.push_str(&format!(
            "{}  {}  {:<24}  [{}]  expires {}\n",
            t["id"].as_str().unwrap_or("-"),
            t["prefix"].as_str().unwrap_or("-"),
            t["name"].as_str().unwrap_or("-"),
            scopes.join(", "),
            t["expires_at"].as_str().unwrap_or("never")
        ));
    }
    output::print_text(&out)
}

/// `--expires 30d|never` → `expires_in_days` (omitted: the server's 90 days).
pub fn expires(input: Option<&str>) -> Result<Option<Value>, Failure> {
    match input.map(str::trim) {
        None => Ok(None),
        Some("never") => Ok(Some(Value::Null)),
        Some(s) => {
            let days: u32 = s
                .strip_suffix('d')
                .and_then(|n| n.parse().ok())
                .filter(|n| (1..=wikanvas_api::tokens::MAX_EXPIRY_DAYS).contains(n))
                .ok_or_else(|| {
                    Failure::usage(format!(
                        "--expires {s:?}: use <days>d (1-3650), e.g. 30d, or never"
                    ))
                })?;
            Ok(Some(Value::from(days)))
        }
    }
}

fn create(ctx: &Ctx, a: TokenCreate) -> Res {
    let mut body = json!({ "name": a.name, "scopes": [a.scope] });
    if let Some(days) = expires(a.expires.as_deref())? {
        body["expires_in_days"] = days;
    }
    let text = ctx
        .client()?
        .send_text(&Req::post("/api/v1/tokens").json(&body))?
        .text;
    #[derive(Deserialize)]
    struct Created<'a> {
        token: String,
        #[serde(borrow)]
        api_token: &'a RawValue,
    }
    let created: Created = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("POST /tokens: unexpected response: {e}")))?;
    match a.token_out {
        Some(path) => {
            config::write_secret(&path, &created.token)?;
            if output::json() {
                #[derive(serde::Serialize)]
                struct Written<'a> {
                    token_file: &'a std::path::Path,
                    api_token: &'a RawValue,
                }
                output::print_json(&Written {
                    token_file: &path,
                    api_token: created.api_token,
                })
            } else {
                output::print_text(&format!(
                    "wrote token {}… to {} (mode 0600)",
                    wikanvas_api::token::display_prefix(&created.token),
                    path.display()
                ))
            }
        }
        None => {
            if output::json() {
                print_json_text(&text)
            } else {
                output::note("the token is shown once: store it now (e.g. in a 0600 file)");
                output::print_text(&created.token)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry() {
        assert_eq!(expires(None).unwrap(), None);
        assert_eq!(expires(Some("never")).unwrap(), Some(Value::Null));
        assert_eq!(expires(Some("30d")).unwrap(), Some(Value::from(30)));
        assert_eq!(expires(Some("3650d")).unwrap(), Some(Value::from(3650)));
        for bad in ["0d", "3651d", "30", "d", "1w", "-1d"] {
            assert!(expires(Some(bad)).is_err(), "{bad}");
        }
    }
}
