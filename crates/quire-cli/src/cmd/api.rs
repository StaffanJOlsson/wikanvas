//! The `quire api` command: any route, with auth, retries and error mapping, as `gh api` (owner: W8)
//!
//! §7.6 "Guards": the path must start with `/api/` and contain no scheme, no `//`, no `\` and no `.` or `..` segment
//! (also percent-encoded); the URL is the configured origin plus the path by concatenation. `--header` refuses
//! `Authorization`, `Cookie` and `Host`. A refusal exits 2 before any request. API calls use the transfer timeout
//! (300 s), since any route may be a download.

use ureq::http::Method;

use super::{Ctx, Res, print_json_text};
use crate::args::ApiArgs;
use crate::error::Failure;
use crate::http::{self, Req};
use crate::output;

fn method(input: &str) -> Result<Method, Failure> {
    match input.to_ascii_uppercase().as_str() {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        _ => Err(Failure::usage(format!(
            "api: {input:?} is not one of GET, POST, PUT, PATCH, DELETE, HEAD"
        ))),
    }
}

fn decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// The path guard (see the module docs).
pub fn check_path(input: &str) -> Result<(), Failure> {
    let refuse = |why: &str| {
        Err(Failure::usage(format!(
            "api: refused path {input:?}: {why} (a path under /api/, such as /api/v1/sidebar)"
        )))
    };
    if input
        .bytes()
        .any(|b| b.is_ascii_control() || b == b' ' || b == b'#')
    {
        return refuse("control characters, spaces and fragments are not allowed");
    }
    if input.contains('\\') {
        return refuse("no backslash");
    }
    if input.contains("://") {
        return refuse("no scheme: the URL is the configured origin");
    }
    let path = input.split('?').next().unwrap_or_default();
    if !path.starts_with("/api/") {
        return refuse("it must start with /api/");
    }
    if path.contains("//") {
        return refuse("no empty segment");
    }
    let decoded = decode(path);
    if decoded.contains(&b'\\') || decoded.windows(2).any(|w| w == b"//") {
        return refuse("no encoded slash or backslash");
    }
    for seg in path.split('/') {
        let seg = decode(seg);
        if seg == b"." || seg == b".." {
            return refuse("no . or .. segment");
        }
        if seg.contains(&b'/') {
            return refuse("no encoded slash");
        }
    }
    Ok(())
}

/// Splits and checks one `--header K:V`.
pub fn check_header(input: &str) -> Result<(String, String), Failure> {
    let (k, v) = input
        .split_once(':')
        .ok_or_else(|| Failure::usage(format!("--header {input:?}: use Name:value")))?;
    let k = k.trim();
    if k.is_empty()
        || !k
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    {
        return Err(Failure::usage(format!(
            "--header {input:?}: not a header name"
        )));
    }
    if ["authorization", "cookie", "host"].contains(&k.to_ascii_lowercase().as_str()) {
        return Err(Failure::usage(format!(
            "--header {k}: refused (the CLI sends the token itself, and never cookies or another Host)"
        )));
    }
    Ok((k.to_owned(), v.trim().to_owned()))
}

pub fn run(ctx: &Ctx, a: ApiArgs) -> Res {
    let method = method(&a.method)?;
    check_path(&a.path)?;
    let mut headers = Vec::new();
    let mut content_type = None;
    for h in &a.header {
        let (k, v) = check_header(h)?;
        if k.eq_ignore_ascii_case("content-type") {
            content_type = Some(v);
        } else {
            headers.push((k, v));
        }
    }
    let body = match &a.input {
        Some(spec) => Some(ctx.read_input(spec, "--input")?),
        None => None,
    };
    let client = ctx.client()?;
    let mut req = Req::new(method.clone(), a.path.clone()).transfer();
    req.headers = headers;
    if let Some(bytes) = body {
        req = req.raw(bytes, content_type.as_deref().unwrap_or("application/json"));
    }
    if !client.has_token() {
        req = req.public();
    }
    let resp = client.send(&req)?;
    let status = resp.status().as_u16();
    if let Some(out) = a.out.as_deref() {
        return super::download(resp, Some(out));
    }
    let rid = http::header(resp.headers(), quire_api::common::header::REQUEST_ID);
    let ct = http::header(resp.headers(), "content-type").unwrap_or_default();
    let is_json = {
        let mime = ct
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        mime == "application/json" || mime.ends_with("+json")
    };
    if status == 204 || method == Method::HEAD {
        return output::print_ok(Some(status), rid.as_deref(), None);
    }
    if is_json {
        let text = http::read_text(resp)?;
        if text.trim().is_empty() {
            return output::print_ok(Some(status), rid.as_deref(), None);
        }
        return print_json_text(&text);
    }
    super::require_out_with_json(None)?;
    super::download(resp, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_guard() {
        for ok in [
            "/api/v1/sidebar",
            "/api/v1/search?q=a%20b&limit=5",
            "/api/v1/items/x.y",
            "/api/v1/nope",
        ] {
            assert!(check_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "//example.invalid/x",
            "https://host/x",
            "/api/../x",
            "/api/%2e%2e/x",
            "/api/%2E/x",
            "/api/./x",
            "/api/v1//x",
            "/api/v1/a%2fb",
            "/api\\v1",
            "/healthz",
            "api/v1/x",
            "/api/v1/x#frag",
            "/api/v1/x y",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn header_guard() {
        assert_eq!(
            check_header("X-Test: 1").unwrap(),
            ("X-Test".into(), "1".into())
        );
        for bad in [
            "Authorization: Bearer x",
            "cookie:a=b",
            "HOST:x",
            "nocolon",
            ": x",
            "a b: c",
        ] {
            assert!(check_header(bad).is_err(), "{bad}");
        }
    }
}
