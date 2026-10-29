//! Accounts and invite links over HTTP (D148): `GET /users`, `PATCH /users/{id}` with the account rules, the invite
//! lifecycle (create, list, look up, accept once, expire, revoke), disabling that ends sessions and tokens at once,
//! `config.multi_user`, and the accept's limits and CSRF rules.

use std::time::Duration;

use axum_test::TestResponse;
use serde_json::{Value, json};
use wikanvas_api::{
    CreatedInvite, ErrorCode, Invite, InviteLookup, InviteState, Me, Role, Scope, UserInfo,
};

use crate::common::{PEER, TestApp, USER_PASSWORD, assert_error, cookie_from};

const PASSWORD: &str = "a new member's password";

/// `POST /invites` by `client` for `role`.
async fn create(client: &axum_test::TestServer, body: Value) -> TestResponse {
    client.post("/api/v1/invites").json(&body).await
}

async fn invite(app: &TestApp, role: &str) -> CreatedInvite {
    let res = create(&app.server, json!({"role": role})).await;
    assert_eq!(res.status_code(), 201, "{}", res.text());
    res.json()
}

/// The token of an invite link: its last path segment.
fn token(created: &CreatedInvite) -> String {
    created.url.rsplit('/').next().unwrap().to_owned()
}

async fn lookup(app: &TestApp, token: &str) -> TestResponse {
    app.anon_client()
        .get(&format!("/api/v1/invites/{token}"))
        .await
}

/// `POST /invites/{token}/accept` from `peer`, as a browser on the same origin.
async fn accept(app: &TestApp, token: &str, email: &str, peer: &str) -> TestResponse {
    app.anon_client()
        .post(&format!("/api/v1/invites/{token}/accept"))
        .add_header(PEER, peer.to_owned())
        .json(&json!({"name": "New Person", "email": email, "password": PASSWORD}))
        .await
}

async fn users(app: &TestApp) -> Vec<UserInfo> {
    let res = app.server.get("/api/v1/users").await;
    res.assert_status_ok();
    res.json()
}

async fn multi_user(app: &TestApp) -> bool {
    let config: Value = app.client().get("/api/v1/config").await.json();
    config["multi_user"].as_bool().unwrap()
}

async fn patch(client: &axum_test::TestServer, id: &str, body: Value) -> TestResponse {
    client
        .patch(&format!("/api/v1/users/{id}"))
        .json(&body)
        .await
}

#[tokio::test]
async fn an_invite_is_made_looked_up_and_accepted_once() {
    let app = TestApp::with_owner().await;
    assert!(!multi_user(&app).await);
    let created = invite(&app, "member").await;
    assert_eq!(created.invite.role, Role::Member);
    assert_eq!(
        created.invite.created_by.map(|u| u.to_string()).as_deref(),
        Some(app.owner())
    );
    let t = token(&created);
    assert!(
        created
            .url
            .starts_with("http://localhost:8080/invite/qinv_"),
        "{}",
        created.url
    );
    assert!(wikanvas_api::token::check_invite(&t).is_ok());
    let days = (created.invite.expires_at.0 - created.invite.created_at.0) / 86_400_000;
    assert_eq!(days, 7);
    // The list has it, never with its token or link.
    let res = app.server.get("/api/v1/invites").await;
    res.assert_status_ok();
    assert!(!res.text().contains(&t));
    let listed: Vec<Invite> = res.json();
    assert_eq!(listed, std::slice::from_ref(&created.invite));
    // The public lookup: valid, the workspace and the role, never who made it.
    let res = lookup(&app, &t).await;
    res.assert_status_ok();
    let body: Value = res.json();
    assert!(body.get("created_by").is_none(), "{body}");
    assert!(!res.text().contains(app.owner()), "{body}");
    let seen: InviteLookup = res.json();
    assert_eq!(
        (
            seen.valid,
            seen.state,
            seen.role,
            seen.workspace_name.as_str()
        ),
        (true, InviteState::Pending, Role::Member, "Test workspace")
    );
    // Accepting creates the member and signs them in.
    let res = accept(&app, &t, " Bea@Wikanvas.Invalid ", "192.0.2.1:1").await;
    assert_eq!(res.status_code(), 201, "{}", res.text());
    let created_user: Value = res.json();
    assert_eq!(created_user["user"]["role"], "member");
    assert_eq!(created_user["user"]["email"], "Bea@Wikanvas.Invalid");
    let cookie = cookie_from(&res, "wikanvas_session").expect("a session cookie");
    let set_cookie = res.header("set-cookie").to_str().unwrap().to_owned();
    assert!(
        set_cookie.contains("HttpOnly") && !set_cookie.contains("Max-Age"),
        "{set_cookie}"
    );
    let me: Me = app
        .cookie_client(&cookie)
        .get("/api/v1/auth/me")
        .await
        .json();
    assert_eq!(me.user.role, Role::Member);
    assert!(!me.must_change_password);
    assert!(multi_user(&app).await);
    // The new member can sign in with the password too.
    let login = app
        .anon_client()
        .post("/api/v1/auth/login")
        .json(&json!({"email": "bea@wikanvas.invalid", "password": PASSWORD}))
        .await;
    login.assert_status_ok();
    // Single use: a second accept is 404, the lookup says used, the list no longer has it.
    let again = accept(&app, &t, "other@wikanvas.invalid", "192.0.2.1:2").await;
    let err = assert_error(&again, 404, ErrorCode::NotFound);
    assert!(err.error.message.contains("used"), "{}", err.error.message);
    let seen: InviteLookup = lookup(&app, &t).await.json();
    assert_eq!((seen.valid, seen.state), (false, InviteState::Used));
    let listed: Vec<Invite> = app.server.get("/api/v1/invites").await.json();
    assert!(listed.is_empty());
    assert_eq!(users(&app).await.len(), 2);
}

#[tokio::test]
async fn an_invite_expires() {
    let mut app = TestApp::with_fake_clock().await;
    app.sign_in_owner().await;
    let res = create(&app.server, json!({"role": "viewer", "expires_in_days": 1})).await;
    assert_eq!(res.status_code(), 201, "{}", res.text());
    let created: CreatedInvite = res.json();
    let t = token(&created);
    for (days, field) in [(0, "expires_in_days"), (31, "expires_in_days")] {
        let res = create(
            &app.server,
            json!({"role": "viewer", "expires_in_days": days}),
        )
        .await;
        let err = assert_error(&res, 422, ErrorCode::Validation);
        assert_eq!(err.error.details, Some(json!({ "field": field })));
    }
    let res = create(&app.server, json!({"role": "owner"})).await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "role"})));
    app.state.clock.advance(Duration::from_secs(86_400));
    let seen: InviteLookup = lookup(&app, &t).await.json();
    assert_eq!((seen.valid, seen.state), (false, InviteState::Expired));
    let res = accept(&app, &t, "late@wikanvas.invalid", "192.0.2.2:1").await;
    let err = assert_error(&res, 404, ErrorCode::NotFound);
    assert!(
        err.error.message.contains("expired"),
        "{}",
        err.error.message
    );
}

#[tokio::test]
async fn a_revoked_invite_stops_working() {
    let app = TestApp::with_owner().await;
    let created = invite(&app, "member").await;
    let t = token(&created);
    let id = created.invite.id;
    app.server
        .delete(&format!("/api/v1/invites/{id}"))
        .await
        .assert_status(axum::http::StatusCode::NO_CONTENT);
    let seen: InviteLookup = lookup(&app, &t).await.json();
    assert_eq!((seen.valid, seen.state), (false, InviteState::Revoked));
    let res = accept(&app, &t, "x@wikanvas.invalid", "192.0.2.3:1").await;
    let err = assert_error(&res, 404, ErrorCode::NotFound);
    assert!(
        err.error.message.contains("revoked"),
        "{}",
        err.error.message
    );
    let again = app.server.delete(&format!("/api/v1/invites/{id}")).await;
    assert_error(&again, 404, ErrorCode::NotFound);
    let listed: Vec<Invite> = app.server.get("/api/v1/invites").await.json();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn accept_validates_and_keeps_emails_unique() {
    let app = TestApp::with_owner().await;
    let created = invite(&app, "viewer").await;
    let t = token(&created);
    let peer = "192.0.2.4:1";
    let post = |body: Value| {
        app.anon_client()
            .post(&format!("/api/v1/invites/{t}/accept"))
            .add_header(PEER, peer.to_owned())
            .json(&body)
    };
    for (body, field) in [
        (
            json!({"name": "N", "email": "nope", "password": PASSWORD}),
            "email",
        ),
        (
            json!({"name": " ", "email": "n@wikanvas.invalid", "password": PASSWORD}),
            "name",
        ),
        (
            json!({"name": "N", "email": "n@wikanvas.invalid", "password": "short"}),
            "password",
        ),
    ] {
        let err = assert_error(&post(body).await, 422, ErrorCode::Validation);
        assert_eq!(err.error.details, Some(json!({ "field": field })));
    }
    // The owner's email, in another case, is taken; the invite stays pending.
    let res =
        post(json!({"name": "N", "email": "OWNER@wikanvas.invalid", "password": PASSWORD})).await;
    let err = assert_error(&res, 409, ErrorCode::Conflict);
    assert_eq!(err.error.details, Some(json!({"field": "email"})));
    let seen: InviteLookup = lookup(&app, &t).await.json();
    assert!(seen.valid);
    // An unknown token, or one that is not an invite's, is 404.
    for bad in [
        wikanvas_api::token::encode_invite(&[3; 32]),
        wikanvas_api::token::encode(&[3; 32]),
    ] {
        assert_error(&lookup(&app, &bad).await, 404, ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn accept_is_limited_like_login_and_checked_like_setup() {
    let app = TestApp::with_fake_clock().await;
    let owner = app.insert_owner().await;
    let session = app.insert_session(&owner).await;
    let created: CreatedInvite = create(&app.cookie_client(&session), json!({"role": "member"}))
        .await
        .json();
    let t = token(&created);
    let bad = wikanvas_api::token::encode_invite(&[5; 32]);
    for _ in 0..5 {
        let res = accept(&app, &bad, "a@wikanvas.invalid", "203.0.113.20:1").await;
        assert_error(&res, 404, ErrorCode::NotFound);
    }
    // The sixth attempt from that address waits, even with a valid invite.
    let res = accept(&app, &t, "a@wikanvas.invalid", "203.0.113.20:2").await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "requests"})));
    assert!(
        res.header("retry-after")
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            >= 1
    );
    // Login from there is limited too (one budget), another address is not.
    let login = app
        .client()
        .post("/api/v1/auth/login")
        .add_header("host", "localhost")
        .add_header("sec-fetch-site", "same-origin")
        .add_header(PEER, "203.0.113.20:3")
        .json(&json!({"email": "x@wikanvas.invalid", "password": "whatever-password"}))
        .await;
    assert_error(&login, 429, ErrorCode::RateLimited);
    // CSRF: a cross-site form post is refused before anything; so is a foreign Host.
    let cross = app
        .client()
        .post(&format!("/api/v1/invites/{t}/accept"))
        .add_header("host", "localhost")
        .add_header("sec-fetch-site", "cross-site")
        .add_header("origin", "https://evil.example")
        .add_header(PEER, "203.0.113.21:1")
        .json(&json!({"name": "N", "email": "a@wikanvas.invalid", "password": PASSWORD}))
        .await;
    assert_error(&cross, 403, ErrorCode::CsrfRejected);
    let host = app
        .client()
        .post(&format!("/api/v1/invites/{t}/accept"))
        .add_header("host", "evil.example")
        .add_header("sec-fetch-site", "same-origin")
        .add_header(PEER, "203.0.113.21:2")
        .json(&json!({"name": "N", "email": "a@wikanvas.invalid", "password": PASSWORD}))
        .await;
    assert_error(&host, 421, ErrorCode::HostNotAllowed);
    let ok = accept(&app, &t, "a@wikanvas.invalid", "203.0.113.22:1").await;
    assert_eq!(ok.status_code(), 201, "{}", ok.text());
    // A minute later the first address may try again.
    app.state.clock.advance(Duration::from_secs(60));
    let res = accept(&app, &bad, "b@wikanvas.invalid", "203.0.113.20:4").await;
    assert_error(&res, 404, ErrorCode::NotFound);
    // Lookups of unknown tokens count per address like failed bearer authentications.
    for i in 0..30u8 {
        let res = app
            .client()
            .get(&format!(
                "/api/v1/invites/{}",
                wikanvas_api::token::encode_invite(&[i; 32])
            ))
            .add_header("host", "localhost")
            .add_header(PEER, "203.0.113.23:1")
            .await;
        assert_error(&res, 404, ErrorCode::NotFound);
    }
    let lookup_from = |t: String| {
        app.client()
            .get(&format!("/api/v1/invites/{t}"))
            .add_header("host", "localhost")
            .add_header(PEER, "203.0.113.23:2")
    };
    let res = lookup_from(wikanvas_api::token::encode_invite(&[99; 32])).await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "auth_failures"})));
    // A real invite is still shown to that address, as valid tokens are still served.
    lookup_from(t.clone()).await.assert_status_ok();
}

#[tokio::test]
async fn admins_manage_members_and_viewers_and_only_the_owner_manages_admins() {
    let app = TestApp::with_owner().await;
    let admin = app.insert_user(Role::Admin).await;
    let member = app.insert_user(Role::Member).await;
    let viewer = app.insert_user(Role::Viewer).await;
    let other_admin = app
        .insert_user_with(Role::Admin, "admin2@wikanvas.invalid")
        .await;
    let as_admin = app.cookie_client(&app.insert_session(&admin).await);
    let owner = app.owner().to_owned();
    let forbidden = |res: TestResponse| {
        assert_error(&res, 403, ErrorCode::Forbidden);
    };
    // The owner's role never changes and the owner cannot be disabled.
    forbidden(patch(&app.server, &owner, json!({"role": "admin"})).await);
    forbidden(patch(&app.server, &owner, json!({"disabled": true})).await);
    forbidden(patch(&as_admin, &owner, json!({"disabled": true})).await);
    // Nobody changes their own role or disables themselves.
    forbidden(patch(&as_admin, &admin, json!({"role": "member"})).await);
    forbidden(patch(&as_admin, &admin, json!({"disabled": true})).await);
    // Only the owner makes or unmakes an admin, or disables one.
    forbidden(patch(&as_admin, &member, json!({"role": "admin"})).await);
    forbidden(patch(&as_admin, &other_admin, json!({"role": "viewer"})).await);
    forbidden(patch(&as_admin, &other_admin, json!({"disabled": true})).await);
    let err = assert_error(
        &patch(&app.server, &member, json!({"role": "owner"})).await,
        422,
        ErrorCode::Validation,
    );
    assert_eq!(err.error.details, Some(json!({"field": "role"})));
    // An admin manages members and viewers.
    let res = patch(&as_admin, &viewer, json!({"role": "member"})).await;
    res.assert_status_ok();
    let changed: UserInfo = res.json();
    assert_eq!(changed.role, Role::Member);
    // The owner makes an admin, and unmakes one.
    let res = patch(&app.server, &member, json!({"role": "admin"})).await;
    res.assert_status_ok();
    assert_eq!(res.json::<UserInfo>().role, Role::Admin);
    let res = patch(&app.server, &other_admin, json!({"role": "viewer"})).await;
    res.assert_status_ok();
    // Unknown and malformed ids are 404.
    assert_error(
        &patch(
            &app.server,
            &uuid::Uuid::now_v7().to_string(),
            json!({"role": "viewer"}),
        )
        .await,
        404,
        ErrorCode::NotFound,
    );
    assert_error(
        &patch(&app.server, "nope", json!({"role": "viewer"})).await,
        404,
        ErrorCode::NotFound,
    );
    // Invites: an admin invites members and viewers, never an admin; the owner may.
    let res = create(&as_admin, json!({"role": "admin"})).await;
    forbidden(res);
    let res = create(&as_admin, json!({"role": "viewer", "note": "For Alex"})).await;
    assert_eq!(res.status_code(), 201, "{}", res.text());
    let res = create(&app.server, json!({"role": "admin"})).await;
    assert_eq!(res.status_code(), 201, "{}", res.text());
    // The list, the owner first.
    let list = users(&app).await;
    assert_eq!(list[0].role, Role::Owner);
    assert_eq!(list.len(), 5);
    // A member or viewer may not list accounts or invites; a write token cannot either.
    let as_member = app.cookie_client(&app.insert_session(&viewer).await);
    assert_error(
        &as_member.get("/api/v1/users").await,
        403,
        ErrorCode::Forbidden,
    );
    let write = app.token_client(&app.mint_token(&owner, Scope::Write).await);
    assert_error(
        &write.get("/api/v1/invites").await,
        403,
        ErrorCode::InsufficientScope,
    );
}

#[tokio::test]
async fn disabling_ends_sessions_and_tokens_at_once() {
    let app = TestApp::with_owner().await;
    let member = app.insert_user(Role::Member).await;
    let session = app.insert_session(&member).await;
    let as_member = app.cookie_client(&session);
    let token = app.token_client(&app.mint_token(&member, Scope::Write).await);
    as_member.get("/api/v1/auth/me").await.assert_status_ok();
    token.get("/api/v1/auth/me").await.assert_status_ok();
    assert!(multi_user(&app).await);
    let res = patch(&app.server, &member, json!({"disabled": true})).await;
    res.assert_status_ok();
    let disabled: UserInfo = res.json();
    assert!(disabled.disabled_at.is_some());
    assert!(!multi_user(&app).await);
    assert_error(
        &as_member.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_error(
        &token.get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
    // The rows are gone or revoked, not only refused.
    let (sessions, live_tokens): (i64, i64) = app
        .state
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT (SELECT count(*) FROM sessions WHERE user_id = ?1),
                        (SELECT count(*) FROM api_tokens WHERE user_id = ?1 AND revoked_at IS NULL)",
                [&member],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!((sessions, live_tokens), (0, 0));
    // Login refuses a disabled account; once enabled it signs in again, and the old session stays ended.
    let login = |email: &'static str| {
        app.anon_client()
            .post("/api/v1/auth/login")
            .json(&json!({"email": email, "password": USER_PASSWORD}))
    };
    assert_error(
        &login("member@wikanvas.invalid").await,
        401,
        ErrorCode::InvalidCredentials,
    );
    let id = disabled.id.to_string();
    patch(&app.server, &id, json!({"disabled": false}))
        .await
        .assert_status_ok();
    login("member@wikanvas.invalid").await.assert_status_ok();
    assert_error(
        &as_member.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
}
