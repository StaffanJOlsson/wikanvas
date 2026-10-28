//! Scope enforcement over the generated spec, Host allowlist, CSRF, CORS and limits (owner: W2)
//!
//! W0b: both branches of `Auth<S>` (the cookie and a Quire bearer token, D43), `session_required` and `forbidden`
//! for `Auth<Session>` / `Auth<AnyToken>`, the CSRF rule for cookie mutations, the Host allowlist, and the error
//! envelope for axum's own rejections.

use axum::Router;
use axum::routing::get;
use quire_api::{ErrorCode, Me, Scope};
use quire_server::extract::{AnyToken, Auth, IdPath, ItemRef, Session, Write};
use serde_json::json;

use crate::common::{TestApp, assert_error};

#[tokio::test]
async fn cookie_session_authenticates() {
    let app = TestApp::with_owner().await;
    let res = app.server.get("/api/v1/auth/me").await;
    res.assert_status_ok();
    let me: Me = res.json();
    assert_eq!(me.auth.method.as_str(), "session");
    assert!(me.auth.token.is_none());
    assert_eq!(me.user.email, crate::common::OWNER_EMAIL);
}

#[tokio::test]
async fn no_credentials_401_unauthenticated() {
    let app = TestApp::with_owner().await;
    let res = app.client().get("/api/v1/auth/me").await;
    assert_error(&res, 401, ErrorCode::Unauthenticated);
    // An unknown or expired session is the same.
    let res = app
        .client()
        .get("/api/v1/auth/me")
        .add_header(
            "cookie",
            "quire_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .await;
    assert_error(&res, 401, ErrorCode::Unauthenticated);
    app.exec("UPDATE sessions SET expires_at = 1", vec![]).await;
    assert_error(
        &app.server.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
}

#[tokio::test]
async fn secure_cookie_name_ignored_on_plain_http() {
    let app = TestApp::with_owner().await;
    let session = app.session.clone().unwrap();
    let res = app
        .client()
        .get("/api/v1/auth/me")
        .add_header("cookie", format!("__Host-quire_session={session}"))
        .await;
    assert_error(&res, 401, ErrorCode::Unauthenticated);
}

#[tokio::test]
async fn setup_required_409_before_an_owner_exists() {
    let app = TestApp::new().await;
    let res = app.server.get("/api/v1/auth/me").await;
    assert_error(&res, 409, ErrorCode::SetupRequired);
    // The cache re-checks while false: an owner inserted behind the server's back is seen on the next request.
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Read).await;
    app.token_client(&token)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn quire_bearer_wins_over_cookie() {
    let app = TestApp::with_owner().await;
    // A valid cookie with a bad Quire token: 401 invalid_token, never a cookie fallback.
    let res = app
        .server
        .get("/api/v1/auth/me")
        .add_header("authorization", "Bearer qpat_not_a_real_token")
        .await;
    assert_error(&res, 401, ErrorCode::InvalidToken);
}

#[tokio::test]
async fn foreign_bearer_ignored() {
    let app = TestApp::with_owner().await;
    for value in ["Bearer eyJhbGciOiJIUzI1NiJ9.e30.x", "Basic dXNlcjpwYXNz"] {
        let res = app
            .server
            .get("/api/v1/auth/me")
            .add_header("authorization", value)
            .await;
        res.assert_status_ok();
        let me: Me = res.json();
        assert_eq!(me.auth.method.as_str(), "session", "{value}");
    }
}

#[tokio::test]
async fn csrf_cross_site_403() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .post("/api/v1/collections")
        .add_header("sec-fetch-site", "cross-site")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
}

#[tokio::test]
async fn csrf_no_fetch_metadata_no_origin_403() {
    let app = TestApp::with_owner().await;
    let session = app.session.clone().unwrap();
    let res = app
        .client()
        .post("/api/v1/tokens")
        .add_header("cookie", format!("quire_session={session}"))
        .json(&json!({"name": "x", "scopes": ["read"]}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    // A foreign Origin without fetch metadata is refused too; the request's own host is fine.
    let res = app
        .client()
        .post("/api/v1/tokens")
        .add_header("cookie", format!("quire_session={session}"))
        .add_header("origin", "https://evil.example")
        .json(&json!({"name": "x", "scopes": ["read"]}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    let res = app
        .client()
        .post("/api/v1/tokens")
        .add_header("cookie", format!("quire_session={session}"))
        .add_header("origin", "http://localhost")
        .json(&json!({"name": "x", "scopes": ["read"]}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn bearer_skips_csrf() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .add_header("sec-fetch-site", "cross-site")
        .add_header("origin", "https://evil.example")
        .json(&json!({"name": "x", "scopes": ["read"], "expires_in_days": 1}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn token_only_route_refuses_session_403() {
    let app = TestApp::with_owner().await;
    let res = app.server.delete("/api/v1/tokens/current").await;
    assert_error(&res, 403, ErrorCode::Forbidden);
}

async fn session_only(_auth: Auth<Session>) -> &'static str {
    "session"
}

async fn token_only(_auth: Auth<AnyToken>) -> &'static str {
    "token"
}

async fn write_scope(auth: Auth<Write>) -> String {
    auth.token_id().unwrap_or("session").to_owned()
}

async fn page(IdPath(id): IdPath, item: ItemRef) -> String {
    format!("{id} {}", item.column())
}

/// `Auth<Session>`, `Auth<AnyToken>` and `Auth<Write>` on a probe router (no W0b route is session-only yet).
#[tokio::test]
async fn auth_requirements_on_a_probe_router() {
    let app = TestApp::with_owner().await;
    let router = Router::new()
        .route("/probe/session", get(session_only))
        .route("/probe/token", get(token_only))
        .route("/probe/write", get(write_scope))
        .route("/probe/items/{id}/pages/{n}", get(page))
        .with_state(app.state.clone());
    let probe = axum_test::TestServer::new(router);
    let cookie = format!("quire_session={}", app.session.clone().unwrap());
    let read = app.mint_token(app.owner(), Scope::Read).await;
    let write = app.mint_token(app.owner(), Scope::Write).await;
    let bearer = |t: &str| format!("Bearer {t}");

    let res = probe
        .get("/probe/session")
        .add_header("cookie", cookie.clone())
        .await;
    res.assert_text("session");
    let res = probe
        .get("/probe/session")
        .add_header("authorization", bearer(&write))
        .await;
    assert_eq!(res.status_code(), 403);
    assert_eq!(
        res.json::<serde_json::Value>()["error"]["code"],
        "session_required"
    );

    let res = probe
        .get("/probe/token")
        .add_header("authorization", bearer(&read))
        .await;
    res.assert_text("token");
    let res = probe
        .get("/probe/token")
        .add_header("cookie", cookie.clone())
        .await;
    assert_eq!(res.status_code(), 403);
    assert_eq!(
        res.json::<serde_json::Value>()["error"]["code"],
        "forbidden"
    );

    let res = probe
        .get("/probe/write")
        .add_header("cookie", cookie.clone())
        .await;
    res.assert_text("session");
    let res = probe
        .get("/probe/write")
        .add_header("authorization", bearer(&write))
        .await;
    res.assert_status_ok();
    let res = probe
        .get("/probe/write")
        .add_header("authorization", bearer(&read))
        .await;
    assert_eq!(res.status_code(), 403);
    assert_eq!(
        res.json::<serde_json::Value>()["error"]["code"],
        "insufficient_scope"
    );

    for path in ["/probe/session", "/probe/token", "/probe/write"] {
        let res = probe.get(path).await;
        assert_eq!(res.status_code(), 401, "{path}");
    }

    // IdPath and ItemRef read `{id}` by name, also next to other path parameters.
    let id = uuid::Uuid::now_v7();
    probe
        .get(&format!("/probe/items/{id}/pages/2"))
        .await
        .assert_text(format!("{id} id"));
    let res = probe.get("/probe/items/aB3dE5gH9k/pages/2").await;
    assert_eq!(res.status_code(), 404, "a url_id is not an IdPath");
}

#[tokio::test]
async fn host_not_allowlisted_421() {
    let app = TestApp::with_owner().await;
    let res = app
        .client()
        .get("/api/v1/openapi.json")
        .add_header("host", "evil.example")
        .await;
    assert_error(&res, 421, ErrorCode::HostNotAllowed);
    // /healthz is exempt.
    let res = app
        .client()
        .get("/healthz")
        .add_header("host", "evil.example")
        .await;
    res.assert_status_ok();
    res.assert_json(&json!({"status": "ok"}));
}

#[tokio::test]
async fn unversioned_api_path_404_json() {
    let app = TestApp::with_owner().await;
    for path in ["/api/v2/items", "/api/nope", "/api/v1/nope", "/api"] {
        let res = app.server.get(path).await;
        assert_error(&res, 404, ErrorCode::NotFound);
    }
    // Outside /api/ an unknown file is a plain 404.
    let res = app.server.get("/nope.js").await;
    assert_eq!(res.status_code(), 404);
}

#[tokio::test]
async fn method_not_allowed_405_envelope() {
    let app = TestApp::with_owner().await;
    let res = app.server.put("/api/v1/auth/me").json(&json!({})).await;
    assert_error(&res, 405, ErrorCode::MethodNotAllowed);
}

#[tokio::test]
async fn malformed_json_422_envelope() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .text("{not json")
        .content_type("application/json")
        .await;
    let body = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(body.error.details, Some(json!({"field": "body"})));
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
    // A non-JSON content type is 415.
    let res = app
        .server
        .post("/api/v1/tokens")
        .text("name=x")
        .content_type("application/x-www-form-urlencoded")
        .await;
    assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
}

#[tokio::test]
async fn api_version_header_on_every_response() {
    let app = TestApp::with_owner().await;
    for path in ["/api/v1/auth/me", "/api/v1/nope", "/api/v1/openapi.json"] {
        let res = app.server.get(path).await;
        assert_eq!(
            res.header("quire-api-version"),
            quire_api::API_VERSION,
            "{path}"
        );
        assert!(!res.header("x-request-id").is_empty());
    }
    let res = app.server.get("/healthz").await;
    assert!(res.maybe_header("quire-api-version").is_none());
    assert!(res.maybe_header("x-request-id").is_some());
}

#[tokio::test]
async fn default_body_limit_413() {
    let app = TestApp::with_owner().await;
    let name = "x".repeat(1024 * 1024 + 10);
    let res = app
        .server
        .post("/api/v1/collections")
        .json(&json!({ "name": name }))
        .await;
    let body = assert_error(&res, 413, ErrorCode::PayloadTooLarge);
    assert_eq!(body.error.details, Some(json!({"limit_bytes": 1048576})));
}
