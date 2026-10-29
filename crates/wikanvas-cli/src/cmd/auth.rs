//! The `wikanvas auth`, `wikanvas me` and `wikanvas session` commands (owner: W8)

use serde_json::{Map, Value, json};

use super::{Ctx, Res, print_json_text, print_response};
use crate::args::{AuthCmd, GlobalArgs, MeCmd, SessionCmd};
use crate::config::{self, Origin, Resolved, StoredToken};
use crate::error::Failure;
use crate::http::{self, Client, Req};
use crate::{output, refs};

pub fn run(ctx: &Ctx, cmd: AuthCmd) -> Res {
    match cmd {
        AuthCmd::Login => login(ctx),
        AuthCmd::Logout(a) => logout(ctx, a.revoke),
        AuthCmd::Whoami => whoami(ctx),
    }
}

/// `auth login --url U (--token-stdin | --token-file F) [--profile P]`: `GET /auth/me` with exactly that pair, then
/// profile P (default `default`) gets `url` and `token` (stdin) or `token_file` (the absolute path of F). The file
/// gets `default_profile = P` when it has none.
fn login(ctx: &Ctx) -> Res {
    let g = &ctx.g;
    let url = g
        .url
        .as_deref()
        .ok_or_else(|| Failure::usage("auth login needs --url URL"))?;
    if !g.token_stdin && g.token_file.is_none() {
        return Err(Failure::usage(
            "auth login needs --token-stdin or --token-file FILE (a token is never an argument)",
        ));
    }
    let origin = Origin::parse(url).map_err(|e| Failure::config(format!("--url: {e}")))?;
    let (token, stored, source) = if g.token_stdin {
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(std::io::stdin().lock(), 64 * 1024),
            &mut text,
        )
        .map_err(|e| Failure::config(format!("cannot read the token from stdin: {e}")))?;
        let t = text.trim().to_owned();
        if t.is_empty() {
            return Err(Failure::config("--token-stdin: no token on stdin"));
        }
        (t.clone(), StoredToken::Inline(t), "--token-stdin")
    } else {
        let f = g.token_file.as_deref().unwrap_or(std::path::Path::new(""));
        let t = config::read_token_file(f, "--token-file")?;
        let abs = std::path::absolute(f).map_err(|e| Failure::io("resolving --token-file", &e))?;
        (t, StoredToken::File(abs), "--token-file")
    };
    config::check_token(&token, source)?;
    let name = g.profile.clone().unwrap_or_else(|| "default".into());
    let resolved = Resolved {
        path: config::config_path(&ctx.env)?,
        profile: Some(name.clone()),
        url: Some(origin.clone()),
        url_source: Some("--url"),
        token: Some(token),
        token_source: Some(source),
        ca_file: g
            .ca_file
            .clone()
            .or_else(|| ctx.env.get("WIKANVAS_CA_FILE").map(Into::into)),
        allow_http: matches!(ctx.env.get("WIKANVAS_ALLOW_HTTP"), Some("1" | "true")),
    };
    let client = Client::new(&resolved, &ctx.env, g.retries, g.timeout)?;
    let text = client.send_text(&Req::get("/api/v1/auth/me"))?.text;
    config::write_profile(&resolved.path, &name, &origin, &stored)?;
    output::note(&format!(
        "saved profile {name:?} ({}) in {}",
        origin.base(),
        resolved.path.display()
    ));
    if output::json() {
        print_json_text(&text)
    } else {
        print_me(&text)
    }
}

/// `auth logout [--revoke]`: removes the profile's `token` and `token_file`. `--revoke` first revokes that same token
/// with `DELETE /tokens/current`, sent to the profile's own URL ([`profile_pair`]): a token from `--token-file`,
/// `--token-stdin`, `WIKANVAS_TOKEN` or `WIKANVAS_TOKEN_FILE` is never the one revoked, since it is not the one
/// removed. Prints the ok object, plus the profile.
fn logout(ctx: &Ctx, revoke: bool) -> Res {
    let resolved = ctx.resolved()?;
    let profile = ctx.g.profile.clone().or(resolved.profile.clone());
    let (mut status, mut request_id) = (None, None);
    if revoke {
        let pair = profile_pair(ctx, &resolved, profile.as_deref())?;
        let client = Client::new(&pair, &ctx.env, ctx.g.retries, ctx.g.timeout)?;
        let resp = client.send(&Req::delete("/api/v1/tokens/current"))?;
        status = Some(resp.status().as_u16());
        request_id = http::header(resp.headers(), wikanvas_api::common::header::REQUEST_ID);
    }
    let removed = match &profile {
        Some(p) => config::remove_token(&resolved.path, p)?,
        None => false,
    };
    if output::json() {
        let mut v = output::ok_object(status, request_id.as_deref(), None);
        v["profile"] = json!(profile);
        v["removed"] = json!(removed);
        output::print_json(&v)
    } else {
        let mut parts = Vec::new();
        if revoke {
            parts.push("token revoked".to_owned());
        }
        match (&profile, removed) {
            (Some(p), true) => parts.push(format!("token removed from profile {p:?}")),
            (Some(p), false) => parts.push(format!("profile {p:?} held no token")),
            (None, _) => parts.push("no profile in use".into()),
        }
        output::print_text(&parts.join("; "))
    }
}

/// The URL and token of the profile in play, for `auth logout --revoke`: the resolved pair when its token is the
/// profile's, else the profile resolved on its own (as `--profile P` would, without token flags or `--url`). No
/// profile, or a profile without a URL and token, is exit 3 before any request.
fn profile_pair(
    ctx: &Ctx,
    resolved: &Resolved,
    profile: Option<&str>,
) -> Result<Resolved, Failure> {
    if resolved.token_source == Some("profile") {
        return Ok(resolved.clone());
    }
    let Some(name) = profile else {
        return Err(Failure::config(
            "auth logout --revoke: no profile is in use, so there is no profile token to revoke (revoke any token \
             with `wikanvas token revoke ID`)",
        ));
    };
    let g = GlobalArgs {
        profile: Some(name.to_owned()),
        url: None,
        token_file: None,
        token_stdin: false,
        ..ctx.g.clone()
    };
    let own = config::resolve(&g, &ctx.env, || Ok(String::new()))?;
    if own.token_source != Some("profile") {
        return Err(Failure::config(format!(
            "auth logout --revoke: profile {name:?} has no url and token to revoke"
        )));
    }
    Ok(own)
}

fn whoami(ctx: &Ctx) -> Res {
    let t = ctx.client()?.send_text(&Req::get("/api/v1/auth/me"))?;
    if output::json() {
        return print_response(&t, None);
    }
    print_me(&t.text)
}

fn print_me(text: &str) -> Res {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| Failure::bad_response(format!("GET /auth/me: not JSON: {e}")))?;
    let s = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_owned()
    };
    let mut out = format!(
        "{} <{}> ({}) in {:?}\n",
        s("/user/name"),
        s("/user/email"),
        s("/user/role"),
        s("/workspace/name")
    );
    match v.pointer("/auth/token").filter(|t| t.is_object()) {
        Some(t) => {
            let scopes: Vec<&str> = t["scopes"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            out.push_str(&format!(
                "token {} {:?} [{}], expires {}\n",
                t["prefix"].as_str().unwrap_or("-"),
                t["name"].as_str().unwrap_or("-"),
                scopes.join(", "),
                t["expires_at"].as_str().unwrap_or("never")
            ));
        }
        None => out.push_str(&format!("signed in with a {}\n", s("/auth/method"))),
    }
    output::print_text(&out)
}

pub fn me(ctx: &Ctx, cmd: MeCmd) -> Res {
    let MeCmd::Update(a) = cmd;
    let mut body = Map::new();
    if let Some(n) = a.name {
        body.insert("name".into(), n.into());
    }
    if let Some(e) = a.email {
        body.insert("email".into(), e.into());
    }
    if let Some(t) = a.theme {
        body.insert("theme".into(), t.into());
    }
    if body.is_empty() {
        return Err(Failure::usage("me update: pass --name, --email or --theme"));
    }
    ctx.call(
        Req::patch("/api/v1/auth/me").json(&Value::Object(body)),
        None,
    )
}

pub fn session(ctx: &Ctx, cmd: SessionCmd) -> Res {
    match cmd {
        SessionCmd::List => ctx.call(Req::get("/api/v1/auth/sessions"), None),
        SessionCmd::Revoke(a) => match a.id {
            Some(id) => {
                let id = refs::uuid(&id, "session")?;
                ctx.call(
                    Req::delete(format!("/api/v1/auth/sessions/{id}")),
                    Some(&id),
                )
            }
            None => ctx.call(
                Req::delete("/api/v1/auth/sessions").query("others", "true"),
                None,
            ),
        },
    }
}
