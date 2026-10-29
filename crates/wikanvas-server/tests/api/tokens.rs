//! Personal access token route tests (owner: W2)
//!
//! The bearer branch of `Auth<S>` (classification of malformed, unknown, expired, revoked and disabled tokens, the
//! scope check), the `/tokens` routes with their caps, revocation with children, the 100-token limit and the
//! once-a-minute last-use write.

use std::time::Duration;

use wikanvas_api::tokens::NewToken;
use wikanvas_api::{ApiToken, CreatedToken, ErrorBody, ErrorCode, Me, Scope};
use wikanvas_store::tokens::{CreatedVia, TokenLookup};
use serde_json::json;

use crate::common::{OWNER_PASSWORD, TestApp, assert_error};

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
    // From here any token lookup fails loudly: the table it reads is gone.
    app.exec("ALTER TABLE api_tokens RENAME TO api_tokens_hidden", vec![])
        .await;
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
    // A well-formed token is looked up, and the lookup hits the missing table: the malformed ones never queried.
    let well_formed = wikanvas_api::token::encode(&[3u8; 32]);
    let res = client
        .get("/api/v1/auth/me")
        .add_header("authorization", format!("Bearer {well_formed}"))
        .await;
    assert_error(&res, 500, ErrorCode::Internal);
}

#[tokio::test]
async fn unknown_token_401_without_reason() {
    let app = TestApp::with_owner().await;
    let token = wikanvas_api::token::encode(&[9u8; 32]);
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
    assert!(wikanvas_api::token::check(&created.token).is_ok());
    assert_eq!(created.api_token.scopes, vec![Scope::Read, Scope::Write]);
    assert_eq!(created.api_token.created_via.as_str(), "web");
    let listed = app.server.get("/api/v1/tokens").await;
    listed.assert_status_ok();
    assert!(!listed.text().contains(&created.token));
    let hash: String = wikanvas_store::tokens::hash(&created.token)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        !listed.text().to_lowercase().contains(&hash),
        "never the hash"
    );
    let me = app
        .token_client(&created.token)
        .get("/api/v1/auth/me")
        .await;
    assert!(!me.text().contains(&created.token) && !me.text().to_lowercase().contains(&hash));
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

#[tokio::test]
async fn scopes_expanded_and_capped_by_caller() {
    let app = TestApp::with_owner().await;
    // A session may grant any scope; scopes are stored expanded.
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "w", "scopes": ["write"], "expires_in_days": 30}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let write: CreatedToken = res.json();
    assert_eq!(write.api_token.scopes, vec![Scope::Read, Scope::Write]);
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "a", "scopes": ["read", "admin"], "expires_in_days": 30, "password": OWNER_PASSWORD}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let admin: CreatedToken = res.json();
    assert_eq!(
        admin.api_token.scopes,
        vec![Scope::Read, Scope::Write, Scope::Admin]
    );
    // An admin token grants write but never admin.
    let res = app
        .token_client(&admin.token)
        .post("/api/v1/tokens")
        .json(&json!({"name": "c", "scopes": ["write"], "expires_in_days": 1}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    // A write token cannot reach POST /tokens at all (admin scope), and below the route the store caps a token
    // parent: a scope above its own is 422 `scopes`, admin is 403 `session_required`.
    let res = app
        .token_client(&write.token)
        .post("/api/v1/tokens")
        .json(&json!({"name": "x", "scopes": ["admin"]}))
        .await;
    assert_error(&res, 403, ErrorCode::InsufficientScope);
    let read = app.mint_token(app.owner(), Scope::Read).await;
    let owner = app.owner().to_owned();
    let now = app.state.now();
    let err = app
        .state
        .db
        .write(move |tx| {
            let TokenLookup::Live(parent) = wikanvas_store::tokens::lookup(tx, &read, now)? else {
                panic!("live");
            };
            let new = |scope| NewToken {
                name: "child".into(),
                scopes: vec![scope],
                expires_in_days: Some(Some(1)),
                password: None,
            };
            let above = wikanvas_store::tokens::create(
                tx,
                &owner,
                &new(Scope::Write),
                CreatedVia::Api,
                Some(&parent),
                now,
            )
            .unwrap_err();
            let admin = wikanvas_store::tokens::create(
                tx,
                &owner,
                &new(Scope::Admin),
                CreatedVia::Api,
                Some(&parent),
                now,
            )
            .unwrap_err();
            Ok((above, admin))
        })
        .await
        .unwrap();
    assert_eq!(err.0.code, ErrorCode::Validation);
    assert_eq!(err.0.details, Some(json!({"field": "scopes"})));
    assert_eq!(err.1.code, ErrorCode::SessionRequired);
}

fn same_rejection(a: &ErrorBody, b: &ErrorBody) {
    assert_eq!(a.error, b.error, "the same code, message and details");
}

#[tokio::test]
async fn lookup_classifies_expired_revoked_disabled() {
    let app = TestApp::with_owner().await;
    let client = app.client();
    let me = |token: String| {
        client
            .get("/api/v1/auth/me")
            .add_header("authorization", format!("Bearer {token}"))
    };
    let unknown = assert_error(
        &me(wikanvas_api::token::encode(&[5u8; 32])).await,
        401,
        ErrorCode::InvalidToken,
    );
    let revoked = app.mint_token(app.owner(), Scope::Read).await;
    app.exec("UPDATE api_tokens SET revoked_at = 1", vec![])
        .await;
    let revoked = assert_error(&me(revoked).await, 401, ErrorCode::InvalidToken);
    same_rejection(&unknown, &revoked);
    let expired = app.mint_token(app.owner(), Scope::Read).await;
    app.exec(
        "UPDATE api_tokens SET expires_at = 1 WHERE revoked_at IS NULL",
        vec![],
    )
    .await;
    let expired = assert_error(&me(expired).await, 401, ErrorCode::InvalidToken);
    assert_eq!(expired.error.details, Some(json!({"reason": "expired"})));
    let live = app.mint_token(app.owner(), Scope::Read).await;
    me(live.clone()).await.assert_status_ok();
    app.exec("UPDATE users SET disabled_at = 1", vec![]).await;
    let disabled = assert_error(&me(live).await, 401, ErrorCode::InvalidToken);
    same_rejection(&unknown, &disabled);
}

#[tokio::test]
async fn revoking_parent_revokes_children() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "parent", "scopes": ["admin"], "expires_in_days": 10, "password": OWNER_PASSWORD}))
        .await;
    let parent: CreatedToken = res.json();
    let mut children = Vec::new();
    for name in ["a", "b"] {
        let res = app
            .token_client(&parent.token)
            .post("/api/v1/tokens")
            .json(&json!({"name": name, "scopes": ["write"], "expires_in_days": 5}))
            .await;
        res.assert_status(axum::http::StatusCode::CREATED);
        let child: CreatedToken = res.json();
        assert_eq!(child.api_token.parent_token_id, Some(parent.api_token.id));
        children.push(child);
    }
    let sibling = app.mint_token(app.owner(), Scope::Read).await;
    app.server
        .delete(&format!("/api/v1/tokens/{}", parent.api_token.id))
        .await
        .assert_status_no_content();
    for t in children.iter().map(|c| &c.token).chain([&parent.token]) {
        assert_error(
            &app.token_client(t).get("/api/v1/auth/me").await,
            401,
            ErrorCode::InvalidToken,
        );
    }
    app.token_client(&sibling)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn limit_100_live_tokens_422() {
    let app = TestApp::with_owner().await;
    for _ in 0..99 {
        app.mint_token(app.owner(), Scope::Read).await;
    }
    // An expired token does not count.
    app.exec(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, prefix, created_via, created_at, expires_at)
         VALUES ('00000000-0000-7000-8000-000000000001', ?1, 'old', randomblob(32), 'qpat_0000000', 'admin', 1, 2)",
        vec![app.owner().to_owned()],
    )
    .await;
    let body = json!({"name": "100th", "scopes": ["read"], "expires_in_days": 1});
    app.server
        .post("/api/v1/tokens")
        .json(&body)
        .await
        .assert_status(axum::http::StatusCode::CREATED);
    let res = app.server.post("/api/v1/tokens").json(&body).await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "tokens"})));
    // Every live token is listed even with expired ones around.
    let listed: Vec<ApiToken> = app.server.get("/api/v1/tokens").await.json();
    assert_eq!(listed.len(), 100);
    assert!(listed.iter().all(|t| t.name != "old"));
}

async fn last_used(app: &TestApp) -> (Option<i64>, Option<String>) {
    app.state
        .db
        .read(|c| {
            Ok(c.query_row(
                "SELECT last_used_at, last_used_ip FROM api_tokens",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
}

/// Waits for the spawned last-use write to show `expect`.
async fn wait_last_used(app: &TestApp, expect: i64) {
    for _ in 0..200 {
        if last_used(app).await.0 == Some(expect) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "last_used_at never became {expect}: {:?}",
        last_used(app).await
    );
}

#[tokio::test]
async fn last_used_written_at_most_once_per_minute() {
    let mut app = TestApp::with_fake_clock().await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Read).await;
    app.server = app.token_client(&token);
    assert_eq!(last_used(&app).await, (None, None));
    let t0 = app.state.now();
    let get = || {
        app.server
            .get("/api/v1/auth/me")
            .add_header(crate::common::PEER, "192.0.2.44:9")
    };
    get().await.assert_status_ok();
    wait_last_used(&app, t0).await;
    assert_eq!(last_used(&app).await.1.as_deref(), Some("192.0.2.44"));
    app.state.clock.advance(Duration::from_secs(59));
    get().await.assert_status_ok();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        last_used(&app).await.0,
        Some(t0),
        "no second write within the minute"
    );
    app.state.clock.advance(Duration::from_secs(1));
    get().await.assert_status_ok();
    wait_last_used(&app, t0 + 60_000).await;
}

#[tokio::test]
async fn password_change_keeps_tokens() {
    let app = TestApp::with_owner().await;
    let token = app.mint_token(app.owner(), Scope::Read).await;
    app.server
        .post("/api/v1/auth/password")
        .json(&json!({"current": OWNER_PASSWORD, "new": "another long password"}))
        .await
        .assert_status_no_content();
    let me: Me = app.token_client(&token).get("/api/v1/auth/me").await.json();
    assert_eq!(me.auth.method.as_str(), "token");
}
