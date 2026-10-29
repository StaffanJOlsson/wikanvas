//! Security and cache headers, HSTS behind Caddy in acme mode (owner: W2).
//!
//! [`layer`] wraps every request (inside the error envelope, so its headers are on errors too):
//! - it computes the client address and scheme once ([`ClientInfo`], §8), records the IP in the access log's span,
//!   and gives the request a [`RequestContext`];
//! - for a Wikanvas bearer request it meters the body as it is read: bytes beyond what `Auth<S>` counted from
//!   `Content-Length` are counted against the token's hourly budget as they arrive, one KiB each time the total
//!   crosses a KiB boundary (so a body costs whole KiB of its size however it is chunked), and a body that goes over
//!   the budget is cut off and answered with 429 `rate_limited` (`reason: bytes`) (D47);
//! - on the way out it adds a renewed session cookie (unless the handler set the session cookie itself, as logout
//!   and login do), `X-Content-Type-Options: nosniff` on every response,
//!   `Cache-Control: no-store` on `/api/` responses that set no cache policy of their own, and
//!   `Strict-Transport-Security: max-age=31536000` on every response to an HTTPS request when `WIKANVAS_TLS=acme`
//!   (D57: never in `internal` mode, never on plain HTTP).
//!
//! [`html`] adds the headers of §11 that every HTML response carries; the static files call it.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use futures_util::StreamExt;

use axum::response::IntoResponse;
use wikanvas_api::RateLimitReason;

use crate::config::TlsMode;
use crate::csrf::is_wikanvas_bearer;
use crate::error::ApiError;
use crate::extract::{ClientInfo, RequestContext, SESSION_COOKIE, SESSION_COOKIE_SECURE};
use crate::state::AppState;

/// HSTS (D57): one year, no `includeSubDomains`, no `preload`.
pub const HSTS: &str = "max-age=31536000";
/// The CSP of the SPA (§11).
pub const SPA_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; media-src 'self' blob:; \
     manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; \
     object-src 'none'";
/// The CSP of the Inkboard frame (§11).
pub const FRAME_CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' \
     'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'self'; object-src 'none'";
/// The CSP of an attachment download (§11).
pub const ATTACHMENT_CSP: &str = "default-src 'none'; sandbox";
/// `Permissions-Policy` on all HTML (§11).
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=()";

/// The headers every HTML response carries (§11).
pub fn html(headers: &mut HeaderMap) {
    let set = |h: &mut HeaderMap, name: &'static str, value: &'static str| {
        h.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    };
    set(headers, "x-content-type-options", "nosniff");
    set(headers, "referrer-policy", "same-origin");
    set(headers, "cross-origin-opener-policy", "same-origin");
    set(headers, "cross-origin-resource-policy", "same-origin");
    set(headers, "permissions-policy", PERMISSIONS_POLICY);
}

/// The middleware (see the module docs).
pub async fn layer(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let client = ClientInfo::from_headers(req.headers(), req.extensions(), &state.config);
    if let Some(ip) = client.ip {
        tracing::Span::current().record("client_ip", tracing::field::display(ip));
    }
    let ctx = Arc::new(RequestContext::default());
    req.extensions_mut().insert(Arc::clone(&ctx));
    req.extensions_mut().insert(client);
    if is_wikanvas_bearer(req.headers()) {
        let meter = Arc::clone(&ctx);
        let limiter = Arc::clone(&state.token_bytes_limiter);
        req = req.map(|body| {
            Body::from_stream(body.into_data_stream().map(move |chunk| {
                let bytes = chunk?;
                let read = meter
                    .body_read
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed)
                    + bytes.len() as u64;
                let charged = meter.body_charged.load(Ordering::Relaxed);
                if let Some(token) = meter.token_id.get()
                    && read > charged
                {
                    meter.body_charged.store(read, Ordering::Relaxed);
                    // The KiB boundaries this chunk crossed: the charges add up to ceil(total / 1024) KiB.
                    let kib = read.div_ceil(1024) - charged.div_ceil(1024);
                    if let Err(retry) = limiter.check(token, kib * 1024) {
                        *meter.over_budget.lock().unwrap_or_else(|p| p.into_inner()) = Some(retry);
                        return Err(axum::Error::new(std::io::Error::other(
                            "the token's request bytes per hour are used up",
                        )));
                    }
                }
                Ok(bytes)
            }))
        });
    }
    let api = req.uri().path().starts_with("/api/");

    let mut res = next.run(req).await;

    let over_budget = *ctx.over_budget.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(retry) = over_budget {
        res = ApiError::rate_limited(RateLimitReason::Bytes, retry).into_response();
    }
    let cookie = ctx
        .set_cookie
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let headers = res.headers_mut();
    if let Some(cookie) = cookie
        && !sets_session_cookie(headers)
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    headers
        .entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    if api && !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if client.https && state.config.tls == TlsMode::Acme {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
    res
}

/// Whether the response already sets (or clears) the session cookie of either scheme: then a renewal must not follow
/// it, because the browser applies `Set-Cookie` headers in order and the last one would win.
fn sets_session_cookie(headers: &HeaderMap) -> bool {
    headers.get_all(header::SET_COOKIE).iter().any(|v| {
        let name = v
            .as_bytes()
            .split(|b| *b == b'=')
            .next()
            .unwrap_or_default();
        name == SESSION_COOKIE.as_bytes() || name == SESSION_COOKIE_SECURE.as_bytes()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_in_response() {
        let mut h = HeaderMap::new();
        assert!(!sets_session_cookie(&h));
        h.append(
            header::SET_COOKIE,
            HeaderValue::from_static("other=1; Path=/"),
        );
        h.append(
            header::SET_COOKIE,
            HeaderValue::from_static("wikanvas_session_x=1; Path=/"),
        );
        assert!(!sets_session_cookie(&h));
        h.append(
            header::SET_COOKIE,
            HeaderValue::from_static("wikanvas_session=; Max-Age=0"),
        );
        assert!(sets_session_cookie(&h));
        let mut h = HeaderMap::new();
        h.append(
            header::SET_COOKIE,
            HeaderValue::from_static("__Host-wikanvas_session=abc; Secure"),
        );
        assert!(sets_session_cookie(&h));
    }
}
