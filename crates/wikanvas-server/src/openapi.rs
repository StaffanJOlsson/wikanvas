//! OpenAPI 3.1 spec assembly and GET /api/v1/openapi.json (owner: W2)
//!
//! §7.4, D41: every route module returns an `OpenApiRouter<AppState>` whose handlers are registered only through
//! `routes!` on `#[utoipa::path]` functions. [`router`] merges them and nests them under `/api/v1`; `app()` and
//! [`spec`] both call it and `split_for_parts()`, so the served routes and the spec come from the same
//! registrations. utoipa's maps are ordered, so the JSON does not depend on registration order; it is built once
//! at startup ([`spec_json`]) and served from memory with `Cache-Control: no-cache` and an ETag.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::security::{
    ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
};
use utoipa::openapi::{Components, Info, License, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::routes;
use crate::state::AppState;

/// Every API route of every module, nested under `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    let v1 = OpenApiRouter::new()
        .merge(routes::auth::router())
        .merge(routes::setup::router())
        .merge(routes::workspace::router())
        .merge(routes::tokens::router())
        .merge(routes::files::router())
        .merge(routes::export::router())
        .merge(routes::config::router())
        .merge(routes::admin::router())
        .merge(routes::collections::router())
        .merge(routes::items::router())
        .merge(routes::tree::router())
        .merge(routes::lists::router())
        .merge(routes::search::router())
        .merge(routes::docs::router())
        .merge(routes::boards::router())
        .merge(OpenApiRouter::new().routes(routes!(openapi_json)));
    OpenApiRouter::with_openapi(base()).nest("/api/v1", v1)
}

/// `info`, the security schemes and the 3.1 version; the paths come from the routers.
fn base() -> OpenApi {
    let mut info = Info::new("Wikanvas API", wikanvas_api::API_VERSION);
    info.description = Some(
        "The public API of Wikanvas, a self-hosted wiki. The web client uses it with its session cookie; external \
         clients and the `wikanvas` CLI use personal access tokens (`Authorization: Bearer qpat_…`). Errors have the \
         shape `{\"error\": {\"code\", \"message\", \"details\"?}, \"request_id\"}`; clients treat an unknown \
         `code` by its HTTP status. Within v1, changes are additive only."
            .into(),
    );
    let mut license = License::new("MIT");
    license.identifier = Some("MIT".into());
    info.license = Some(license);
    let bearer = SecurityScheme::Http(
        HttpBuilder::new()
            .scheme(HttpAuthScheme::Bearer)
            .bearer_format("qpat")
            .description(Some(
                "A personal access token, `qpat_` + 46 base62 characters, with the scopes read ⊂ write ⊂ admin.",
            ))
            .build(),
    );
    let session = SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
        "wikanvas_session",
        "The web client's session cookie. Over HTTPS (behind the proxy) it is named `__Host-wikanvas_session`. A \
         session has every scope; cookie-authenticated mutations must pass the Fetch-Metadata / Origin check.",
    )));
    let mut components = Components::new();
    components.add_security_scheme("bearer", bearer);
    components.add_security_scheme("session", session);
    let mut api = OpenApi::new(info, utoipa::openapi::Paths::new());
    api.components = Some(components);
    api
}

/// The OpenAPI document of every `/api/v1` route.
///
/// A public operation is annotated `security(())`, which utoipa writes as `[{}]`; §7.4 spells it `[]`, so it is
/// rewritten here. An operation without any `security` would inherit none and look public by accident:
/// `every_operation_declares_security` catches it.
pub fn spec() -> OpenApi {
    let mut api = router().split_for_parts().1;
    let public = vec![SecurityRequirement::default()];
    for item in api.paths.paths.values_mut() {
        for op in operations(item) {
            if op.security.as_ref() == Some(&public) {
                op.security = Some(Vec::new());
            }
        }
    }
    api
}

/// Every operation of a path item.
fn operations(item: &mut PathItem) -> impl Iterator<Item = &mut Operation> {
    [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
    ]
    .into_iter()
    .filter_map(Option::as_mut)
}

/// [`spec`] as served and committed: pretty-printed JSON with a trailing newline.
pub fn spec_json() -> String {
    let mut json = spec()
        .to_pretty_json()
        .expect("the OpenAPI document serializes");
    json.push('\n');
    json
}

/// The ETag of a served spec: `"o<first 16 hex digits of its sha256>"`.
pub fn etag(json: &str) -> String {
    let digest = Sha256::digest(json.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"o{hex}\"")
}

/// Whether `If-None-Match` lists `etag` (or is `*`); a `W/` prefix is ignored.
pub fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == etag || t == "*")
}

/// The OpenAPI 3.1 document of this API.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "meta",
    operation_id = "get_openapi",
    security(()),
    params(("If-None-Match" = Option<String>, Header, description = "A previous ETag: 304 when unchanged")),
    responses(
        (status = 200, description = "The spec (`Cache-Control: no-cache`)", content_type = "application/json",
            headers(("ETag" = String, description = "The spec's version"))),
        (status = 304, description = "Unchanged since the ETag in `If-None-Match`"),
    )
)]
async fn openapi_json(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let tag = etag(&state.openapi);
    let mut res = if if_none_match(&headers, &tag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "application/json")],
            state.openapi.to_string(),
        )
            .into_response()
    };
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(v) = HeaderValue::from_str(&tag) {
        h.insert(header::ETAG, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_is_3_1_with_info_and_schemes() {
        let json: serde_json::Value = serde_json::from_str(&spec_json()).unwrap();
        assert_eq!(json["openapi"], "3.1.0");
        assert_eq!(json["info"]["title"], "Wikanvas API");
        assert_eq!(json["info"]["version"], wikanvas_api::API_VERSION);
        assert_eq!(json["info"]["license"]["name"], "MIT");
        let schemes = &json["components"]["securitySchemes"];
        assert_eq!(schemes["bearer"]["type"], "http");
        assert_eq!(schemes["bearer"]["scheme"], "bearer");
        assert_eq!(schemes["bearer"]["bearerFormat"], "qpat");
        assert_eq!(schemes["session"]["type"], "apiKey");
        assert_eq!(schemes["session"]["in"], "cookie");
        assert_eq!(schemes["session"]["name"], "wikanvas_session");
        let paths = json["paths"].as_object().unwrap();
        assert!(
            paths.keys().all(|p| p.starts_with("/api/v1/")),
            "{:?}",
            paths.keys()
        );
        assert!(paths.contains_key("/api/v1/openapi.json"));
    }

    #[test]
    fn every_operation_declares_security() {
        let mut api = spec();
        for (path, item) in api.paths.paths.iter_mut() {
            for op in operations(item) {
                let id = op.operation_id.clone().unwrap_or_default();
                assert!(op.security.is_some(), "{path} {id} declares no security");
                assert!(
                    op.operation_id.is_some(),
                    "{path} has an operation without operation_id"
                );
            }
        }
        let json: serde_json::Value = serde_json::to_value(spec()).unwrap();
        assert_eq!(
            json["paths"]["/api/v1/openapi.json"]["get"]["security"],
            serde_json::json!([])
        );
    }

    #[test]
    fn spec_is_deterministic() {
        assert_eq!(spec_json(), spec_json());
    }

    #[test]
    fn etags() {
        let tag = etag("{}");
        assert!(tag.starts_with("\"o") && tag.len() == 19, "{tag}");
        let mut h = HeaderMap::new();
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("W/{tag}")).unwrap(),
        );
        assert!(if_none_match(&h, &tag));
        assert!(!if_none_match(&h, "\"other\""));
    }
}
