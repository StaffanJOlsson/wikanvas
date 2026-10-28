//! Personal access token route tests (owner: W2)
//!
//! W0b: the bearer branch of `Auth<S>` (classification of malformed, unknown, expired and revoked tokens, the scope
//! check) and the minimal `/tokens` routes.

use quire_api::{ApiToken, CreatedToken, ErrorCode, Me, Scope};
use serde_json::json;

use crate::common::{TestApp, assert_error};

#[tokio::test]
async fn me_with_token_reports_auth_token() {
    let app = TestApp::with_token(Scope::Read).await;
    let res = app.server.get("/api/v1/auth/me").await;
    res.assert_status_ok();
    let me: Me = res.json();
    assert_eq!(me.user.id.to_string(), app.owner());
    assert_eq!(me.workspace.name, "Test workspace");
    assert_eq!(me.auth.method.as_str(), "token");
    let token = me.auth.token.expect("auth.token for a token request");
    assert_eq!(token.scopes, vec![Scope::Read]);
    assert!(app.token.as_deref().unwrap().starts_with(&token.prefix));
    assert!(
        !res.text().contains(app.token.as_deref().unwrap()),
        "never the secret"
    );
}

#[tokio::test]
async fn me_describes_the_token_behind_100_newer_ones() {
    let app = TestApp::with_token(Scope::Read).await;
    // 100 newer expired tokens fill `tokens::list`'s page; `auth.token` must still describe the caller.
    app.exec(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100)
         INSERT INTO api_tokens (id, user_id, name, token_hash, prefix, created_via, created_at, expires_at)
         SELECT printf('%08d-0000-7000-8000-000000000000', i), ?1, 'old ' || i, randomblob(32), 'qpat_0000000',
                'admin', 4000000000000 + i, 1
         FROM n",
        vec![app.owner().to_owned()],
    )
    .await;
    let res = app.server.get("/api/v1/auth/me").await;
    res.assert_status_ok();
    let me: Me = res.json();
    let token = me.auth.token.expect("auth.token for a token request");
    assert!(app.token.as_deref().unwrap().starts_with(&token.prefix));
    assert_eq!(token.scopes, vec![Scope::Read]);
}

#[tokio::test]
async fn malformed_token_401_without_lookup() {
    let app = TestApp::with_owner().await;
    let client = app.client();
    let mut token = app.mint_token(app.owner(), Scope::Read).await;
    // Flip the last checksum character: the format check fails before any lookup.
    let last = token.pop().unwrap();
    token.push(if last == 'A' { 'B' } else { 'A' });
    for bad in [token.as_str(), "qpat_short"] {
        let res = client
            .get("/api/v1/auth/me")
            .add_header("authorization", format!("Bearer {bad}"))
            .await;
        let body = assert_error(&res, 401, ErrorCode::InvalidToken);
        assert!(body.error.details.is_none());
        assert_eq!(
            res.header("www-authenticate"),
            "Bearer error=\"invalid_token\""
        );
    }
}

#[tokio::test]
async fn unknown_token_401_without_reason() {
    let app = TestApp::with_owner().await;
    let token = quire_api::token::encode(&[9u8; 32]);
    let res = app
        .client()
        .get("/api/v1/auth/me")
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    let body = assert_error(&res, 401, ErrorCode::InvalidToken);
    assert!(body.error.details.is_none());
}

#[tokio::test]
async fn expired_token_401_reason_expired() {
    let app = TestApp::with_token(Scope::Read).await;
    app.exec("UPDATE api_tokens SET expires_at = 1", vec![])
        .await;
    let res = app.server.get("/api/v1/auth/me").await;
    let body = assert_error(&res, 401, ErrorCode::InvalidToken);
    assert_eq!(body.error.details, Some(json!({"reason": "expired"})));
}

#[tokio::test]
async fn revoked_token_rejected_on_next_request() {
    let app = TestApp::with_token(Scope::Admin).await;
    let list: Vec<ApiToken> = app.server.get("/api/v1/tokens").await.json();
    assert_eq!(list.len(), 1);
    let id = list[0].id;
    app.server
        .delete(&format!("/api/v1/tokens/{id}"))
        .await
        .assert_status_no_content();
    let res = app.server.get("/api/v1/auth/me").await;
    let body = assert_error(&res, 401, ErrorCode::InvalidToken);
    assert!(body.error.details.is_none(), "revoked looks like unknown");
}

#[tokio::test]
async fn wrong_scope_403_insufficient_scope() {
    let app = TestApp::with_token(Scope::Read).await;
    let res = app
        .server
        .post("/api/v1/collections")
        .json(&json!({"name": "Nope"}))
        .await;
    let body = assert_error(&res, 403, ErrorCode::InsufficientScope);
    assert_eq!(
        body.error.details,
        Some(json!({"required": "write", "granted": ["read"]}))
    );
    assert_eq!(
        res.header("www-authenticate"),
        "Bearer error=\"insufficient_scope\", scope=\"write\""
    );
    let res = app.server.get("/api/v1/tokens").await;
    assert_error(&res, 403, ErrorCode::InsufficientScope);
}

#[tokio::test]
async fn scope_checked_before_the_body() {
    // Auth<S> is the first extractor: a read token gets 403, not 415 or 422, for a body that would not parse.
    let app = TestApp::with_token(Scope::Read).await;
    let res = app
        .server
        .post("/api/v1/collections")
        .text("not json")
        .await;
    assert_error(&res, 403, ErrorCode::InsufficientScope);
}

#[tokio::test]
async fn create_shows_secret_once_list_never() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "ci", "scopes": ["write"], "expires_in_days": 7}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let created: CreatedToken = res.json();
    assert!(quire_api::token::check(&created.token).is_ok());
    assert_eq!(created.api_token.scopes, vec![Scope::Read, Scope::Write]);
    assert_eq!(created.api_token.created_via.as_str(), "web");
    let listed = app.server.get("/api/v1/tokens").await;
    listed.assert_status_ok();
    assert!(!listed.text().contains(&created.token));
    let tokens: Vec<ApiToken> = listed.json();
    assert_eq!(tokens.len(), 1);
    // The new token works and carries its scopes.
    let me = app
        .token_client(&created.token)
        .get("/api/v1/auth/me")
        .await;
    me.assert_status_ok();
}

#[tokio::test]
async fn session_admin_or_never_needs_password() {
    let app = TestApp::with_owner().await;
    for body in [
        json!({"name": "a", "scopes": ["admin"]}),
        json!({"name": "a", "scopes": ["read"], "expires_in_days": null}),
        json!({"name": "a", "scopes": ["admin"], "password": "wrong password!"}),
    ] {
        let res = app.server.post("/api/v1/tokens").json(&body).await;
        let err = assert_error(&res, 422, ErrorCode::Validation);
        assert_eq!(
            err.error.details,
            Some(json!({"field": "password"})),
            "{body}"
        );
    }
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "a", "scopes": ["admin"], "password": crate::common::OWNER_PASSWORD}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn token_cannot_mint_admin_403() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "x", "scopes": ["admin"]}))
        .await;
    assert_error(&res, 403, ErrorCode::SessionRequired);
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "x", "scopes": ["write"], "expires_in_days": 1}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let created: CreatedToken = res.json();
    assert_eq!(created.api_token.created_via.as_str(), "api");
    assert!(created.api_token.parent_token_id.is_some());
}

#[tokio::test]
async fn token_cannot_mint_longer_lived_token() {
    let app = TestApp::with_token(Scope::Admin).await;
    // The calling token expires in 90 days.
    for body in [
        json!({"name": "x", "scopes": ["read"], "expires_in_days": 365}),
        json!({"name": "x", "scopes": ["read"], "expires_in_days": null}),
    ] {
        let res = app.server.post("/api/v1/tokens").json(&body).await;
        let err = assert_error(&res, 422, ErrorCode::Validation);
        assert_eq!(
            err.error.details,
            Some(json!({"field": "expires_in_days"})),
            "{body}"
        );
    }
}

#[tokio::test]
async fn current_revokes_itself() {
    let app = TestApp::with_token(Scope::Read).await;
    app.server
        .delete("/api/v1/tokens/current")
        .await
        .assert_status_no_content();
    assert_error(
        &app.server.get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
}

#[tokio::test]
async fn revoke_unknown_token_404() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .delete(&format!("/api/v1/tokens/{}", uuid::Uuid::now_v7()))
        .await;
    assert_error(&res, 404, ErrorCode::NotFound);
    let res = app.server.delete("/api/v1/tokens/not-a-uuid").await;
    assert_error(&res, 404, ErrorCode::NotFound);
}
