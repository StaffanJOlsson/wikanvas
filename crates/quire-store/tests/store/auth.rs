//! Users and sessions tests (owner: W2)
//!
//! The owner insert and its guard, profile changes, settings, the "Getting started" seed, and the session lifecycle
//! (create, lookup by cookie, renewal, revocation, expiry).

use quire_core::api::{ErrorCode, Theme};
use quire_store::sessions::{
    self, NewSession, PERSISTENT_CAP_MS, PERSISTENT_WINDOW_MS, SESSION_WINDOW_MS,
};
use quire_store::users::{self, NewOwner, ProfilePatch};
use quire_store::{seed, settings};

use crate::common::{DAY, MINUTE, NOW, TestDb, count};

fn owner(email: &str) -> NewOwner {
    NewOwner {
        email: email.into(),
        name: " Ada ".into(),
        password_hash: "$argon2id$v=19$m=8,t=1,p=1$c2FsdHNhbHQ$aGFzaA".into(),
        must_change_password: true,
    }
}

#[tokio::test]
async fn create_owner_once_and_lookups() {
    let t = TestDb::new();
    let row =
        t.db.write(|tx| users::create_owner(tx, &owner(" Ada@Quire.Invalid "), NOW))
            .await
            .unwrap();
    assert_eq!(
        (row.email.as_str(), row.name.as_str()),
        ("Ada@Quire.Invalid", "Ada")
    );
    assert!(row.must_change_password && row.disabled_at.is_none());
    assert_eq!(row.theme, Theme::System);
    let e =
        t.db.write(|tx| users::create_owner(tx, &owner("other@quire.invalid"), NOW))
            .await
            .unwrap_err();
    assert!(e.is(ErrorCode::AlreadySetup), "{e:?}");
    let found =
        t.db.read(|c| users::by_email(c, "ADA@quire.invalid"))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(found.id, row.id);
    assert!(t.db.read(users::owner_exists).await.unwrap());
    assert_eq!(t.db.read(users::owner).await.unwrap().unwrap().id, row.id);
    let api = row.to_api().unwrap();
    assert_eq!(api.id.to_string(), row.id);
    for (email, field) in [("nope", "email"), ("a b@c", "email"), ("@x", "email")] {
        let e =
            t.db.write(move |tx| users::create_owner(tx, &owner(email), NOW))
                .await
                .unwrap_err();
        assert_eq!(
            e.details,
            Some(serde_json::json!({ "field": field })),
            "{email}"
        );
    }
}

#[tokio::test]
async fn profile_and_password_updates() {
    let t = TestDb::new();
    let id = t.owner().await;
    let patch = ProfilePatch {
        name: Some("  Grace ".into()),
        email: Some("grace@quire.invalid".into()),
        theme: Some(Theme::Dark),
    };
    let uid = id.clone();
    let row =
        t.db.write(move |tx| users::update_profile(tx, &uid, &patch, NOW + 1))
            .await
            .unwrap();
    assert_eq!(
        (row.name.as_str(), row.email.as_str(), row.theme),
        ("Grace", "grace@quire.invalid", Theme::Dark)
    );
    assert_eq!(row.updated_at, NOW + 1);
    t.user("member", "taken@quire.invalid").await;
    let uid = id.clone();
    let e =
        t.db.write(move |tx| {
            users::update_profile(
                tx,
                &uid,
                &ProfilePatch {
                    email: Some("TAKEN@quire.invalid".into()),
                    ..Default::default()
                },
                NOW,
            )
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Conflict), "{e:?}");
    let uid = id.clone();
    t.db.write(move |tx| users::set_password(tx, &uid, "$argon2id$new", false, NOW))
        .await
        .unwrap();
    let uid = id.clone();
    let row = t.db.read(move |c| users::get(c, &uid)).await.unwrap();
    assert_eq!(row.password_hash.as_deref(), Some("$argon2id$new"));
    let e = t.db.read(|c| users::get(c, "missing")).await.unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}

#[tokio::test]
async fn settings_roundtrip_and_workspace() {
    let t = TestDb::new();
    t.db.write(|tx| settings::init_workspace(tx, "Acme", NOW))
        .await
        .unwrap();
    t.db.write(|tx| settings::init_workspace(tx, "Acme 2", NOW + 5))
        .await
        .unwrap();
    let ws = t.db.read(settings::workspace).await.unwrap();
    assert_eq!((ws.name.as_str(), ws.logo_url), ("Acme 2", None));
    let created =
        t.db.read(|c| settings::get(c, settings::INSTANCE_CREATED_AT))
            .await
            .unwrap();
    assert_eq!(created, Some(serde_json::json!(NOW)), "set once");
    t.db.write(|tx| settings::set(tx, settings::WORKSPACE_LOGO, &serde_json::json!("att-1")))
        .await
        .unwrap();
    let ws = t.db.read(settings::workspace).await.unwrap();
    assert_eq!(ws.logo_url.as_deref(), Some("/api/v1/attachments/att-1"));
    t.db.write(|tx| settings::delete(tx, settings::WORKSPACE_LOGO))
        .await
        .unwrap();
    assert_eq!(
        t.db.read(|c| settings::get_str(c, settings::WORKSPACE_LOGO))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn seed_creates_getting_started() {
    let t = TestDb::new();
    let id = t.owner().await;
    let blobs = t.blobs.clone();
    let collection =
        t.db.write(move |tx| seed::getting_started(tx, &blobs, &id, NOW))
            .await
            .unwrap();
    assert_eq!(collection.name, "Getting started");
    assert_eq!(collection.color, "#3E63DD");
    assert_eq!(collection.icon.as_deref(), Some("rocket"));
    assert_eq!(collection.item_count, 3);
    let published = count(
        &t,
        "SELECT count(*) FROM items WHERE published_at = created_at AND collection_id = ?1",
        collection.id.to_string(),
    )
    .await;
    assert_eq!(published, 3);
    let children = count(
        &t,
        "SELECT count(*) FROM items c JOIN items p ON p.id = c.parent_id WHERE p.title = ?1",
        seed::WELCOME.into(),
    )
    .await;
    assert_eq!(children, 2);
}

fn new_session(user: &str, persistent: bool) -> NewSession {
    NewSession {
        user_id: user.into(),
        persistent,
        user_agent: Some("x".repeat(600)),
        ip: Some("192.0.2.1".into()),
    }
}

#[tokio::test]
async fn sessions_lifecycle() {
    let t = TestDb::new();
    let user = t.owner().await;
    let u = user.clone();
    let (row, cookie) =
        t.db.write(move |tx| sessions::create(tx, &new_session(&u, true), NOW))
            .await
            .unwrap();
    assert_eq!(row.expires_at, NOW + PERSISTENT_WINDOW_MS);
    assert_eq!(
        row.user_agent.as_ref().unwrap().len(),
        sessions::MAX_USER_AGENT
    );
    assert_eq!(row.max_age_secs(NOW), Some(PERSISTENT_WINDOW_MS / 1000));
    // Only the hash is stored.
    let stored = count(
        &t,
        "SELECT count(*) FROM sessions WHERE token_hash = unhex(?1)",
        sessions::token_hash(&cookie)
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
    .await;
    assert_eq!(stored, 1);
    let c = cookie.clone();
    let found =
        t.db.read(move |conn| sessions::lookup(conn, &c, NOW))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(found.id, row.id);
    let c = cookie.clone();
    let expired =
        t.db.read(move |conn| sessions::lookup(conn, &c, NOW + PERSISTENT_WINDOW_MS))
            .await
            .unwrap();
    assert!(expired.is_none());
    assert!(
        t.db.read(|conn| sessions::lookup(conn, "garbage", NOW))
            .await
            .unwrap()
            .is_none()
    );
    // Renewal past half of the window.
    let later = NOW + 16 * DAY;
    let next = sessions::renewal(&found, later).unwrap();
    assert_eq!(next, later + PERSISTENT_WINDOW_MS);
    assert!(sessions::needs_refresh(&found, NOW + 6 * MINUTE));
    assert!(!sessions::needs_refresh(&found, NOW + MINUTE));
    let id = row.id.clone();
    t.db.write(move |tx| sessions::refresh(tx, &id, Some(next), later, None))
        .await
        .unwrap();
    let c = cookie.clone();
    let renewed =
        t.db.read(move |conn| sessions::lookup(conn, &c, later))
            .await
            .unwrap()
            .unwrap();
    assert_eq!((renewed.expires_at, renewed.last_seen_at), (next, later));
    assert_eq!(
        renewed.ip.as_deref(),
        Some("192.0.2.1"),
        "kept without a new address"
    );
    assert!(
        sessions::renewal(&renewed, NOW + PERSISTENT_CAP_MS - DAY).unwrap()
            <= NOW + PERSISTENT_CAP_MS
    );
    // A disabled user's session does not authenticate.
    let u = user.clone();
    t.db.write(move |tx| {
        tx.execute("UPDATE users SET disabled_at = 1 WHERE id = ?1", [&u])?;
        Ok(())
    })
    .await
    .unwrap();
    let c = cookie.clone();
    assert!(
        t.db.read(move |conn| sessions::lookup(conn, &c, later))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sessions_revoke_others_and_expire() {
    let t = TestDb::new();
    let user = t.owner().await;
    let mut ids = Vec::new();
    for persistent in [true, false, false] {
        let u = user.clone();
        let (row, _) =
            t.db.write(move |tx| sessions::create(tx, &new_session(&u, persistent), NOW))
                .await
                .unwrap();
        ids.push(row.id);
    }
    let u = user.clone();
    let listed =
        t.db.read(move |c| sessions::list(c, &u, NOW))
            .await
            .unwrap();
    assert_eq!(listed.len(), 3);
    let (u, keep) = (user.clone(), ids[0].clone());
    let n =
        t.db.write(move |tx| sessions::delete_others(tx, &u, Some(&keep)))
            .await
            .unwrap();
    assert_eq!(n, 2);
    let (u, gone) = (user.clone(), ids[1].clone());
    assert_eq!(
        t.db.write(move |tx| sessions::delete(tx, &u, &gone))
            .await
            .unwrap(),
        0
    );
    let u = user.clone();
    let (_, _) =
        t.db.write(move |tx| sessions::create(tx, &new_session(&u, false), NOW))
            .await
            .unwrap();
    // The hourly job: the 24-hour session is expired a day later, the persistent one is not.
    let n =
        t.db.write(|tx| sessions::delete_expired(tx, NOW + SESSION_WINDOW_MS))
            .await
            .unwrap();
    assert_eq!(n, 1);
    let u = user.clone();
    let left =
        t.db.read(move |c| sessions::list(c, &u, NOW))
            .await
            .unwrap();
    assert_eq!(
        left.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        [ids[0].clone()]
    );
    let u = user.clone();
    assert_eq!(
        t.db.write(move |tx| sessions::delete_others(tx, &u, None))
            .await
            .unwrap(),
        1
    );
}
