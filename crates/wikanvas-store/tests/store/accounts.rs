//! Accounts and invites (D148): migration 0002 on an existing database, the account rules of `users::change`, and
//! the invite lifecycle (create, list, revoke, look up, accept, purge).

use wikanvas_core::api::token;
use wikanvas_core::api::{ErrorCode, InviteState, NewInvite, NewToken, Role, Scope};
use wikanvas_store::invites::{self, Acceptance};
use wikanvas_store::sessions::{self, NewSession};
use wikanvas_store::tokens::{self, CreatedVia, TokenLookup};
use wikanvas_store::users::{self, Manager, NewUser, UserChange};
use wikanvas_store::{Db, DbOptions, backup, migrate};

use crate::common::{DAY, NOW, TestDb};

const HASH: &str = "$argon2id$v=19$m=8,t=1,p=1$c2FsdHNhbHQ$aGFzaA";

fn new_user(email: &str, role: Role) -> NewUser {
    NewUser {
        email: email.into(),
        name: " Bea ".into(),
        password_hash: HASH.into(),
        role,
        must_change_password: false,
    }
}

fn invite(role: Role) -> NewInvite {
    NewInvite {
        role,
        expires_in_days: None,
        note: None,
    }
}

fn code<T: std::fmt::Debug>(r: wikanvas_store::Result<T>) -> ErrorCode {
    r.unwrap_err().code
}

/// The owner, an admin, a member and a viewer: (owner, admin, member, viewer) ids.
async fn people(t: &TestDb) -> (String, String, String, String) {
    let owner = t.owner().await;
    let admin = t.user("admin", "admin@wikanvas.invalid").await;
    let member = t.user("member", "member@wikanvas.invalid").await;
    let viewer = t.user("viewer", "viewer@wikanvas.invalid").await;
    (owner, admin, member, viewer)
}

#[tokio::test]
async fn migration_0002_backs_up_an_existing_database_first() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("wikanvas.db"), DbOptions::default()).unwrap();
    // An M1 database: schema version 1, with an owner and a token.
    {
        let mut conn = db.maintenance().unwrap();
        migrate::migrations().to_version(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO users (id, email, name, role, created_at, updated_at)
             VALUES ('0192f3a4-5b6c-7d8e-9f01-23456789abcd', 'o@wikanvas.invalid', 'O', 'owner', 1, 1)",
            [],
        )
        .unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let invites: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'invites'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(invites, 0);
    }
    let migrated = migrate::run(&db, dir.path(), NOW).unwrap();
    assert_eq!((migrated.from, migrated.to), (1, 2));
    let backup_path = migrated.backup.expect("a pre-migration backup");
    assert_eq!(
        backup_path,
        dir.path()
            .join("backups")
            .join("pre-migrate-v1-20261028T0930Z.db")
    );
    backup::quick_check(&backup_path).unwrap();
    let copy = rusqlite::Connection::open(&backup_path).unwrap();
    let (version, email): (u32, String) = copy
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version), (SELECT email FROM users)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((version, email.as_str()), (1, "o@wikanvas.invalid"));
    drop(copy);
    // The owner is still there and the invites table exists; a second run changes nothing.
    let owner = db.read(users::owner).await.unwrap().unwrap();
    assert_eq!(owner.email, "o@wikanvas.invalid");
    let pending = db.read(|c| invites::list_pending(c, NOW)).await.unwrap();
    assert!(pending.is_empty());
    let again = migrate::run(&db, dir.path(), NOW).unwrap();
    assert_eq!((again.from, again.to, again.backup), (2, 2, None));
}

#[tokio::test]
async fn create_user_validates_and_keeps_emails_unique_ignoring_case() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let bea = t
        .w(|tx| users::create(tx, &new_user(" Bea@Wikanvas.Invalid ", Role::Member), NOW))
        .await
        .unwrap();
    assert_eq!(
        (bea.email.as_str(), bea.name.as_str(), bea.role),
        ("Bea@Wikanvas.Invalid", "Bea", Role::Member)
    );
    let taken = t
        .w(|tx| users::create(tx, &new_user("bea@WIKANVAS.invalid", Role::Viewer), NOW))
        .await
        .unwrap_err();
    assert_eq!(taken.code, ErrorCode::Conflict);
    assert_eq!(taken.details, Some(serde_json::json!({"field": "email"})));
    let owner_email = t
        .w(|tx| users::create(tx, &new_user("OWNER@wikanvas.invalid", Role::Admin), NOW))
        .await
        .unwrap_err();
    assert_eq!(owner_email.code, ErrorCode::Conflict);
    for (email, role, field) in [
        ("nope", Role::Member, "email"),
        ("c@wikanvas.invalid", Role::Owner, "role"),
    ] {
        let e = t
            .w(move |tx| users::create(tx, &new_user(email, role), NOW))
            .await
            .unwrap_err();
        assert_eq!(
            e.details,
            Some(serde_json::json!({ "field": field })),
            "{email}"
        );
    }
    let list = t.r(users::list).await.unwrap();
    assert_eq!(
        list.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
        [owner.as_str(), bea.id.as_str()]
    );
    assert_eq!(t.r(users::count_enabled).await.unwrap(), 2);
    let info = bea.to_info().unwrap();
    assert_eq!(info.id.to_string(), bea.id);
    assert_eq!(info.disabled_at, None);
}

/// `users::change` by `by` (a user id and role, or the operator for `None`).
async fn change(
    t: &TestDb,
    by: Option<(&str, Role)>,
    id: &str,
    role: Option<Role>,
    disabled: Option<bool>,
) -> wikanvas_store::Result<users::Changed> {
    let by = by.map(|(id, role)| (id.to_owned(), role));
    let id = id.to_owned();
    t.w(move |tx| {
        let manager = match &by {
            Some((id, role)) => Manager::User { id, role: *role },
            None => Manager::Operator,
        };
        users::change(tx, manager, &id, &UserChange { role, disabled }, NOW)
    })
    .await
}

#[tokio::test]
async fn the_account_rules() {
    let t = TestDb::new();
    let (owner, admin, member, viewer) = people(&t).await;
    let as_owner = Some((owner.as_str(), Role::Owner));
    let as_admin = Some((admin.as_str(), Role::Admin));
    let as_member = Some((member.as_str(), Role::Member));
    let forbidden = ErrorCode::Forbidden;
    // The owner's role never changes and the owner cannot be disabled, by anyone.
    for by in [as_owner, as_admin, None] {
        for (role, disabled) in [(Some(Role::Admin), None), (None, Some(true))] {
            assert_eq!(
                code(change(&t, by, &owner, role, disabled).await),
                forbidden
            );
        }
    }
    // Nobody changes their own role or disables themselves.
    assert_eq!(
        code(change(&t, as_admin, &admin, Some(Role::Member), None).await),
        forbidden
    );
    assert_eq!(
        code(change(&t, as_admin, &admin, None, Some(true)).await),
        forbidden
    );
    // Only the owner (or the operator) makes or unmakes an admin, or disables one.
    assert_eq!(
        code(change(&t, as_admin, &member, Some(Role::Admin), None).await),
        forbidden
    );
    let other_admin = t.user("admin", "admin2@wikanvas.invalid").await;
    for (role, disabled) in [(Some(Role::Member), None), (None, Some(true))] {
        assert_eq!(
            code(change(&t, as_admin, &other_admin, role, disabled).await),
            forbidden
        );
    }
    // A member manages nobody (the API refuses it earlier, in authz::require).
    assert_eq!(
        code(change(&t, as_member, &viewer, Some(Role::Member), None).await),
        forbidden
    );
    // The owner is made once.
    let e = change(&t, as_owner, &member, Some(Role::Owner), None)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert_eq!(e.details, Some(serde_json::json!({"field": "role"})));
    // An admin manages members and viewers.
    let changed = change(&t, as_admin, &viewer, Some(Role::Member), None)
        .await
        .unwrap();
    assert_eq!(changed.user.role, Role::Member);
    let changed = change(&t, as_admin, &viewer, Some(Role::Viewer), None)
        .await
        .unwrap();
    assert_eq!(changed.user.role, Role::Viewer);
    // The owner makes and unmakes admins; so does the operator.
    let made = change(&t, as_owner, &member, Some(Role::Admin), None)
        .await
        .unwrap();
    assert_eq!(made.user.role, Role::Admin);
    let unmade = change(&t, None, &member, Some(Role::Member), None)
        .await
        .unwrap();
    assert_eq!(unmade.user.role, Role::Member);
    // A change to what it already is, or no change, writes nothing.
    let same = change(&t, as_admin, &member, Some(Role::Member), Some(false))
        .await
        .unwrap();
    assert_eq!(same.user.updated_at, unmade.user.updated_at);
    let none = change(&t, as_member, &owner, None, None).await.unwrap();
    assert_eq!(none.user.id, owner);
    // Unknown users are 404.
    assert_eq!(
        code(
            change(
                &t,
                as_owner,
                "0192f3a4-0000-7000-8000-000000000000",
                Some(Role::Viewer),
                None
            )
            .await
        ),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn disabling_ends_sessions_tokens_and_invites_at_once() {
    let t = TestDb::new();
    let (owner, admin, _member, _viewer) = people(&t).await;
    let a = admin.clone();
    let (cookie, secret) = t
        .w(move |tx| {
            let (_, cookie) = sessions::create(
                tx,
                &NewSession {
                    user_id: a.clone(),
                    ..Default::default()
                },
                NOW,
            )?;
            let token = tokens::create(
                tx,
                &a,
                &NewToken {
                    name: "agent".into(),
                    scopes: vec![Scope::Write],
                    expires_in_days: None,
                    password: None,
                },
                CreatedVia::Admin,
                None,
                NOW,
            )?;
            Ok((cookie, token.token))
        })
        .await
        .unwrap();
    let a = admin.clone();
    let made = t
        .w(move |tx| {
            invites::create(
                tx,
                Manager::User {
                    id: &a,
                    role: Role::Admin,
                },
                &invite(Role::Member),
                NOW,
            )
        })
        .await
        .unwrap();
    assert!(
        t.r(move |c| sessions::lookup(c, &cookie, NOW))
            .await
            .unwrap()
            .is_some()
    );
    let disabled = change(
        &t,
        Some((owner.as_str(), Role::Owner)),
        &admin,
        None,
        Some(true),
    )
    .await
    .unwrap();
    assert_eq!(disabled.user.disabled_at, Some(NOW));
    assert_eq!(
        (
            disabled.sessions_revoked,
            disabled.tokens_revoked,
            disabled.invites_revoked
        ),
        (1, 1, 1)
    );
    assert_eq!(t.r(users::count_enabled).await.unwrap(), 3);
    let s = secret.clone();
    assert_eq!(
        t.r(move |c| tokens::lookup(c, &s, NOW)).await.unwrap(),
        TokenLookup::Rejected
    );
    let tok = made.token.clone();
    let found = t
        .r(move |c| invites::lookup(c, &tok))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.state(NOW), InviteState::Revoked);
    // Enabling lets the account sign in again; what was revoked stays revoked.
    let enabled = change(&t, None, &admin, None, Some(false)).await.unwrap();
    assert_eq!(enabled.user.disabled_at, None);
    assert_eq!(
        t.r(move |c| tokens::lookup(c, &secret, NOW)).await.unwrap(),
        TokenLookup::Rejected
    );
    // Taking the admin role away revokes the invites the account made, too.
    let a = admin.clone();
    let second = t
        .w(move |tx| {
            invites::create(
                tx,
                Manager::User {
                    id: &a,
                    role: Role::Admin,
                },
                &invite(Role::Viewer),
                NOW,
            )
        })
        .await
        .unwrap();
    let demoted = change(&t, None, &admin, Some(Role::Member), None)
        .await
        .unwrap();
    assert_eq!(demoted.invites_revoked, 1);
    let tok = second.token.clone();
    assert_eq!(
        t.r(move |c| invites::lookup(c, &tok))
            .await
            .unwrap()
            .unwrap()
            .state(NOW),
        InviteState::Revoked
    );
}

#[tokio::test]
async fn invites_are_created_within_the_rules() {
    let t = TestDb::new();
    let (owner, admin, _, _) = people(&t).await;
    let create = |by: (String, Role), new: NewInvite| {
        t.w(move |tx| {
            invites::create(
                tx,
                Manager::User {
                    id: &by.0,
                    role: by.1,
                },
                &new,
                NOW,
            )
        })
    };
    let owner_by = (owner.clone(), Role::Owner);
    let admin_by = (admin.clone(), Role::Admin);
    let made = create(owner_by.clone(), invite(Role::Admin)).await.unwrap();
    assert!(token::check_invite(&made.token).is_ok(), "{}", made.token);
    assert_eq!(made.invite.expires_at, NOW + 7 * DAY);
    assert_eq!(made.invite.created_by.as_deref(), Some(owner.as_str()));
    assert_eq!(made.invite.state(NOW), InviteState::Pending);
    // Only the owner invites an admin; an admin invites members and viewers.
    assert_eq!(
        create(admin_by.clone(), invite(Role::Admin))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Forbidden
    );
    let note = NewInvite {
        role: Role::Viewer,
        expires_in_days: Some(30),
        note: Some("  For Alex  ".into()),
    };
    let viewer = create(admin_by.clone(), note).await.unwrap();
    assert_eq!(viewer.invite.expires_at, NOW + 30 * DAY);
    assert_eq!(viewer.invite.note.as_deref(), Some("For Alex"));
    let blank = NewInvite {
        role: Role::Member,
        expires_in_days: Some(1),
        note: Some("   ".into()),
    };
    assert_eq!(
        create(admin_by.clone(), blank).await.unwrap().invite.note,
        None
    );
    for (new, field) in [
        (invite(Role::Owner), "role"),
        (
            NewInvite {
                expires_in_days: Some(31),
                ..invite(Role::Member)
            },
            "expires_in_days",
        ),
        (
            NewInvite {
                expires_in_days: Some(0),
                ..invite(Role::Member)
            },
            "expires_in_days",
        ),
        (
            NewInvite {
                note: Some("x".repeat(201)),
                ..invite(Role::Member)
            },
            "note",
        ),
    ] {
        let e = create(owner_by.clone(), new).await.unwrap_err();
        assert_eq!(e.details, Some(serde_json::json!({ "field": field })));
    }
    // The list: pending only, newest first, never a token hash.
    let pending = t.r(|c| invites::list_pending(c, NOW)).await.unwrap();
    assert_eq!(pending.len(), 3);
    assert!(
        pending
            .windows(2)
            .all(|w| w[0].created_at >= w[1].created_at)
    );
    let api = made.invite.to_api().unwrap();
    assert_eq!(api.role, Role::Admin);
    // At most 100 pending invites.
    t.w(move |tx| {
        for _ in 0..97 {
            invites::create(tx, Manager::Operator, &invite(Role::Viewer), NOW)?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let e = create(owner_by.clone(), invite(Role::Viewer))
        .await
        .unwrap_err();
    assert_eq!(e.details, Some(serde_json::json!({"field": "invites"})));
    // A revoked one frees a place.
    let id = made.invite.id.clone();
    assert_eq!(
        t.w(move |tx| invites::revoke(tx, &id, NOW)).await.unwrap(),
        1
    );
    let id = made.invite.id.clone();
    assert_eq!(
        t.w(move |tx| invites::revoke(tx, &id, NOW)).await.unwrap(),
        0
    );
    create(owner_by, invite(Role::Viewer)).await.unwrap();
}

fn who(email: &str) -> Acceptance {
    Acceptance {
        email: email.into(),
        name: "New Person".into(),
        password_hash: HASH.into(),
    }
}

#[tokio::test]
async fn accept_is_single_use_and_refuses_expired_and_revoked_invites() {
    let t = TestDb::new();
    let _owner = t.owner().await;
    let new = |role| t.w(move |tx| invites::create(tx, Manager::Operator, &invite(role), NOW));
    let viewer = new(Role::Viewer).await.unwrap();
    let tok = viewer.token.clone();
    let user = t
        .w(move |tx| invites::accept(tx, &tok, &who("new@wikanvas.invalid"), NOW + 1))
        .await
        .unwrap();
    assert_eq!(
        (user.role, user.email.as_str()),
        (Role::Viewer, "new@wikanvas.invalid")
    );
    let tok = viewer.token.clone();
    let used = t
        .r(move |c| invites::lookup(c, &tok))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.state(NOW + 2), InviteState::Used);
    assert_eq!(used.used_by.as_deref(), Some(user.id.as_str()));
    // Single use.
    let tok = viewer.token.clone();
    let again = t
        .w(move |tx| invites::accept(tx, &tok, &who("other@wikanvas.invalid"), NOW + 2))
        .await
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
    assert!(again.message.contains("used"), "{}", again.message);
    // Expired.
    let member = new(Role::Member).await.unwrap();
    let tok = member.token.clone();
    let expired = t
        .w(move |tx| invites::accept(tx, &tok, &who("late@wikanvas.invalid"), NOW + 7 * DAY))
        .await
        .unwrap_err();
    assert!(expired.message.contains("expired"), "{}", expired.message);
    // A taken email leaves the invite pending (the transaction rolls back).
    let tok = member.token.clone();
    let taken = t
        .w(move |tx| invites::accept(tx, &tok, &who("NEW@wikanvas.invalid"), NOW))
        .await
        .unwrap_err();
    assert_eq!(taken.code, ErrorCode::Conflict);
    let tok = member.token.clone();
    let still = t
        .r(move |c| invites::lookup(c, &tok))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.state(NOW), InviteState::Pending);
    // Revoked.
    let id = member.invite.id.clone();
    t.w(move |tx| invites::revoke(tx, &id, NOW)).await.unwrap();
    let tok = member.token.clone();
    let revoked = t
        .w(move |tx| invites::accept(tx, &tok, &who("late@wikanvas.invalid"), NOW))
        .await
        .unwrap_err();
    assert!(revoked.message.contains("revoked"), "{}", revoked.message);
    // A string that is not an invite token is never looked up; an unknown one is none.
    for bad in [
        "nope".to_owned(),
        token::encode(&[1; 32]),
        token::encode_invite(&[1; 32]),
    ] {
        assert!(
            t.r(move |c| invites::lookup(c, &bad))
                .await
                .unwrap()
                .is_none()
        );
    }
    // The hourly purge drops revoked and expired invites after 30 days, and keeps used ones.
    let purged = t.w(|tx| invites::purge(tx, NOW + 38 * DAY)).await.unwrap();
    assert_eq!(purged, 1);
    let tok = viewer.token.clone();
    assert!(
        t.r(move |c| invites::lookup(c, &tok))
            .await
            .unwrap()
            .is_some()
    );
}
