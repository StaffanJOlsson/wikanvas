//! Setup, login, session and cookie tests (owner: W2)
//!
//! Setup with its code, rate limit and seed; login cookies per request scheme; the login limiters on the fake clock;
//! session renewal; password change and reset; `Me` for tokens; the session-only email change; `others=true`; and the
//! limits of `GET /config`.

use std::future::IntoFuture;
use std::time::Duration;

use axum::http::StatusCode;
use quire_api::{
    Config as ApiConfig, ErrorCode, Me, Scope, SessionInfo, SetupStatus, UserEnvelope,
};
use serde_json::json;

use crate::common::{
    OWNER_EMAIL, OWNER_PASSWORD, PEER, TestApp, admin, assert_error, cookie_from, set_cookies,
    stdout_json,
};

fn setup_body(code: &str) -> serde_json::Value {
    json!({
        "code": code,
        "workspace_name": "Acme",
        "name": "Ada",
        "email": "ada@quire.invalid",
        "password": "a long enough password"
    })
}

#[tokio::test]
async fn setup_requires_code() {
    let app = TestApp::new().await;
    let client = app.anon_client();
    let code = app.state.setup_code.to_string();
    assert_eq!(code.len(), 14, "XXXX-XXXX-XXXX");
    assert!(
        app.state
            .setup_message()
            .contains(&format!("/setup code {code}"))
    );
    for wrong in ["", "AAAA-AAAA-AAAA"] {
        let res = client.post("/api/v1/setup").json(&setup_body(wrong)).await;
        assert_error(&res, 401, ErrorCode::InvalidCredentials);
    }
    let res = client
        .post("/api/v1/setup")
        .json(&setup_body(&code.to_lowercase()))
        .await;
    res.assert_status(StatusCode::CREATED);
    let created: UserEnvelope = res.json();
    assert_eq!(created.user.email, "ada@quire.invalid");
    assert_eq!(created.user.role.as_str(), "owner");
    let cookie = cookie_from(&res, "quire_session").expect("signed in");
    let me = app
        .client()
        .get("/api/v1/auth/me")
        .add_header("cookie", format!("quire_session={cookie}"))
        .await;
    me.assert_status_ok();
    let me: Me = me.json();
    assert_eq!(me.workspace.name, "Acme");
    assert!(!me.must_change_password);
    let status: SetupStatus = client.get("/api/v1/setup").await.json();
    assert!(!status.needs_setup);
    // Once done, even the right code gets 409.
    let res = client.post("/api/v1/setup").json(&setup_body(&code)).await;
    assert_error(&res, 409, ErrorCode::AlreadySetup);
}

#[tokio::test]
async fn setup_validates_its_fields() {
    let app = TestApp::new().await;
    let code = app.state.setup_code.to_string();
    for (field, value) in [
        ("email", json!("nope")),
        ("name", json!(" ")),
        ("workspace_name", json!("")),
        ("password", json!("short")),
    ] {
        let mut body = setup_body(&code);
        body[field] = value;
        let res = app
            .anon_client()
            .post("/api/v1/setup")
            .add_header(PEER, format!("192.0.2.{}:1", field.len()))
            .json(&body)
            .await;
        let err = assert_error(&res, 422, ErrorCode::Validation);
        assert_eq!(err.error.details, Some(json!({ "field": field })));
    }
    assert!(!app.state.owner_exists().await.unwrap());
}

#[tokio::test]
async fn setup_only_once_under_concurrency() {
    let app = TestApp::new().await;
    let code = app.state.setup_code.to_string();
    let client = app.anon_client();
    let requests = (0..6).map(|i| {
        client
            .post("/api/v1/setup")
            .add_header(PEER, format!("198.51.100.{i}:4000"))
            .json(&setup_body(&code))
            .into_future()
    });
    let results = futures_util::future::join_all(requests).await;
    let created = results.iter().filter(|r| r.status_code() == 201).count();
    assert_eq!(created, 1, "exactly one setup succeeds");
    for res in results.iter().filter(|r| r.status_code() != 201) {
        assert_error(res, 409, ErrorCode::AlreadySetup);
    }
    let owners = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))?))
        .await
        .unwrap();
    assert_eq!(owners, 1);
    let collections = app
        .state
        .db
        .read(|c| {
            Ok(c.query_row("SELECT count(*) FROM collections", [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .unwrap();
    assert_eq!(collections, 1, "seeded once");
}

#[tokio::test]
async fn setup_required_409_before_setup() {
    let app = TestApp::new().await;
    let client = app.anon_client();
    for (method, path) in [
        ("GET", "/api/v1/auth/me"),
        ("GET", "/api/v1/tokens"),
        ("GET", "/api/v1/workspace"),
        ("POST", "/api/v1/auth/logout"),
    ] {
        let res = match method {
            "GET" => client.get(path).await,
            _ => client.post(path).json(&json!({})).await,
        };
        assert_error(&res, 409, ErrorCode::SetupRequired);
    }
    let status: SetupStatus = client.get("/api/v1/setup").await.json();
    assert!(status.needs_setup);
    let config: ApiConfig = client.get("/api/v1/config").await.json();
    assert!(config.setup_required);
    let res = client
        .post("/api/v1/setup")
        .json(&setup_body(&app.state.setup_code))
        .await;
    res.assert_status(StatusCode::CREATED);
    assert_error(
        &client.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    let config: ApiConfig = client.get("/api/v1/config").await.json();
    assert!(!config.setup_required);
}

#[tokio::test]
async fn setup_rate_limited_per_ip() {
    let app = TestApp::with_fake_clock().await;
    let client = app.anon_client();
    let attempt = |peer: &'static str| {
        client
            .post("/api/v1/setup")
            .add_header(PEER, peer)
            .json(&setup_body("WRONG-CODE-0000"))
    };
    for _ in 0..5 {
        assert_error(
            &attempt("203.0.113.7:1000").await,
            401,
            ErrorCode::InvalidCredentials,
        );
    }
    let res = attempt("203.0.113.7:1001").await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "requests"})));
    let retry: u64 = res.header("retry-after").to_str().unwrap().parse().unwrap();
    assert!((1..=12).contains(&retry), "{retry}");
    // Even the right code waits: the limiter answers first.
    let res = client
        .post("/api/v1/setup")
        .add_header(PEER, "203.0.113.7:1002")
        .json(&setup_body(&app.state.setup_code))
        .await;
    assert_error(&res, 429, ErrorCode::RateLimited);
    // Another address is not affected, and the first one recovers.
    assert_error(
        &attempt("203.0.113.8:1000").await,
        401,
        ErrorCode::InvalidCredentials,
    );
    app.state.clock.advance(Duration::from_secs(12));
    assert_error(
        &attempt("203.0.113.7:1000").await,
        401,
        ErrorCode::InvalidCredentials,
    );
}

#[tokio::test]
async fn setup_seeds_getting_started() {
    let app = TestApp::new().await;
    let res = app
        .anon_client()
        .post("/api/v1/setup")
        .json(&setup_body(&app.state.setup_code))
        .await;
    res.assert_status(StatusCode::CREATED);
    let (collection, items) = app
        .state
        .db
        .read(|c| {
            let collection = c.query_row(
                "SELECT name, color, icon, deleted_at IS NULL FROM collections",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?)),
            )?;
            let items = c
                .prepare(
                    "SELECT i.title, i.kind, p.title, i.published_at IS NOT NULL AND i.published_at = i.created_at,
                            (SELECT count(*) FROM doc_state d WHERE d.item_id = i.id)
                              + (SELECT count(*) FROM board_state b WHERE b.item_id = i.id)
                     FROM items i LEFT JOIN items p ON p.id = i.parent_id ORDER BY i.created_at, i.sort_key",
                )?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((collection, items))
        })
        .await
        .unwrap();
    assert_eq!(
        collection,
        (
            "Getting started".into(),
            "#3E63DD".into(),
            "rocket".into(),
            true
        )
    );
    let mut items = items;
    items.sort();
    let welcome = Some("Welcome to Quire".to_owned());
    assert_eq!(
        items,
        vec![
            (
                "Example board".into(),
                "board".into(),
                welcome.clone(),
                true,
                1
            ),
            ("Welcome to Quire".into(), "doc".into(), None, true, 1),
            ("Writing in Quire".into(), "doc".into(), welcome, true, 1),
        ]
    );
}

#[tokio::test]
async fn login_cookie_attributes() {
    let app = TestApp::with_config(|c| {
        c.trusted_proxies = vec![quire_server::config::IpNet::parse("172.31.240.2").unwrap()];
    })
    .await;
    app.insert_owner().await;
    // Plain HTTP: quire_session, HttpOnly, SameSite=Lax, Path=/, no Secure, no Max-Age.
    let res = app
        .login(OWNER_PASSWORD, false, Some("10.0.0.5:4000"))
        .await;
    res.assert_status_ok();
    let body: UserEnvelope = res.json();
    assert_eq!(body.user.email, OWNER_EMAIL);
    let cookies = set_cookies(&res);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    let c = &cookies[0];
    assert!(c.starts_with("quire_session="), "{c}");
    for attr in ["; HttpOnly", "; SameSite=Lax", "; Path=/"] {
        assert!(c.contains(attr), "{c} lacks {attr}");
    }
    assert!(!c.contains("Secure") && !c.contains("Max-Age"), "{c}");
    // remember: Max-Age of the 30-day window.
    let res = app.login(OWNER_PASSWORD, true, Some("10.0.0.6:4000")).await;
    let c = &set_cookies(&res)[0];
    assert!(c.contains("; Max-Age=2592000"), "{c}");
    // HTTPS through the trusted proxy: the __Host- name and Secure.
    let https = |peer: &'static str| {
        app.anon_client()
            .post("/api/v1/auth/login")
            .add_header(PEER, peer)
            .add_header("x-forwarded-proto", "https")
            .add_header("x-forwarded-for", "198.51.100.20")
            .json(&json!({"email": OWNER_EMAIL, "password": OWNER_PASSWORD}))
    };
    let res = https("172.31.240.2:5000").await;
    res.assert_status_ok();
    let c = &set_cookies(&res)[0];
    assert!(c.starts_with("__Host-quire_session="), "{c}");
    assert!(
        c.contains("; Secure") && c.contains("; Path=/") && !c.contains("Domain"),
        "{c}"
    );
    // The same headers from an untrusted peer count for nothing.
    let res = https("10.0.0.7:5000").await;
    let c = &set_cookies(&res)[0];
    assert!(
        c.starts_with("quire_session=") && !c.contains("Secure"),
        "{c}"
    );
}

#[tokio::test]
async fn cookie_read_by_request_scheme() {
    let app = TestApp::with_config(|c| {
        c.trusted_proxies = vec![quire_server::config::IpNet::parse("172.31.240.2").unwrap()];
    })
    .await;
    let owner = app.insert_owner().await;
    let session = app.insert_session(&owner).await;
    let me = |cookie: String, https: bool| {
        let mut req = app
            .client()
            .get("/api/v1/auth/me")
            .add_header("cookie", cookie);
        if https {
            req = req
                .add_header(PEER, "172.31.240.2:5000")
                .add_header("x-forwarded-proto", "https");
        }
        req
    };
    assert_eq!(
        me(format!("quire_session={session}"), false)
            .await
            .status_code(),
        200
    );
    assert_error(
        &me(format!("__Host-quire_session={session}"), false).await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_error(
        &me(format!("quire_session={session}"), true).await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_eq!(
        me(format!("__Host-quire_session={session}"), true)
            .await
            .status_code(),
        200
    );
}

#[tokio::test]
async fn login_rate_limited_429_retry_after() {
    let app = TestApp::with_fake_clock().await;
    app.insert_owner().await;
    let peer = Some("203.0.113.9:1234");
    for _ in 0..5 {
        assert_error(
            &app.login("wrong password!", false, peer).await,
            401,
            ErrorCode::InvalidCredentials,
        );
    }
    let res = app.login(OWNER_PASSWORD, false, peer).await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "requests"})));
    assert_eq!(
        res.header("retry-after"),
        "12",
        "one attempt every 12 s refills"
    );
    app.state.clock.advance(Duration::from_secs(11));
    assert_eq!(
        app.login(OWNER_PASSWORD, false, peer).await.status_code(),
        429
    );
    app.state.clock.advance(Duration::from_secs(1));
    app.login(OWNER_PASSWORD, false, peer)
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn email_limiter_counts_failures_only() {
    let app = TestApp::with_fake_clock().await;
    app.insert_owner().await;
    let peer = |i: usize| format!("198.51.100.{i}:1");
    // Successful logins never count.
    for i in 0..12 {
        app.login(OWNER_PASSWORD, false, Some(&peer(i)))
            .await
            .assert_status_ok();
    }
    for i in 0..10 {
        let res = app
            .login("wrong password!", false, Some(&peer(100 + i)))
            .await;
        assert_error(&res, 401, ErrorCode::InvalidCredentials);
    }
    // The 11th attempt is refused before the password is checked, even the right one, from any address.
    let res = app.login(OWNER_PASSWORD, false, Some(&peer(200))).await;
    let err = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(err.error.details, Some(json!({"reason": "auth_failures"})));
    // Another email is not locked.
    let res = app
        .anon_client()
        .post("/api/v1/auth/login")
        .add_header(PEER, peer(201))
        .json(&json!({"email": "other@quire.invalid", "password": "whatever password"}))
        .await;
    assert_error(&res, 401, ErrorCode::InvalidCredentials);
    app.state.clock.advance(Duration::from_secs(15 * 60));
    app.login(OWNER_PASSWORD, false, Some(&peer(202)))
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn unknown_email_same_code_as_bad_password() {
    let app = TestApp::new().await;
    app.insert_owner().await;
    let bad_password = app
        .login("wrong password!", false, Some("192.0.2.1:1"))
        .await;
    let unknown = app
        .anon_client()
        .post("/api/v1/auth/login")
        .add_header(PEER, "192.0.2.2:1")
        .json(&json!({"email": "nobody@quire.invalid", "password": OWNER_PASSWORD}))
        .await;
    let a = assert_error(&bad_password, 401, ErrorCode::InvalidCredentials);
    let b = assert_error(&unknown, 401, ErrorCode::InvalidCredentials);
    assert_eq!(a.error, b.error, "the same code, message and details");
    assert!(set_cookies(&unknown).is_empty());
}

async fn session_expiry(app: &TestApp, cookie: &str) -> (i64, i64) {
    let hash = quire_store::sessions::token_hash(cookie).unwrap();
    app.state
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT expires_at, created_at FROM sessions WHERE token_hash = ?1",
                [hash.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn renewal_after_half_window_resends_cookie() {
    const DAY: i64 = 86_400_000;
    let app = TestApp::with_fake_clock().await;
    app.insert_owner().await;
    let me = |cookie: &str| {
        app.client()
            .get("/api/v1/auth/me")
            .add_header("cookie", format!("quire_session={cookie}"))
    };
    // A persistent session: no renewal before half of the 30-day window has passed.
    let res = app.login(OWNER_PASSWORD, true, Some("192.0.2.1:1")).await;
    let cookie = cookie_from(&res, "quire_session").unwrap();
    let (expires, created) = session_expiry(&app, &cookie).await;
    assert_eq!(expires - created, 30 * DAY);
    app.state.clock.advance(Duration::from_secs(14 * 86_400));
    let res = me(&cookie).await;
    res.assert_status_ok();
    assert!(set_cookies(&res).is_empty());
    // Past half of it: a new expiry and the cookie again with the new Max-Age.
    app.state.clock.advance(Duration::from_secs(2 * 86_400));
    let res = me(&cookie).await;
    res.assert_status_ok();
    let renewed = set_cookies(&res);
    assert_eq!(renewed.len(), 1, "{renewed:?}");
    assert!(
        renewed[0].starts_with(&format!("quire_session={cookie};")),
        "same token"
    );
    assert!(renewed[0].contains("; Max-Age=2592000"), "{renewed:?}");
    let (expires, _) = session_expiry(&app, &cookie).await;
    assert_eq!(expires, app.state.now() + 30 * DAY);
    // The 180-day cap: near it the expiry is the cap.
    let hash = quire_store::sessions::token_hash(&cookie).unwrap();
    let now = app.state.now();
    app.state
        .db
        .write(move |tx| {
            tx.execute(
                "UPDATE sessions SET created_at = ?2, expires_at = ?3 WHERE token_hash = ?1",
                rusqlite::params![hash.as_slice(), now - 175 * DAY, now + DAY],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let res = me(&cookie).await;
    let renewed = set_cookies(&res);
    assert!(
        renewed[0].contains(&format!("; Max-Age={}", 5 * 86_400)),
        "{renewed:?}"
    );
    let (expires, created) = session_expiry(&app, &cookie).await;
    assert_eq!(expires - created, 180 * DAY);
    // A browser-session cookie is renewed in the database only (the cookie has no Max-Age to refresh).
    let res = app.login(OWNER_PASSWORD, false, Some("192.0.2.2:1")).await;
    let cookie = cookie_from(&res, "quire_session").unwrap();
    app.state.clock.advance(Duration::from_secs(13 * 3600));
    let res = me(&cookie).await;
    res.assert_status_ok();
    assert!(set_cookies(&res).is_empty());
    let (expires, _) = session_expiry(&app, &cookie).await;
    assert_eq!(expires, app.state.now() + DAY);
}

#[tokio::test]
async fn password_change_revokes_other_sessions() {
    let app = TestApp::new().await;
    app.insert_owner().await;
    let a = cookie_from(
        &app.login(OWNER_PASSWORD, false, Some("192.0.2.1:1")).await,
        "quire_session",
    )
    .unwrap();
    let b = cookie_from(
        &app.login(OWNER_PASSWORD, false, Some("192.0.2.2:1")).await,
        "quire_session",
    )
    .unwrap();
    let client_a = app.cookie_client(&a);
    let client_b = app.cookie_client(&b);
    let res = client_a
        .post("/api/v1/auth/password")
        .json(&json!({"current": "not my password", "new": "a brand new password"}))
        .await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "current"})));
    let res = client_a
        .post("/api/v1/auth/password")
        .json(&json!({"current": OWNER_PASSWORD, "new": "short"}))
        .await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "new"})));
    client_a
        .post("/api/v1/auth/password")
        .json(&json!({"current": OWNER_PASSWORD, "new": "a brand new password"}))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    client_a.get("/api/v1/auth/me").await.assert_status_ok();
    assert_error(
        &client_b.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_error(
        &app.login(OWNER_PASSWORD, false, Some("192.0.2.3:1")).await,
        401,
        ErrorCode::InvalidCredentials,
    );
    app.login("a brand new password", false, Some("192.0.2.4:1"))
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn reset_password_cli_sets_must_change() {
    let app = TestApp::with_owner().await;
    app.server.get("/api/v1/auth/me").await.assert_status_ok();
    let out = admin(
        app.dir.path(),
        &["--json", "reset-password", "--email", OWNER_EMAIL],
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reset = stdout_json(&out);
    let password = reset["password"].as_str().unwrap().to_owned();
    assert_eq!(password.len(), 24);
    assert_eq!(reset["sessions_revoked"], 1);
    assert_eq!(reset["tokens_revoked"], 0);
    assert_error(
        &app.server.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_error(
        &app.login(OWNER_PASSWORD, false, Some("192.0.2.1:1")).await,
        401,
        ErrorCode::InvalidCredentials,
    );
    let res = app.login(&password, false, Some("192.0.2.2:1")).await;
    res.assert_status_ok();
    let cookie = cookie_from(&res, "quire_session").unwrap();
    let me: Me = app
        .cookie_client(&cookie)
        .get("/api/v1/auth/me")
        .await
        .json();
    assert!(me.must_change_password);
    // Changing it clears the flag.
    app.cookie_client(&cookie)
        .post("/api/v1/auth/password")
        .json(&json!({"current": password, "new": "my own new password"}))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let me: Me = app
        .cookie_client(&cookie)
        .get("/api/v1/auth/me")
        .await
        .json();
    assert!(!me.must_change_password);
    let unknown = admin(
        app.dir.path(),
        &["reset-password", "--email", "nobody@quire.invalid"],
        None,
    );
    assert_eq!(unknown.status.code(), Some(7));
}

#[tokio::test]
async fn logout_clears_cookie_and_session() {
    let app = TestApp::new().await;
    app.insert_owner().await;
    let res = app.login(OWNER_PASSWORD, true, None).await;
    let cookie = cookie_from(&res, "quire_session").unwrap();
    let client = app.cookie_client(&cookie);
    let res = client.post("/api/v1/auth/logout").await;
    res.assert_status(StatusCode::NO_CONTENT);
    let cleared = &set_cookies(&res)[0];
    assert!(
        cleared.starts_with("quire_session=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );
    assert_error(
        &client.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
}

#[tokio::test]
async fn login_rotates_the_presented_session() {
    let app = TestApp::new().await;
    app.insert_owner().await;
    let old = cookie_from(
        &app.login(OWNER_PASSWORD, false, None).await,
        "quire_session",
    )
    .unwrap();
    let res = app
        .cookie_client(&old)
        .post("/api/v1/auth/login")
        .json(&json!({"email": OWNER_EMAIL, "password": OWNER_PASSWORD}))
        .await;
    res.assert_status_ok();
    let new = cookie_from(&res, "quire_session").unwrap();
    assert_ne!(new, old);
    assert_error(
        &app.cookie_client(&old).get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    app.cookie_client(&new)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
}

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
    let session: Me = TestApp::with_owner()
        .await
        .server
        .get("/api/v1/auth/me")
        .await
        .json();
    assert_eq!(session.auth.method.as_str(), "session");
    assert!(session.auth.token.is_none());
}

#[tokio::test]
async fn email_change_needs_session() {
    let app = TestApp::with_owner().await;
    let token = app.mint_token(app.owner(), Scope::Admin).await;
    let bearer = app.token_client(&token);
    let res = bearer
        .patch("/api/v1/auth/me")
        .json(&json!({"email": "new@quire.invalid"}))
        .await;
    assert_error(&res, 403, ErrorCode::SessionRequired);
    // The same email again is no change; a name or theme change is fine with a token.
    let res = bearer
        .patch("/api/v1/auth/me")
        .json(&json!({"email": OWNER_EMAIL.to_uppercase(), "name": "Renamed", "theme": "dark"}))
        .await;
    res.assert_status_ok();
    let body: UserEnvelope = res.json();
    assert_eq!(
        (body.user.name.as_str(), body.user.theme.as_str()),
        ("Renamed", "dark")
    );
    let res = app
        .server
        .patch("/api/v1/auth/me")
        .json(&json!({"email": "new@quire.invalid"}))
        .await;
    res.assert_status_ok();
    let body: UserEnvelope = res.json();
    assert_eq!(body.user.email, "new@quire.invalid");
    let res = app
        .server
        .patch("/api/v1/auth/me")
        .json(&json!({"email": "nope"}))
        .await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "email"})));
    let res = app
        .server
        .patch("/api/v1/auth/me")
        .json(&json!({"name": ""}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn sessions_others_with_token_revokes_all() {
    let app = TestApp::new().await;
    let owner = app.insert_owner().await;
    let a = app.insert_session(&owner).await;
    let b = app.insert_session(&owner).await;
    let c = app.insert_session(&owner).await;
    // From a session: every other session ends, the caller's stays.
    let listed: Vec<SessionInfo> = app
        .cookie_client(&a)
        .get("/api/v1/auth/sessions")
        .await
        .json();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed.iter().filter(|s| s.current).count(), 1);
    let res = app.cookie_client(&a).delete("/api/v1/auth/sessions").await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "others"})));
    app.cookie_client(&a)
        .delete("/api/v1/auth/sessions?others=true")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    app.cookie_client(&a)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
    for gone in [&b, &c] {
        assert_error(
            &app.cookie_client(gone).get("/api/v1/auth/me").await,
            401,
            ErrorCode::Unauthenticated,
        );
    }
    // From a token: every session ends.
    let d = app.insert_session(&owner).await;
    let token = app.mint_token(&owner, Scope::Admin).await;
    let listed: Vec<SessionInfo> = app
        .token_client(&token)
        .get("/api/v1/auth/sessions")
        .await
        .json();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|s| !s.current));
    app.token_client(&token)
        .delete("/api/v1/auth/sessions?others=true")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    for gone in [&a, &d] {
        assert_error(
            &app.cookie_client(gone).get("/api/v1/auth/me").await,
            401,
            ErrorCode::Unauthenticated,
        );
    }
    app.token_client(&token)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn revoke_one_session() {
    let app = TestApp::with_owner().await;
    let other = app.insert_session(app.owner()).await;
    let listed: Vec<SessionInfo> = app.server.get("/api/v1/auth/sessions").await.json();
    let id = listed.iter().find(|s| !s.current).unwrap().id;
    app.server
        .delete(&format!("/api/v1/auth/sessions/{id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_error(
        &app.cookie_client(&other).get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    let res = app
        .server
        .delete(&format!("/api/v1/auth/sessions/{id}"))
        .await;
    assert_error(&res, 404, ErrorCode::NotFound);
}

#[tokio::test]
async fn config_reports_limits_and_api_version() {
    let app = TestApp::with_config(|c| {
        c.max_upload_mb = 7;
        c.token_rate_per_min = 60;
    })
    .await;
    let res = app.anon_client().get("/api/v1/config").await;
    res.assert_status_ok();
    let raw: serde_json::Value = res.json();
    assert_eq!(raw["api_version"], quire_api::API_VERSION);
    assert_eq!(raw["openapi_url"], "/api/v1/openapi.json");
    assert_eq!(raw["multi_user"], false);
    assert_eq!(raw["inkboard"], serde_json::Value::Null);
    assert_eq!(raw["version"], env!("CARGO_PKG_VERSION"));
    assert!(raw["commit"].is_string());
    assert_eq!(
        raw["limits"],
        json!({
            "max_upload_mb": 7,
            "max_board_mb": 50,
            "max_doc_mb": 16,
            "max_update_bytes": 16 * 1024 * 1024,
            "max_markdown_bytes": 1024 * 1024,
            "max_json_bytes": 4 * 1024 * 1024,
            "max_body_bytes": 1024 * 1024,
            "json_depth": 100,
            "q_max_chars": 500,
            "page_max": 100,
            "token_rate_per_min": 60,
            "token_burst": 120,
            "token_bytes_per_hour": 1024 * 1024 * 1024u64,
        })
    );
    assert_eq!(res.header("quire-api-version"), quire_api::API_VERSION);
}

#[tokio::test]
async fn workspace_get_and_patch() {
    let app = TestApp::with_owner().await;
    let ws: quire_api::Workspace = app.server.get("/api/v1/workspace").await.json();
    assert_eq!((ws.name.as_str(), ws.logo_url), ("Test workspace", None));
    let res = app
        .server
        .patch("/api/v1/workspace")
        .json(&json!({"name": "  Renamed  "}))
        .await;
    res.assert_status_ok();
    let ws: quire_api::Workspace = res.json();
    assert_eq!(ws.name, "Renamed");
    let res = app
        .server
        .patch("/api/v1/workspace")
        .json(&json!({"logo_attachment_id": uuid::Uuid::now_v7()}))
        .await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(
        err.error.details,
        Some(json!({"field": "logo_attachment_id"}))
    );
    let res = app
        .server
        .patch("/api/v1/workspace")
        .json(&json!({"name": ""}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}
