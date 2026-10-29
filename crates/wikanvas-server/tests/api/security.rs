//! Scope enforcement over the generated spec, Host allowlist, CSRF, CORS and limits (owner: W2)
//!
//! Both branches of `Auth<S>` (the cookie and a Quire bearer token, D43), `session_required` and `forbidden`; the
//! generic scope test over the spec (§7.4); the Host allowlist with `QUIRE_DOMAIN`; the trusted-proxy rule, HSTS and
//! the derived public URL (§8, D57); every CSRF branch; 415 and 413; the per-token and failed-bearer limiters on the
//! fake clock; CORS; the 408 envelope; and the redaction of credentials in the access log.

use std::collections::BTreeSet;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::routing::get;
use quire_api::{ErrorBody, ErrorCode, Me, Scope};
use quire_server::extract::{AnyToken, Auth, IdPath, ItemRef, Session, Write};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::common::{OWNER_PASSWORD, PEER, TestApp, assert_error, router, set_cookies};

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

/// `Auth<Session>`, `Auth<AnyToken>` and `Auth<Write>` on a probe router, independent of the real routes.
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

#[tokio::test]
async fn host_allowlist_accepts_quire_domain_any_port() {
    let app = TestApp::from_vars(&[("QUIRE_DOMAIN", "quire.lan")]).await;
    for (host, allowed) in [
        ("quire.lan", true),
        ("quire.lan:8443", true),
        ("QUIRE.LAN:443", true),
        ("localhost:8080", true),
        ("evil.lan", false),
        ("quire.lan.evil", false),
        ("evil.quire.lan", false),
    ] {
        let res = app
            .client()
            .get("/api/v1/config")
            .add_header("host", host)
            .await;
        if allowed {
            res.assert_status_ok();
        } else {
            assert_error(&res, 421, ErrorCode::HostNotAllowed);
        }
    }
    // The allowlist applies to bearer requests too, and X-Forwarded-Host never widens it.
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Read).await;
    let res = app
        .token_client(&token)
        .get("/api/v1/auth/me")
        .add_header("host", "evil.lan")
        .await;
    assert_error(&res, 421, ErrorCode::HostNotAllowed);
    let res = app
        .client()
        .get("/api/v1/config")
        .add_header("host", "evil.lan")
        .add_header("x-forwarded-host", "quire.lan")
        .await;
    assert_error(&res, 421, ErrorCode::HostNotAllowed);
    let allowed = TestApp::from_vars(&[("QUIRE_ALLOWED_HOSTS", "quire, wiki.lan:8443")]).await;
    for (host, ok) in [
        ("quire:8080", true),
        ("wiki.lan:8443", true),
        ("wiki.lan", false),
    ] {
        let res = allowed
            .client()
            .get("/api/v1/config")
            .add_header("host", host)
            .await;
        assert_eq!(res.status_code() == 200, ok, "{host}");
    }
}

fn login_as(app: &TestApp, email: &str, peer: &str, xff: &str) -> axum_test::TestRequest {
    app.anon_client()
        .post("/api/v1/auth/login")
        .add_header(PEER, peer.to_owned())
        .add_header("x-forwarded-for", xff.to_owned())
        .json(&json!({"email": email, "password": "not the password"}))
}

#[tokio::test]
async fn forwarded_headers_only_from_trusted_proxy() {
    let app = TestApp::from_vars(&[("QUIRE_TRUSTED_PROXIES", "172.31.240.2")]).await;
    app.insert_owner().await;
    // Untrusted peer: X-Forwarded-For does not change the limiter's key (the socket peer).
    for i in 0..5 {
        let res = login_as(
            &app,
            &format!("u{i}@quire.invalid"),
            "10.0.0.5:4000",
            &format!("203.0.113.{i}"),
        )
        .await;
        assert_error(&res, 401, ErrorCode::InvalidCredentials);
    }
    let res = login_as(&app, "u9@quire.invalid", "10.0.0.5:4001", "203.0.113.99").await;
    assert_error(&res, 429, ErrorCode::RateLimited);
    // Trusted peer: the rightmost address is the client.
    for i in 0..5 {
        let res = login_as(
            &app,
            &format!("t{i}@quire.invalid"),
            "172.31.240.2:4000",
            "10.0.0.5, 192.0.2.1",
        )
        .await;
        assert_error(&res, 401, ErrorCode::InvalidCredentials);
    }
    let res = login_as(
        &app,
        "t8@quire.invalid",
        "172.31.240.2:4000",
        "192.0.2.2, 192.0.2.1",
    )
    .await;
    assert_error(&res, 429, ErrorCode::RateLimited);
    let res = login_as(
        &app,
        "t9@quire.invalid",
        "172.31.240.2:4000",
        "192.0.2.1, 192.0.2.2",
    )
    .await;
    assert_error(&res, 401, ErrorCode::InvalidCredentials);
    // X-Forwarded-Proto: https gives Secure only from the trusted peer.
    let login = |peer: &'static str| {
        app.anon_client()
            .post("/api/v1/auth/login")
            .add_header(PEER, peer)
            .add_header("x-forwarded-proto", "https")
            .add_header("x-forwarded-for", "198.51.100.77")
            .json(&json!({"email": crate::common::OWNER_EMAIL, "password": OWNER_PASSWORD}))
    };
    let direct = set_cookies(&login("10.0.0.6:4000").await);
    assert!(
        direct[0].starts_with("quire_session=") && !direct[0].contains("Secure"),
        "{direct:?}"
    );
    let proxied = login("172.31.240.2:4000").await;
    let cookies = set_cookies(&proxied);
    assert!(
        cookies[0].starts_with("__Host-quire_session=") && cookies[0].contains("; Secure"),
        "{cookies:?}"
    );
    // The session records the forwarded client address.
    let ips: Vec<String> = app
        .state
        .db
        .read(|c| {
            Ok(c.prepare("SELECT ip FROM sessions ORDER BY ip")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await
        .unwrap();
    assert_eq!(ips, ["10.0.0.6", "198.51.100.77"]);
}

#[tokio::test]
async fn hsts_only_on_https_in_acme_mode() {
    let vars = |tls: &'static str| {
        [
            ("QUIRE_TLS", tls),
            ("QUIRE_TRUSTED_PROXIES", "172.31.240.2"),
            ("QUIRE_DOMAIN", "wiki.example.org"),
        ]
    };
    let acme = TestApp::from_vars(&vars("acme")).await;
    let get = |app: &TestApp, path: &'static str, peer: &'static str, https: bool| {
        let mut req = app
            .client()
            .get(path)
            .add_header("host", "wiki.example.org")
            .add_header(PEER, peer);
        if https {
            req = req.add_header("x-forwarded-proto", "https");
        }
        req
    };
    for path in ["/api/v1/config", "/api/v1/auth/me", "/", "/nope.js"] {
        let res = get(&acme, path, "172.31.240.2:1", true).await;
        assert_eq!(
            res.header("strict-transport-security"),
            "max-age=31536000",
            "{path}"
        );
        let res = get(&acme, path, "172.31.240.2:1", false).await;
        assert!(
            res.maybe_header("strict-transport-security").is_none(),
            "{path} over plain HTTP"
        );
        let res = get(&acme, path, "10.0.0.9:1", true).await;
        assert!(
            res.maybe_header("strict-transport-security").is_none(),
            "{path} from an untrusted peer"
        );
    }
    let internal = TestApp::from_vars(&vars("internal")).await;
    let res = get(&internal, "/api/v1/config", "172.31.240.2:1", true).await;
    assert!(res.maybe_header("strict-transport-security").is_none());
}

fn setup_post(app: &TestApp, host: &str, origin: &str) -> axum_test::TestRequest {
    app.client()
        .post("/api/v1/setup")
        .add_header("host", host.to_owned())
        .add_header("origin", origin.to_owned())
        .json(&json!({"code": "WRONG", "workspace_name": "W", "name": "N", "email": "a@b", "password": "x"}))
}

#[tokio::test]
async fn public_url_derived_from_domain() {
    let app =
        TestApp::from_vars(&[("QUIRE_DOMAIN", "quire.lan"), ("QUIRE_HTTPS_PORT", "8443")]).await;
    assert_eq!(
        app.state.config.public_url.to_string(),
        "https://quire.lan:8443"
    );
    let message = app.state.setup_message();
    assert!(
        message.starts_with("First-run setup: https://quire.lan:8443/setup code "),
        "{message}"
    );
    // The public URL is the CSRF origin, whatever Host the request came in with.
    let res = setup_post(&app, "localhost:8080", "https://quire.lan:8443").await;
    assert_error(&res, 401, ErrorCode::InvalidCredentials);
    let res = setup_post(&app, "localhost:8080", "https://quire.lan").await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    let default_port = TestApp::from_vars(&[("QUIRE_DOMAIN", "quire.lan")]).await;
    assert_eq!(
        default_port.state.config.public_url.to_string(),
        "https://quire.lan"
    );
    // An explicit QUIRE_PUBLIC_URL wins.
    let explicit = TestApp::from_vars(&[
        ("QUIRE_DOMAIN", "quire.lan"),
        ("QUIRE_HTTPS_PORT", "8443"),
        ("QUIRE_PUBLIC_URL", "http://wiki.lan:9000"),
    ])
    .await;
    assert!(
        explicit
            .state
            .setup_message()
            .starts_with("First-run setup: http://wiki.lan:9000/setup code ")
    );
    let res = setup_post(&explicit, "localhost:8080", "http://wiki.lan:9000").await;
    assert_error(&res, 401, ErrorCode::InvalidCredentials);
    let res = setup_post(&explicit, "localhost:8080", "https://quire.lan:8443").await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
}

/// A cookie client without fetch metadata (the plain-HTTP browser path).
fn bare_cookie_client(app: &TestApp) -> axum_test::TestServer {
    let mut client = app.client();
    client.add_header(
        "cookie",
        format!("quire_session={}", app.session.clone().unwrap()),
    );
    client
}

#[tokio::test]
async fn csrf_origin_only_cross_site_403() {
    let app = TestApp::with_owner().await;
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("origin", "https://evil.example")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("origin", "null")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
}

#[tokio::test]
async fn csrf_origin_only_same_host_ok() {
    let app = TestApp::with_owner().await;
    for (host, origin) in [
        ("localhost", "http://localhost"),
        ("localhost:8080", "http://localhost:8080"),
    ] {
        let res = bare_cookie_client(&app)
            .post("/api/v1/collections")
            .add_header("host", host)
            .add_header("origin", origin)
            .json(&json!({"name": "x"}))
            .await;
        res.assert_status(StatusCode::CREATED);
    }
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("host", "localhost:8080")
        .add_header("origin", "http://localhost:8081")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
}

#[tokio::test]
async fn csrf_same_origin_ok() {
    let app = TestApp::with_owner().await;
    for site in ["same-origin", "none"] {
        let res = bare_cookie_client(&app)
            .post("/api/v1/collections")
            .add_header("sec-fetch-site", site)
            .add_header("origin", "https://evil.example")
            .json(&json!({"name": "x"}))
            .await;
        res.assert_status(StatusCode::CREATED);
    }
    for site in ["same-site", "cross-site"] {
        let res = bare_cookie_client(&app)
            .post("/api/v1/collections")
            .add_header("sec-fetch-site", site)
            .json(&json!({"name": "x"}))
            .await;
        assert_error(&res, 403, ErrorCode::CsrfRejected);
    }
}

#[tokio::test]
async fn basic_auth_header_takes_cookie_path_with_csrf() {
    let app = TestApp::with_owner().await;
    let post = |site: &'static str| {
        bare_cookie_client(&app)
            .post("/api/v1/collections")
            .add_header("authorization", "Basic dXNlcjpwYXNz")
            .add_header("sec-fetch-site", site)
            .json(&json!({"name": "x"}))
    };
    assert_error(&post("cross-site").await, 403, ErrorCode::CsrfRejected);
    post("same-origin").await.assert_status(StatusCode::CREATED);
    // A foreign Bearer value is the same: the cookie path with the full check.
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("authorization", "Bearer eyJhbGciOiJIUzI1NiJ9.e30.x")
        .add_header("sec-fetch-site", "cross-site")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
}

#[tokio::test]
async fn form_body_415() {
    let app = TestApp::with_owner().await;
    for content_type in [
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "text/plain",
    ] {
        let res = app
            .server
            .post("/api/v1/collections")
            .bytes("name=x".into())
            .content_type(content_type)
            .await;
        assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
        let res = app
            .server
            .post("/api/v1/attachments?filename=a.txt")
            .bytes("hello".into())
            .content_type(content_type)
            .await;
        assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
    }
}

#[tokio::test]
async fn session_only_routes_refuse_tokens_403() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/auth/password")
        .json(&json!({"current": OWNER_PASSWORD, "new": "a new long password"}))
        .await;
    assert_error(&res, 403, ErrorCode::SessionRequired);
    let res = app.server.post("/api/v1/auth/logout").await;
    assert_error(&res, 403, ErrorCode::SessionRequired);
}

/// How an operation's `security` says it is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declared {
    Public,
    Scope(Scope),
    Session,
    Token,
}

fn declared(security: &Value) -> Declared {
    let reqs = security.as_array().expect("security is an array");
    if reqs.is_empty() {
        return Declared::Public;
    }
    let bearer = reqs.iter().find_map(|r| r.get("bearer"));
    let session = reqs.iter().any(|r| r.get("session").is_some());
    match (bearer.and_then(|b| b.as_array()), session) {
        (Some(scopes), true) if scopes.len() == 1 => {
            Declared::Scope(scopes[0].as_str().unwrap().parse().unwrap())
        }
        (None, true) => Declared::Session,
        (Some(scopes), false) if scopes.is_empty() => Declared::Token,
        _ => panic!("unexpected security {security}"),
    }
}

/// Every operation of the spec: (method, path with `{…}` replaced by random UUIDs, declared security).
fn operations() -> Vec<(Method, String, Declared)> {
    let spec: Value = serde_json::from_str(&quire_server::openapi::spec_json()).unwrap();
    let mut out = Vec::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        let concrete: String = path
            .split('/')
            .map(|seg| {
                if seg.starts_with('{') {
                    uuid::Uuid::now_v7().to_string()
                } else {
                    seg.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        for (method, op) in item.as_object().unwrap() {
            let method: Method = method.to_uppercase().parse().unwrap();
            let security = op
                .get("security")
                .unwrap_or_else(|| panic!("{method} {path} declares no security"));
            out.push((method, concrete.clone(), declared(security)));
        }
    }
    out
}

async fn call(
    client: &axum_test::TestServer,
    method: &Method,
    path: &str,
    headers: &[(&str, String)],
) -> axum_test::TestResponse {
    let mut req = client.method(method.clone(), path);
    for (k, v) in headers {
        req = req.add_header(k.to_string(), v.clone());
    }
    if [Method::POST, Method::PUT, Method::PATCH].contains(method) {
        req = req.json(&json!({}));
    }
    req.await
}

#[tokio::test]
async fn every_operation_enforces_its_declared_scope() {
    let mut app = TestApp::with_config(|c| c.token_rate_per_min = 100_000).await;
    app.sign_in_owner().await;
    let tokens: Vec<String> = {
        let mut t = Vec::new();
        for scope in [Scope::Read, Scope::Write, Scope::Admin] {
            t.push(app.mint_token(app.owner(), scope).await);
        }
        t
    };
    let bearer = |scope: Scope| {
        (
            "authorization",
            format!("Bearer {}", tokens[scope as usize]),
        )
    };
    let anon = [
        ("sec-fetch-site", "same-origin".to_owned()),
        ("host", "localhost".to_owned()),
    ];
    let cookie = [
        ("sec-fetch-site", "same-origin".to_owned()),
        ("host", "localhost".to_owned()),
        (
            "cookie",
            format!("quire_session={}", app.session.clone().unwrap()),
        ),
    ];
    let client = app.client();
    let ops = operations();
    assert!(ops.len() >= 20, "{} operations", ops.len());
    for (method, path, declared) in &ops {
        let what = format!("{method} {path} ({declared:?})");
        if *declared == Declared::Public {
            continue;
        }
        let res = call(&client, method, path, &anon).await;
        assert_error(&res, 401, ErrorCode::Unauthenticated);
        match declared {
            Declared::Scope(scope) => {
                let below = match scope {
                    Scope::Read => None,
                    Scope::Write => Some(Scope::Read),
                    Scope::Admin => Some(Scope::Write),
                };
                if let Some(below) = below {
                    let res = call(&client, method, path, &[bearer(below)]).await;
                    let body: ErrorBody = res.json();
                    assert_eq!(
                        (res.status_code(), body.error.code),
                        (StatusCode::FORBIDDEN, ErrorCode::InsufficientScope),
                        "{what}"
                    );
                    assert_eq!(
                        body.error.details.unwrap()["required"],
                        scope.as_str(),
                        "{what}"
                    );
                }
                let res = call(&client, method, path, &[bearer(*scope)]).await;
                let code = serde_json::from_slice::<Value>(res.as_bytes())
                    .ok()
                    .and_then(|v| v["error"]["code"].as_str().map(str::to_owned));
                assert!(
                    res.status_code() != 401
                        && !matches!(
                            code.as_deref(),
                            Some("insufficient_scope" | "session_required")
                        ),
                    "{what}: the declared scope passes, got {} {}",
                    res.status_code(),
                    res.text()
                );
            }
            Declared::Session => {
                let res = call(&client, method, path, &[bearer(Scope::Admin)]).await;
                assert_error(&res, 403, ErrorCode::SessionRequired);
            }
            Declared::Token => {
                let res = call(&client, method, path, &cookie).await;
                assert_error(&res, 403, ErrorCode::Forbidden);
            }
            Declared::Public => unreachable!(),
        }
    }
}

#[test]
fn public_operations_are_exactly() {
    let public: BTreeSet<String> = operations()
        .into_iter()
        .filter(|(_, _, d)| *d == Declared::Public)
        .map(|(m, p, _)| format!("{m} {p}"))
        .collect();
    let expect: BTreeSet<String> = [
        "GET /api/v1/config",
        "GET /api/v1/openapi.json",
        "GET /api/v1/setup",
        "POST /api/v1/setup",
        "POST /api/v1/auth/login",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(public, expect);
}

fn unknown_token(i: u8) -> String {
    quire_api::token::encode(&[i; 32])
}

#[tokio::test]
async fn failed_bearer_limited_per_ip() {
    let app = TestApp::with_fake_clock().await;
    app.insert_owner().await;
    let me = |token: String, peer: &'static str| {
        app.client()
            .get("/api/v1/auth/me")
            .add_header(PEER, peer)
            .add_header("authorization", format!("Bearer {token}"))
    };
    for i in 0..30 {
        assert_error(
            &me(unknown_token(i), "192.0.2.10:1").await,
            401,
            ErrorCode::InvalidToken,
        );
    }
    let res = me(unknown_token(99), "192.0.2.10:2").await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "auth_failures"})));
    assert_eq!(res.header("retry-after"), "2");
    // A malformed token counts too, without a lookup.
    assert_error(
        &me("qpat_short".into(), "192.0.2.10:3").await,
        429,
        ErrorCode::RateLimited,
    );
    assert_error(
        &me(unknown_token(1), "192.0.2.11:1").await,
        401,
        ErrorCode::InvalidToken,
    );
    app.state.clock.advance(Duration::from_secs(2));
    assert_error(
        &me(unknown_token(1), "192.0.2.10:1").await,
        401,
        ErrorCode::InvalidToken,
    );
}

#[tokio::test]
async fn failed_bearer_limit_spares_valid_tokens() {
    let app = TestApp::with_fake_clock().await;
    let owner = app.insert_owner().await;
    let valid = app.mint_token(&owner, Scope::Read).await;
    let me = |token: &str| {
        app.client()
            .get("/api/v1/auth/me")
            .add_header(PEER, "192.0.2.20:1")
            .add_header("authorization", format!("Bearer {token}"))
    };
    for i in 0..31 {
        me(&unknown_token(i)).await;
    }
    assert_error(&me(&unknown_token(200)).await, 429, ErrorCode::RateLimited);
    me(&valid).await.assert_status_ok();
}

#[tokio::test]
async fn token_rate_limit_429_retry_after() {
    let app = TestApp::with_config(|c| {
        c.fake_clock = true;
        c.token_rate_per_min = 60;
    })
    .await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Read).await;
    let other = app.mint_token(&owner, Scope::Read).await;
    let client = app.token_client(&token);
    for _ in 0..quire_server::config::TOKEN_BURST {
        client.get("/api/v1/auth/me").await.assert_status_ok();
    }
    let res = client.get("/api/v1/auth/me").await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "requests"})));
    assert_eq!(res.header("retry-after"), "1");
    app.token_client(&other)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
    app.state.clock.advance(Duration::from_secs(1));
    client.get("/api/v1/auth/me").await.assert_status_ok();
}

#[tokio::test]
async fn token_byte_budget_429_reason_bytes() {
    let app = TestApp::with_config(|c| {
        c.fake_clock = true;
        c.token_bytes_per_hour = 4 * 1024;
    })
    .await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Write).await;
    let client = app.token_client(&token);
    // About 2 KiB per body; the routes refuse it (name too long), but the bytes count.
    let body = json!({ "name": "x".repeat(2000) });
    for _ in 0..2 {
        let res = client.post("/api/v1/collections").json(&body).await;
        assert_error(&res, 422, ErrorCode::Validation);
    }
    let res = client.post("/api/v1/collections").json(&body).await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "bytes"})));
    let retry: u64 = res.header("retry-after").to_str().unwrap().parse().unwrap();
    assert!((900..=1800).contains(&retry), "{retry}");
    // A body-less request still goes through, and after the wait the budget is back.
    client.get("/api/v1/auth/me").await.assert_status_ok();
    app.state.clock.advance(Duration::from_secs(retry));
    let res = client.post("/api/v1/collections").json(&body).await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn token_bytes_in_small_chunks_cost_their_size() {
    let app = TestApp::with_config(|c| {
        c.fake_clock = true;
        c.token_bytes_per_hour = 4 * 1024;
    })
    .await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Write).await;
    // About 1.5 KiB without Content-Length, in 1-byte and in 100-byte chunks: 2 KiB each, as one chunk would cost.
    let post = |chunk_size: usize| {
        let json = serde_json::to_vec(&json!({ "name": "z".repeat(1500) })).unwrap();
        let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = json
            .chunks(chunk_size)
            .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
            .collect();
        Request::post("/api/v1/collections")
            .header("host", "localhost")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from_stream(futures_util::stream::iter(chunks)))
            .unwrap()
    };
    let first = router(&app.state).oneshot(post(1)).await.unwrap();
    assert_eq!(
        first.status(),
        422,
        "the name is too long, but the body was read"
    );
    let second = router(&app.state).oneshot(post(100)).await.unwrap();
    assert_eq!(second.status(), 422);
    // The 4 KiB are used up now.
    let third = router(&app.state).oneshot(post(1)).await.unwrap();
    assert_eq!(third.status(), 429);
}

#[tokio::test]
async fn token_bytes_counted_as_read_without_content_length() {
    let app = TestApp::with_config(|c| {
        c.fake_clock = true;
        c.token_bytes_per_hour = 4 * 1024;
    })
    .await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Write).await;
    let post = || {
        let chunk =
            bytes::Bytes::from(serde_json::to_vec(&json!({ "name": "y".repeat(3000) })).unwrap());
        let body = Body::from_stream(futures_util::stream::iter([Ok::<_, std::io::Error>(chunk)]));
        Request::post("/api/v1/collections")
            .header("host", "localhost")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body)
            .unwrap()
    };
    let first = router(&app.state).oneshot(post()).await.unwrap();
    assert_eq!(first.status(), 422);
    let second = router(&app.state).oneshot(post()).await.unwrap();
    assert_eq!(second.status(), 429);
    assert!(second.headers().contains_key("retry-after"));
    let body = axum::body::to_bytes(second.into_body(), 1 << 20)
        .await
        .unwrap();
    let err: ErrorBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(err.error.details, Some(json!({"reason": "bytes"})));
}

#[tokio::test]
async fn timeout_408_envelope() {
    let mut app = TestApp::with_config(|c| c.request_timeout = Duration::from_millis(200)).await;
    app.sign_in_owner().await;
    // A body that never ends keeps the handler waiting past the deadline.
    let stalled = Body::from_stream(futures_util::stream::pending::<
        Result<bytes::Bytes, std::io::Error>,
    >());
    let req = Request::post("/api/v1/tokens")
        .header("host", "localhost")
        .header("sec-fetch-site", "same-origin")
        .header(
            "cookie",
            format!("quire_session={}", app.session.clone().unwrap()),
        )
        .header("content-type", "application/json")
        .body(stalled)
        .unwrap();
    let res = router(&app.state).oneshot(req).await.unwrap();
    assert_eq!(res.status(), 408);
    let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    assert_eq!(res.headers()["quire-api-version"], quire_api::API_VERSION);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let err: ErrorBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(err.error.code, ErrorCode::RequestTimeout);
    assert_eq!(err.request_id.as_deref(), Some(id.as_str()));
}

fn preflight(client: &axum_test::TestServer, origin: &'static str) -> axum_test::TestRequest {
    client
        .method(Method::OPTIONS, "/api/v1/tokens")
        .add_header("origin", origin)
        .add_header("access-control-request-method", "POST")
        .add_header(
            "access-control-request-headers",
            "authorization, content-type",
        )
}

#[tokio::test]
async fn cors_off_by_default() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = preflight(&app.client(), "https://tool.example").await;
    assert!(
        res.headers()
            .keys()
            .all(|k| !k.as_str().starts_with("access-control-")),
        "{:?}",
        res.headers()
    );
    let res = app
        .server
        .get("/api/v1/tokens")
        .add_header("origin", "https://tool.example")
        .await;
    res.assert_status_ok();
    assert!(
        res.headers()
            .keys()
            .all(|k| !k.as_str().starts_with("access-control-"))
    );
}

#[tokio::test]
async fn cors_allow_list_never_credentials() {
    let mut app = TestApp::from_vars(&[(
        "QUIRE_CORS_ORIGINS",
        "https://tool.example, http://localhost:5173",
    )])
    .await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Admin).await;
    app.server = app.token_client(&token);
    let res = preflight(&app.client(), "https://tool.example").await;
    assert!(res.status_code().is_success(), "{}", res.status_code());
    assert_eq!(
        res.header("access-control-allow-origin"),
        "https://tool.example"
    );
    let methods = res
        .header("access-control-allow-methods")
        .to_str()
        .unwrap()
        .to_uppercase();
    for m in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        assert!(methods.contains(m), "{methods}");
    }
    let headers = res
        .header("access-control-allow-headers")
        .to_str()
        .unwrap()
        .to_lowercase();
    for h in [
        "authorization",
        "content-type",
        "if-match",
        "if-none-match",
        "x-quire-engine",
    ] {
        assert!(headers.contains(h), "{headers}");
    }
    assert_eq!(res.header("access-control-max-age"), "600");
    assert!(
        res.maybe_header("access-control-allow-credentials")
            .is_none()
    );
    let res = app
        .server
        .get("/api/v1/tokens")
        .add_header("origin", "https://tool.example")
        .await;
    res.assert_status_ok();
    assert_eq!(
        res.header("access-control-allow-origin"),
        "https://tool.example"
    );
    let exposed = res
        .header("access-control-expose-headers")
        .to_str()
        .unwrap()
        .to_lowercase();
    for h in [
        "etag",
        "retry-after",
        "x-request-id",
        "quire-api-version",
        "quire-conversion-warnings",
    ] {
        assert!(exposed.contains(h), "{exposed}");
    }
    assert!(
        res.maybe_header("access-control-allow-credentials")
            .is_none()
    );
    // Another origin gets nothing.
    let res = preflight(&app.client(), "https://evil.example").await;
    assert!(res.maybe_header("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn cors_star_refused_at_startup() {
    let err = quire_server::config::Config::from_lookup(|k| {
        (k == "QUIRE_CORS_ORIGINS").then(|| "*".to_owned())
    })
    .unwrap_err();
    assert_eq!(err.var, "QUIRE_CORS_ORIGINS");
    let dir = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quire-server"))
        .arg("serve")
        .env_clear()
        .env("QUIRE_DATA_DIR", dir.path())
        .env("QUIRE_LISTEN", "127.0.0.1:0")
        .env("QUIRE_CORS_ORIGINS", "https://ok.example, *")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("QUIRE_CORS_ORIGINS")
            || String::from_utf8_lossy(&out.stderr).contains("QUIRE_CORS_ORIGINS")
    );
}

#[tokio::test]
async fn cookie_from_cors_origin_still_csrf_rejected() {
    let mut app = TestApp::from_vars(&[("QUIRE_CORS_ORIGINS", "https://tool.example")]).await;
    app.sign_in_owner().await;
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("origin", "https://tool.example")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    let res = bare_cookie_client(&app)
        .post("/api/v1/collections")
        .add_header("origin", "https://tool.example")
        .add_header("sec-fetch-site", "cross-site")
        .json(&json!({"name": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::CsrfRejected);
    assert!(
        res.maybe_header("access-control-allow-credentials")
            .is_none()
    );
}

/// One HTTP/1.1 request over a fresh connection; returns (status, head, body).
fn http(addr: std::net::SocketAddr, request: &str) -> (u16, String, String) {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (
        head[9..12].parse().unwrap(),
        head.to_owned(),
        body.to_owned(),
    )
}

/// The access log of a real `quire-server serve` (debug level): token requests carry `token_id` and the client IP,
/// headers are logged with `Authorization` and `Cookie` redacted, and no secret appears anywhere.
#[test]
fn authorization_redacted_in_logs() {
    let dir = tempfile::tempdir().unwrap();
    let owner = crate::common::admin(
        dir.path(),
        &[
            "create-owner",
            "--email",
            "a@quire.invalid",
            "--name",
            "A",
            "--workspace",
            "W",
            "--password-stdin",
        ],
        Some(&format!("{OWNER_PASSWORD}\n")),
    );
    assert_eq!(owner.status.code(), Some(0));
    let created = crate::common::stdout_json(&crate::common::admin(
        dir.path(),
        &["--json", "create-token", "--name", "log", "--scope", "read"],
        None,
    ));
    let token = created["token"].as_str().unwrap().to_owned();
    let token_id = created["api_token"]["id"].as_str().unwrap().to_owned();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_quire-server"))
        .arg("serve")
        .env_clear()
        .env("QUIRE_DATA_DIR", dir.path())
        .env("QUIRE_LISTEN", addr.to_string())
        .env("QUIRE_TEST_ARGON2", "1")
        .env("RUST_LOG", "debug")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut up = false;
    for _ in 0..200 {
        if quire_server::cli::probe(&format!("http://{addr}"), Duration::from_millis(200))
            == Ok(200)
        {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(up, "the server did not come up");
    let host = format!("Host: {addr}\r\nConnection: close\r\n");
    let (status, _, _) = http(
        addr,
        &format!("GET /api/v1/auth/me HTTP/1.1\r\n{host}Authorization: Bearer {token}\r\n\r\n"),
    );
    assert_eq!(status, 200);
    let body = format!(r#"{{"email":"a@quire.invalid","password":"{OWNER_PASSWORD}"}}"#);
    let (status, head, _) = http(
        addr,
        &format!(
            "POST /api/v1/auth/login HTTP/1.1\r\n{host}Origin: http://{addr}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    assert_eq!(status, 200, "{head}");
    let cookie = head
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: quire_session="))
        .and_then(|v| v.split(';').next())
        .unwrap()
        .to_owned();
    let (status, _, _) = http(
        addr,
        &format!("GET /api/v1/auth/me HTTP/1.1\r\n{host}Cookie: quire_session={cookie}\r\n\r\n"),
    );
    assert_eq!(status, 200);
    let _ = server.kill();
    let out = server.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains(&format!("token_id={token_id}")), "{text}");
    assert!(text.contains("client_ip=127.0.0.1"), "{text}");
    assert!(
        text.contains("\"authorization\": Sensitive"),
        "headers are logged, redacted: {text}"
    );
    assert!(text.contains("\"cookie\": Sensitive"), "{text}");
    assert!(!text.contains(&token) && !text.contains("qpat_"), "{text}");
    assert!(!text.contains(&cookie), "{text}");
    assert!(!text.contains(OWNER_PASSWORD), "{text}");
}
