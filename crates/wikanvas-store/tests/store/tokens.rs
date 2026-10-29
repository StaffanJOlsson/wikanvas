//! Personal access token tests (owner: W2)
//!
//! Creation (format, stored hash and prefix, expanded scopes, parent caps, the 100 limit), lookup with
//! classification (live, expired, rejected), list, revoke with children, the last-use write, and the 30-day purge.

use rusqlite::params;
use sha2::{Digest, Sha256};
use wikanvas_core::api::token;
use wikanvas_core::api::{ApiToken, CreatedToken, ErrorCode, NewToken, Role, Scope};
use wikanvas_store::tokens::{self, CreatedVia, TokenLookup, TokenRow};

use crate::common::{DAY, NOW, TestDb};

fn new_token(name: &str, scopes: &[Scope], days: Option<Option<u32>>) -> NewToken {
    NewToken {
        name: name.to_owned(),
        scopes: scopes.to_vec(),
        expires_in_days: days,
        password: None,
    }
}

async fn create(
    t: &TestDb,
    user: &str,
    new: NewToken,
    parent: Option<TokenRow>,
    now: i64,
) -> wikanvas_store::Result<CreatedToken> {
    let user = user.to_owned();
    t.db.write(move |tx| tokens::create(tx, &user, &new, CreatedVia::Api, parent.as_ref(), now))
        .await
}

async fn lookup(t: &TestDb, token: &str, now: i64) -> TokenLookup {
    let token = token.to_owned();
    t.db.read(move |c| tokens::lookup(c, &token, now))
        .await
        .unwrap()
}

async fn live(t: &TestDb, token: &str) -> TokenRow {
    match lookup(t, token, NOW).await {
        TokenLookup::Live(row) => row,
        other => panic!("{other:?}"),
    }
}

async fn list(t: &TestDb, user: &str) -> Vec<ApiToken> {
    let user = user.to_owned();
    t.db.read(move |c| tokens::list(c, &user, NOW))
        .await
        .unwrap()
}

async fn revoke(t: &TestDb, user: &str, id: &str) -> u64 {
    let (user, id) = (user.to_owned(), id.to_owned());
    t.db.write(move |tx| tokens::revoke(tx, &user, &id, NOW))
        .await
        .unwrap()
}

#[tokio::test]
async fn create_stores_hash_prefix_and_expanded_scopes() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let created = create(
        &t,
        &owner,
        new_token("  ci  ", &[Scope::Write], None),
        None,
        NOW,
    )
    .await
    .unwrap();
    let secret = &created.token;
    assert_eq!(token::check(secret), Ok(()));
    let api = &created.api_token;
    assert_eq!(api.name, "ci");
    assert_eq!(api.prefix, &secret[..12]);
    assert_eq!(api.scopes, [Scope::Read, Scope::Write]);
    assert_eq!(api.expires_at.map(|e| e.millis()), Some(NOW + 90 * DAY));
    assert_eq!(
        (api.created_via, api.parent_token_id),
        (CreatedVia::Api, None)
    );
    let id = api.id.to_string();
    let (hash, prefix, scopes): (Vec<u8>, String, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT token_hash, prefix, (SELECT group_concat(scope, ',') FROM
                   (SELECT scope FROM api_token_scopes WHERE token_id = ?1 ORDER BY scope))
                 FROM api_tokens WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!(
        hash,
        Sha256::digest(secret.as_bytes()).to_vec(),
        "sha256 of the whole token"
    );
    assert_eq!(prefix, &secret[..12]);
    assert_eq!(scopes, "read,write");
    let row = live(&t, secret).await;
    assert_eq!(row.id, api.id.to_string());
    assert_eq!(
        (row.user_id.as_str(), row.role),
        (owner.as_str(), Role::Owner)
    );
    assert_eq!(row.scopes, [Scope::Read, Scope::Write]);
    // Never-expiring and admin tokens (a session's; the route checks the password).
    let never = create(
        &t,
        &owner,
        new_token("never", &[Scope::Admin], Some(None)),
        None,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(never.api_token.expires_at, None);
    assert_eq!(
        never.api_token.scopes,
        [Scope::Read, Scope::Write, Scope::Admin]
    );
    assert_ne!(never.token, created.token);
}

#[tokio::test]
async fn create_validates_its_input() {
    let t = TestDb::new();
    let owner = t.owner().await;
    for (new, field) in [
        (new_token("", &[Scope::Read], None), "name"),
        (new_token(&"n".repeat(101), &[Scope::Read], None), "name"),
        (new_token("t", &[], None), "scopes"),
        (
            new_token("t", &[Scope::Read], Some(Some(0))),
            "expires_in_days",
        ),
        (
            new_token("t", &[Scope::Read], Some(Some(3651))),
            "expires_in_days",
        ),
    ] {
        let e = create(&t, &owner, new, None, NOW).await.unwrap_err();
        assert!(e.is(ErrorCode::Validation), "{e:?}");
        assert_eq!(e.details, Some(serde_json::json!({ "field": field })));
    }
}

#[tokio::test]
async fn lookup_classifies_live_expired_revoked_disabled_and_malformed() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let short = create(
        &t,
        &owner,
        new_token("short", &[Scope::Read], Some(Some(1))),
        None,
        NOW,
    )
    .await
    .unwrap();
    assert!(matches!(
        lookup(&t, &short.token, NOW + DAY - 1).await,
        TokenLookup::Live(_)
    ));
    assert_eq!(
        lookup(&t, &short.token, NOW + DAY).await,
        TokenLookup::Expired
    );
    // Malformed (bad checksum, wrong prefix, wrong length) and unknown well-formed tokens are rejected alike.
    let mut bad = short.token.clone();
    let last = bad.pop().unwrap();
    bad.push(if last == 'A' { 'B' } else { 'A' });
    let unknown = token::encode(&[7u8; 32]);
    for s in [bad.as_str(), "qpat_short", "Bearer x", unknown.as_str()] {
        assert_eq!(lookup(&t, s, NOW).await, TokenLookup::Rejected, "{s}");
    }
    // Revoked: rejected, also once it would have expired.
    let revoked = create(
        &t,
        &owner,
        new_token("revoked", &[Scope::Read], None),
        None,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        revoke(&t, &owner, &revoked.api_token.id.to_string()).await,
        1
    );
    assert_eq!(lookup(&t, &revoked.token, NOW).await, TokenLookup::Rejected);
    assert_eq!(
        lookup(&t, &revoked.token, NOW + 100 * DAY).await,
        TokenLookup::Rejected
    );
    // A disabled user's tokens are rejected.
    let o = owner.clone();
    t.db.write(move |tx| {
        tx.execute(
            "UPDATE users SET disabled_at = ?2 WHERE id = ?1",
            params![o, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(lookup(&t, &short.token, NOW).await, TokenLookup::Rejected);
}

#[tokio::test]
async fn parent_caps_scope_and_expiry_and_forbids_admin() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let parent = create(
        &t,
        &owner,
        new_token("parent", &[Scope::Write], Some(Some(30))),
        None,
        NOW,
    )
    .await
    .unwrap();
    let p = live(&t, &parent.token).await;
    let e = create(
        &t,
        &owner,
        new_token("admin", &[Scope::Admin], Some(Some(1))),
        Some(p.clone()),
        NOW,
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::SessionRequired), "{e:?}");
    // Default 90 days and never both outlive a 30-day parent.
    for days in [None, Some(None), Some(Some(31))] {
        let e = create(
            &t,
            &owner,
            new_token("long", &[Scope::Read], days),
            Some(p.clone()),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(e.is(ErrorCode::Validation), "{days:?}: {e:?}");
        assert_eq!(
            e.details,
            Some(serde_json::json!({"field": "expires_in_days"}))
        );
    }
    let child = create(
        &t,
        &owner,
        new_token("child", &[Scope::Write], Some(Some(30))),
        Some(p.clone()),
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(child.api_token.parent_token_id, Some(parent.api_token.id));
    // A read token cannot mint a write token.
    let reader = create(
        &t,
        &owner,
        new_token("reader", &[Scope::Read], Some(Some(30))),
        None,
        NOW,
    )
    .await
    .unwrap();
    let r = live(&t, &reader.token).await;
    let e = create(
        &t,
        &owner,
        new_token("w", &[Scope::Write], Some(Some(1))),
        Some(r),
        NOW,
    )
    .await
    .unwrap_err();
    assert_eq!(e.details, Some(serde_json::json!({"field": "scopes"})));
    // A token of another user is not a parent here.
    let other = t.user("member", "member@wikanvas.invalid").await;
    let e = create(
        &t,
        &other,
        new_token("x", &[Scope::Read], Some(Some(1))),
        Some(p),
        NOW,
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::Forbidden));
}

#[tokio::test]
async fn revoking_a_parent_revokes_its_children() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let parent = create(
        &t,
        &owner,
        new_token("parent", &[Scope::Write], Some(None)),
        None,
        NOW,
    )
    .await
    .unwrap();
    let p = live(&t, &parent.token).await;
    let child = create(
        &t,
        &owner,
        new_token("child", &[Scope::Write], None),
        Some(p),
        NOW,
    )
    .await
    .unwrap();
    let c = live(&t, &child.token).await;
    let grandchild = create(
        &t,
        &owner,
        new_token("grandchild", &[Scope::Read], None),
        Some(c),
        NOW,
    )
    .await
    .unwrap();
    let sibling = create(
        &t,
        &owner,
        new_token("sibling", &[Scope::Read], None),
        None,
        NOW,
    )
    .await
    .unwrap();
    // Another user cannot revoke it; an unknown id revokes nothing.
    let other = t.user("member", "member@wikanvas.invalid").await;
    assert_eq!(
        revoke(&t, &other, &parent.api_token.id.to_string()).await,
        0
    );
    assert_eq!(
        revoke(&t, &owner, &uuid::Uuid::now_v7().to_string()).await,
        0
    );
    assert_eq!(
        revoke(&t, &owner, &parent.api_token.id.to_string()).await,
        3
    );
    for s in [&parent.token, &child.token, &grandchild.token] {
        assert_eq!(lookup(&t, s, NOW).await, TokenLookup::Rejected);
    }
    assert!(matches!(
        lookup(&t, &sibling.token, NOW).await,
        TokenLookup::Live(_)
    ));
    // Revoking again finds nothing to revoke.
    assert_eq!(
        revoke(&t, &owner, &parent.api_token.id.to_string()).await,
        0
    );
}

#[tokio::test]
async fn list_is_newest_first_without_revoked_tokens() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let a = create(
        &t,
        &owner,
        new_token("a", &[Scope::Read], Some(Some(1))),
        None,
        NOW - 2 * DAY,
    )
    .await
    .unwrap();
    let b = create(
        &t,
        &owner,
        new_token("b", &[Scope::Admin], None),
        None,
        NOW - DAY,
    )
    .await
    .unwrap();
    let c = create(&t, &owner, new_token("c", &[Scope::Read], None), None, NOW)
        .await
        .unwrap();
    revoke(&t, &owner, &c.api_token.id.to_string()).await;
    let listed = list(&t, &owner).await;
    // `a` has expired and is still listed; `c` is revoked and is not.
    assert_eq!(
        listed.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(listed[0], b.api_token);
    assert_eq!(listed[1], a.api_token);
    let json = serde_json::to_string(&listed).unwrap();
    assert!(!json.contains(&b.token) && !json.contains(&a.token));
    assert!(list(&t, "nobody").await.is_empty());
}

#[tokio::test]
async fn get_finds_one_unrevoked_token_of_the_user() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let a = create(
        &t,
        &owner,
        new_token("a", &[Scope::Write], Some(Some(1))),
        None,
        NOW - 2 * DAY,
    )
    .await
    .unwrap();
    let b = create(&t, &owner, new_token("b", &[Scope::Read], None), None, NOW)
        .await
        .unwrap();
    let get = |user: &str, id: String| {
        let user = user.to_owned();
        let db = t.db.clone();
        async move { db.read(move |c| tokens::get(c, &user, &id)).await.unwrap() }
    };
    // Expired tokens are still described, like in `list`.
    assert_eq!(
        get(&owner, a.api_token.id.to_string()).await,
        Some(a.api_token.clone())
    );
    assert_eq!(
        get(&owner, b.api_token.id.to_string()).await,
        Some(b.api_token.clone())
    );
    assert_eq!(get("nobody", a.api_token.id.to_string()).await, None);
    assert_eq!(get(&owner, "not-an-id".into()).await, None);
    revoke(&t, &owner, &b.api_token.id.to_string()).await;
    assert_eq!(get(&owner, b.api_token.id.to_string()).await, None);
}

#[tokio::test]
async fn at_most_100_live_tokens() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let o = owner.clone();
    t.db.write(move |tx| {
        for i in 0..100 {
            tokens::create(
                tx,
                &o,
                &new_token(&format!("t{i}"), &[Scope::Read], None),
                CreatedVia::Admin,
                None,
                NOW,
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let e = create(
        &t,
        &owner,
        new_token("101st", &[Scope::Read], None),
        None,
        NOW,
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::Validation), "{e:?}");
    // Expired tokens do not count.
    assert!(
        create(
            &t,
            &owner,
            new_token("later", &[Scope::Read], None),
            None,
            NOW + 91 * DAY
        )
        .await
        .is_ok()
    );
    assert_eq!(
        list(&t, &owner).await.len(),
        100,
        "the list is capped at 100"
    );
}

#[tokio::test]
async fn purged_30_days_after_revocation() {
    let t = TestDb::new();
    let user = t.owner().await;
    let revoked = create(
        &t,
        &user,
        new_token("revoked", &[Scope::Read], Some(None)),
        None,
        NOW,
    )
    .await
    .unwrap();
    let expiring = create(
        &t,
        &user,
        new_token("expiring", &[Scope::Admin], Some(Some(1))),
        None,
        NOW,
    )
    .await
    .unwrap();
    let parent = live(&t, &expiring.token).await;
    let child = create(
        &t,
        &user,
        new_token("child", &[Scope::Read], Some(Some(1))),
        Some(parent),
        NOW,
    )
    .await
    .unwrap();
    let kept = create(
        &t,
        &user,
        new_token("kept", &[Scope::Write], Some(None)),
        None,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        revoke(&t, &user, &revoked.api_token.id.to_string()).await,
        1
    );
    let db = t.db.clone();
    let purge = |now: i64| {
        let db = db.clone();
        async move { db.write(move |tx| tokens::purge(tx, now)).await.unwrap() }
    };
    // 30 days after revocation (and after the 1-day expiry) nothing is deleted yet...
    assert_eq!(purge(NOW + 30 * DAY - 1).await, 0);
    assert_eq!(purge(NOW + 30 * DAY).await, 1, "the revoked token");
    // ...the expired parent and child go a day later, with their scopes; the live token stays.
    assert_eq!(purge(NOW + 31 * DAY).await, 2);
    let rows: Vec<String> = t
        .query(|c| {
            c.prepare("SELECT name FROM api_tokens")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(rows, ["kept"]);
    let scopes: i64 = t
        .query(|c| c.query_row("SELECT count(*) FROM api_token_scopes", [], |r| r.get(0)))
        .await;
    assert_eq!(scopes, 2, "read and write of the kept token");
    let _ = (child, kept);
}

#[tokio::test]
async fn touch_last_used_and_revoke_all() {
    let t = TestDb::new();
    let user = t.owner().await;
    let a = create(&t, &user, new_token("a", &[Scope::Read], None), None, NOW)
        .await
        .unwrap();
    let b = create(&t, &user, new_token("b", &[Scope::Read], None), None, NOW)
        .await
        .unwrap();
    let id = a.api_token.id.to_string();
    t.db.write(move |tx| tokens::touch_last_used(tx, &id, NOW + 5, Some("192.0.2.9")))
        .await
        .unwrap();
    let id = a.api_token.id.to_string();
    t.db.write(move |tx| tokens::touch_last_used(tx, &id, NOW + 9, None))
        .await
        .unwrap();
    let listed = list(&t, &user).await;
    let a_row = listed.iter().find(|x| x.name == "a").unwrap();
    assert_eq!(a_row.last_used_at.map(|ts| ts.millis()), Some(NOW + 9));
    assert_eq!(a_row.last_used_ip.as_deref(), Some("192.0.2.9"));
    let row = live(&t, &a.token).await;
    let id = row.id.clone();
    let again =
        t.db.read(move |c| tokens::live_row(c, &id, NOW))
            .await
            .unwrap();
    assert_eq!(again, Some(row));
    let u = user.clone();
    assert_eq!(
        t.db.write(move |tx| tokens::revoke_all(tx, &u, NOW))
            .await
            .unwrap(),
        2
    );
    assert_eq!(lookup(&t, &b.token, NOW).await, TokenLookup::Rejected);
    let id = a.api_token.id.to_string();
    assert_eq!(
        t.db.read(move |c| tokens::live_row(c, &id, NOW))
            .await
            .unwrap(),
        None
    );
}
