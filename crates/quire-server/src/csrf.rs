//! The Host allowlist and the Fetch-Metadata / Origin CSRF checks (owner: W2).
//!
//! One middleware, [`layer`], applied to every route except `/healthz` (§8, D16, D43):
//! - `Host` must be the public URL's host[:port], `QUIRE_DOMAIN` with any port, an entry of `QUIRE_ALLOWED_HOSTS`, or
//!   loopback (`localhost`, `127.0.0.1`, `[::1]`) with any port; otherwise 421 `host_not_allowed`. It applies to
//!   bearer requests too. Quire reads `Host` itself, never `X-Forwarded-Host`.
//! - POST, PUT, PATCH and DELETE that are not Quire bearer requests ([`is_quire_bearer`]) must carry
//!   `Sec-Fetch-Site: same-origin | none`, or an `Origin` whose host[:port] is the public URL's or the request's
//!   `Host`; otherwise 403 `csrf_rejected`, also when neither header is present.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use quire_api::ErrorCode;

use crate::config::{Config, Scheme, split_host_port};
use crate::error::ApiError;
use crate::state::AppState;

/// Whether the request carries a Quire token: `Authorization` with scheme `Bearer` (any case) and credentials that
/// start with `qpat_`. Such a request is authenticated by the token alone (a bad token is 401, never a cookie
/// fallback) and skips the CSRF checks. Any other `Authorization` value is ignored (D43).
pub fn is_quire_bearer(headers: &HeaderMap) -> bool {
    bearer_token(headers).is_some()
}

/// The credentials of a Quire bearer request (see [`is_quire_bearer`]).
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, credentials) = value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let credentials = credentials.trim();
    credentials
        .starts_with(quire_api::token::PREFIX)
        .then_some(credentials)
}

/// The request's `Host` (HTTP/1.1 header, or the URI authority of an HTTP/2 request), split and lowercased.
pub fn request_host(
    req_headers: &HeaderMap,
    uri: &axum::http::Uri,
) -> Option<(String, Option<u16>)> {
    let raw = match req_headers.get(header::HOST) {
        Some(v) => v.to_str().ok()?.to_owned(),
        None => uri.authority()?.as_str().to_owned(),
    };
    split_host_port(raw.trim())
}

/// Whether `host` / `port` may address this server (§8).
pub fn host_allowed(config: &Config, host: &str, port: Option<u16>) -> bool {
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    loopback
        || config.public_url.host == host
            && port.unwrap_or(config.public_url.scheme.default_port())
                == config.public_url.port_or_default()
        || config.domain.as_deref() == Some(host)
        || config.allowed_hosts.iter().any(|p| p.matches(host, port))
}

/// Whether a non-bearer mutation passes the Fetch-Metadata / Origin rule.
pub fn origin_allowed(config: &Config, headers: &HeaderMap, host: &(String, Option<u16>)) -> bool {
    // Every Sec-Fetch-Site value must be same-origin or none (a browser sends one; a forged second never helps).
    let mut sites = headers.get_all("sec-fetch-site").iter().peekable();
    if sites.peek().is_some()
        && sites.all(|v| {
            v.to_str().is_ok_and(|site| {
                site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none")
            })
        })
    {
        return true;
    }
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (scheme, authority) = if let Some(rest) = origin.strip_prefix("https://") {
        (Scheme::Https, rest)
    } else if let Some(rest) = origin.strip_prefix("http://") {
        (Scheme::Http, rest)
    } else {
        return false; // "null" and anything else
    };
    let Some((o_host, o_port)) = split_host_port(authority) else {
        return false;
    };
    let o_port = o_port.unwrap_or(scheme.default_port());
    // The scheme is not compared (§8): host[:port] already pins the origin to Quire. Default ports are normalized
    // with the origin's own scheme, and the request's Host with the public URL's.
    let public = &config.public_url;
    if public.host == o_host && public.port_or_default() == o_port {
        return true;
    }
    let (r_host, r_port) = host;
    *r_host == o_host && r_port.unwrap_or(scheme.default_port()) == o_port
}

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// The middleware (see the module docs). `/healthz` passes untouched.
pub async fn layer(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let config = &state.config;
    let Some(host) = request_host(req.headers(), req.uri()) else {
        return ApiError::new(ErrorCode::HostNotAllowed, "the request has no valid Host")
            .into_response();
    };
    if !host_allowed(config, &host.0, host.1) {
        return ApiError::new(
            ErrorCode::HostNotAllowed,
            format!(
                "Host {} is not allowed; use {} or add it to QUIRE_ALLOWED_HOSTS",
                host.0, config.public_url
            ),
        )
        .into_response();
    }
    if is_mutation(req.method())
        && !is_quire_bearer(req.headers())
        && !origin_allowed(config, req.headers(), &host)
    {
        return ApiError::new(
            ErrorCode::CsrfRejected,
            "cross-site request refused: send Sec-Fetch-Site: same-origin, a same-origin Origin, or use a token",
        )
        .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_static(v));
        }
        h
    }

    fn config(vars: &[(&str, &str)]) -> Config {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
    }

    #[test]
    fn quire_bearer_predicate() {
        let token = "Bearer qpat_abc";
        assert!(is_quire_bearer(&headers(&[("authorization", token)])));
        assert!(is_quire_bearer(&headers(&[(
            "authorization",
            "bearer  qpat_x"
        )])));
        assert!(!is_quire_bearer(&headers(&[(
            "authorization",
            "Bearer eyJhbGciOi.x.y"
        )])));
        assert!(!is_quire_bearer(&headers(&[(
            "authorization",
            "Basic cXBhdF94Og=="
        )])));
        assert!(!is_quire_bearer(&headers(&[("authorization", "qpat_abc")])));
        assert!(!is_quire_bearer(&headers(&[])));
        assert_eq!(
            bearer_token(&headers(&[("authorization", token)])),
            Some("qpat_abc")
        );
    }

    #[test]
    fn host_allowlist() {
        let c = config(&[
            ("QUIRE_DOMAIN", "quire.lan"),
            ("QUIRE_PUBLIC_URL", "https://wiki.example.org"),
            ("QUIRE_ALLOWED_HOSTS", "quire"),
        ]);
        let ok = |h: &str, p: Option<u16>| host_allowed(&c, h, p);
        assert!(ok("wiki.example.org", None) && ok("wiki.example.org", Some(443)));
        assert!(!ok("wiki.example.org", Some(8443)));
        assert!(ok("quire.lan", None) && ok("quire.lan", Some(8443)));
        assert!(ok("quire", Some(8080)));
        assert!(ok("localhost", Some(5173)) && ok("127.0.0.1", None) && ok("[::1]", Some(1)));
        assert!(!ok("evil.lan", None) && !ok("quire.lan.evil", None));
    }

    #[test]
    fn origin_rule() {
        let c = config(&[("QUIRE_PUBLIC_URL", "https://wiki.example.org")]);
        let host = ("localhost".to_owned(), Some(8080));
        let ok =
            |pairs: &[(&'static str, &'static str)]| origin_allowed(&c, &headers(pairs), &host);
        assert!(ok(&[("sec-fetch-site", "same-origin")]));
        assert!(ok(&[("sec-fetch-site", "none")]));
        assert!(!ok(&[("sec-fetch-site", "cross-site")]));
        assert!(!ok(&[("sec-fetch-site", "same-site")]));
        assert!(!ok(&[]));
        assert!(ok(&[("origin", "https://wiki.example.org")]));
        assert!(ok(&[("origin", "http://wiki.example.org:443")]));
        assert!(ok(&[("origin", "http://localhost:8080")]));
        assert!(!ok(&[("origin", "http://localhost:8081")]));
        assert!(!ok(&[("origin", "https://evil.example")]));
        assert!(!ok(&[("origin", "null")]));
        assert!(!ok(&[
            ("sec-fetch-site", "cross-site"),
            ("origin", "https://evil.example")
        ]));
        assert!(!ok(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "cross-site")
        ]));
    }
}
