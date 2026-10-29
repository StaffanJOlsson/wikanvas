//! CORS for bearer clients from WIKANVAS_CORS_ORIGINS, off by default (owner: W2).
//!
//! D48, §8: without `WIKANVAS_CORS_ORIGINS` no CORS header is ever sent. With it (exact `scheme://host[:port]` origins;
//! `*` or a malformed origin stops startup in [`crate::config`]), a tower-http `CorsLayer` on the `/api/v1` routes
//! answers preflights from those origins: methods GET, POST, PUT, PATCH and DELETE; request headers `Authorization`,
//! `Content-Type`, `If-Match`, `If-None-Match` and `X-Wikanvas-Engine`; exposed `ETag`, `Retry-After`, `X-Request-Id`,
//! `Wikanvas-Api-Version` and `Wikanvas-Conversion-Warnings`; `max-age` 600; never `Access-Control-Allow-Credentials`.
//! An allowed origin is not trusted by the CSRF check, so cross-origin cookie requests stay rejected.

use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, Method, header};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::Config;

/// The preflight cache time (§8).
pub const MAX_AGE: Duration = Duration::from_secs(600);

/// The layer for the configured origins, or `None` when CORS is off.
pub fn layer(config: &Config) -> Option<CorsLayer> {
    if config.cors_origins.is_empty() {
        return None;
    }
    let origins: Vec<HeaderValue> = config
        .cors_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(&o.to_string()).ok())
        .collect();
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                header::IF_MATCH,
                header::IF_NONE_MATCH,
                HeaderName::from_static("x-wikanvas-engine"),
            ])
            .expose_headers([
                header::ETAG,
                header::RETRY_AFTER,
                HeaderName::from_static("x-request-id"),
                HeaderName::from_static("wikanvas-api-version"),
                HeaderName::from_static("wikanvas-conversion-warnings"),
            ])
            .max_age(MAX_AGE)
            .allow_credentials(false),
    )
}
