//! Access per collection (D152): migration 0003 on an existing database, the effective access of each role, the
//! visibility filter of the lists, attachments, memberships and their epochs, and the directory.

use wikanvas_core::api::{
    CollectionAccess, CollectionPatch, DefaultAccess, ItemKind, MemberAccess, NewCollection, Role,
    SearchQuery,
};
use wikanvas_store::lists::PageRequest;
use wikanvas_store::users::{self, Manager, UserChange};
use wikanvas_store::{Db, DbOptions, Viewer, access, collections, lists, migrate, search, stars};

use crate::common::{NOW, TestDb};

fn as_(user: &str, role: Role) -> Viewer {
    Viewer {
        user_id: user.to_owned(),
        role,
    }
}

/// The owner, a member and a viewer, and a private collection (`none`) with a document.
async fn private_world(t: &TestDb) -> (String, String, String, String, String) {
    let owner = t.owner().await;
    let member = t.user("member", "member@wikanvas.invalid").await;
    let viewer = t.user("viewer", "viewer@wikanvas.invalid").await;
    let c = t.collection(&owner, "Private").await;
    let cid = c.id.to_string();
    let o = owner.clone();
    t.w(move |tx| {
        collections::update(
            tx,
            &as_(&o, Role::Owner),
            &cid,
            &CollectionPatch {
                default_access: Some(DefaultAccess::None),
                ..Default::default()
            },
            NOW,
        )
    })
    .await
    .unwrap();
    let doc = t
        .item(&owner, c.id, None, ItemKind::Doc, "Hidden plan")
        .await;
    (owner, member, viewer, c.id.to_string(), doc.id.to_string())
}

#[tokio::test]
async fn migration_0003_keeps_todays_access_and_gives_creators_manage() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("wikanvas.db"), DbOptions::default()).unwrap();
    // A database at schema version 2: the owner, a member, a collection each made, and a board with a card.
    let board_with_card = include_str!("../../../../fixtures/boards/embed.inkboard");
    {
        let mut conn = db.maintenance().unwrap();
        migrate::migrations().to_version(&mut conn, 2).unwrap();
        conn.execute_batch(
            "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES
               ('00000000-0000-7000-8000-00000000000a', 'o@wikanvas.invalid', 'O', 'owner', 1, 1),
               ('00000000-0000-7000-8000-00000000000b', 'm@wikanvas.invalid', 'M', 'member', 1, 1);
             INSERT INTO collections (id, url_id, name, sort_key, created_by, created_at, updated_at) VALUES
               ('00000000-0000-7000-8000-0000000000c1', 'aaaaaaaaa1', 'Owner''s', 'a0',
                '00000000-0000-7000-8000-00000000000a', 2, 2),
               ('00000000-0000-7000-8000-0000000000c2', 'aaaaaaaaa2', 'Member''s', 'a1',
                '00000000-0000-7000-8000-00000000000b', 3, 3),
               ('00000000-0000-7000-8000-0000000000c3', 'aaaaaaaaa3', 'Nobody''s', 'a2', NULL, 4, 4);
             INSERT INTO items (id, url_id, collection_id, kind, title, sort_key, created_at, updated_at, published_at)
               VALUES ('00000000-0000-7000-8000-0000000000b1', 'bbbbbbbbb1', '00000000-0000-7000-8000-0000000000c1',
                       'board', 'Board', 'a0', 5, 5, 5);",
        )
        .unwrap();
        // The board as an earlier build stored it: the card's label in its text.
        let body = wikanvas_store::boards::deflate(board_with_card.as_bytes()).unwrap();
        conn.execute(
            "INSERT INTO board_state (item_id, body, body_sha256, raw_len, format_version, engine_version, meta,
               updated_at)
             VALUES ('00000000-0000-7000-8000-0000000000b1', ?1, x'00', 1, 2, 'server',
               '{\"pages\":[],\"objects\":3,\"background\":\"#ffffff\",\"bytes\":1,\
                 \"text_excerpt\":\"Page 1\\nFirst page\\nSecond page\\nPlain box\"}', 5)",
            [body],
        )
        .unwrap();
        conn.execute(
            "UPDATE item_text SET body = 'Page 1\nFirst page\nSecond page\nPlain box'
             WHERE item_id = '00000000-0000-7000-8000-0000000000b1'",
            [],
        )
        .unwrap();
    }
    let migrated = migrate::run(&db, dir.path(), NOW).unwrap();
    assert_eq!((migrated.from, migrated.to), (2, 3));
    let backup = migrated.backup.expect("a pre-migration backup");
    assert_eq!(
        backup.file_name().unwrap(),
        "pre-migrate-v2-20261028T0930Z.db"
    );
    let rows: Vec<(String, String)> = db
        .read(|c| {
            Ok(
                c.prepare("SELECT name, default_access FROM collections ORDER BY sort_key")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?,
            )
        })
        .await
        .unwrap();
    assert!(rows.iter().all(|(_, a)| a == "write"), "{rows:?}");
    let members: Vec<(String, String, String)> = db
        .read(|c| {
            Ok(c.prepare(
                "SELECT collection_id, user_id, access FROM collection_members ORDER BY collection_id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
        })
        .await
        .unwrap();
    assert_eq!(
        members,
        [
            (
                "00000000-0000-7000-8000-0000000000c1".to_owned(),
                "00000000-0000-7000-8000-00000000000a".to_owned(),
                "manage".to_owned()
            ),
            (
                "00000000-0000-7000-8000-0000000000c2".to_owned(),
                "00000000-0000-7000-8000-00000000000b".to_owned(),
                "manage".to_owned()
            ),
        ]
    );
    // Today's access: a member edits every collection, its own one it manages.
    let m = as_("00000000-0000-7000-8000-00000000000b", Role::Member);
    let access_of = |id: &'static str| {
        let m = m.clone();
        db.read(move |c| access::collection(c, &m, id))
    };
    assert_eq!(
        access_of("00000000-0000-7000-8000-0000000000c1")
            .await
            .unwrap(),
        CollectionAccess::Write
    );
    assert_eq!(
        access_of("00000000-0000-7000-8000-0000000000c2")
            .await
            .unwrap(),
        CollectionAccess::Manage
    );
    // The board's text no longer holds its cards' labels (D152).
    let (body, excerpt): (String, String) = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT t.body, json_extract(s.meta, '$.text_excerpt') FROM item_text t
                   JOIN board_state s ON s.item_id = t.item_id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        (body.as_str(), excerpt.as_str()),
        ("Page 1\nPlain box", "Page 1\nPlain box")
    );
    let again = migrate::run(&db, dir.path(), NOW).unwrap();
    assert_eq!((again.from, again.to, again.backup), (3, 3, None));
}

#[tokio::test]
async fn effective_access_per_role_and_membership() {
    let t = TestDb::new();
    let (owner, member, viewer, private, doc) = private_world(&t).await;
    let admin = t.user("admin", "admin@wikanvas.invalid").await;
    let check = |who: Viewer, collection: CollectionAccess| {
        let (p, d) = (private.clone(), doc.clone());
        let t = &t;
        async move {
            let w = who.clone();
            let got = t
                .r(move |c| Ok((access::collection(c, &w, &p)?, access::item(c, &w, &d)?)))
                .await
                .unwrap();
            assert_eq!(got, (collection, collection), "{who:?}");
        }
    };
    check(as_(&owner, Role::Owner), CollectionAccess::Manage).await;
    check(as_(&admin, Role::Admin), CollectionAccess::Manage).await;
    check(as_(&member, Role::Member), CollectionAccess::None).await;
    check(as_(&viewer, Role::Viewer), CollectionAccess::None).await;
    // A missing collection or item is `none`, as a private one is.
    let m = as_(&member, Role::Member);
    let missing = t
        .r(move |c| {
            Ok((
                access::collection(c, &m, "00000000-0000-7000-8000-000000000000")?,
                access::item(c, &m, "nourlidxyz")?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(missing, (CollectionAccess::None, CollectionAccess::None));
    for (grant, as_member, as_viewer) in [
        (
            MemberAccess::Read,
            CollectionAccess::Read,
            CollectionAccess::Read,
        ),
        (
            MemberAccess::Write,
            CollectionAccess::Write,
            CollectionAccess::Read,
        ),
        (
            MemberAccess::Manage,
            CollectionAccess::Manage,
            CollectionAccess::Read,
        ),
    ] {
        for user in [&member, &viewer] {
            let (p, u, by) = (private.clone(), user.clone(), owner.clone());
            t.w(move |tx| access::set_member(tx, &p, &u, grant, Some(&by), NOW))
                .await
                .unwrap();
        }
        check(as_(&member, Role::Member), as_member).await;
        check(as_(&viewer, Role::Viewer), as_viewer).await;
    }
    // By url_id too.
    let url_id: String = t
        .query({
            let p = private.clone();
            move |c| {
                c.query_row("SELECT url_id FROM collections WHERE id = ?1", [p], |r| {
                    r.get(0)
                })
            }
        })
        .await;
    let m = as_(&member, Role::Member);
    let by_url = t
        .r(move |c| access::collection(c, &m, &url_id))
        .await
        .unwrap();
    assert_eq!(by_url, CollectionAccess::Manage);
}

#[tokio::test]
async fn lists_search_and_stars_show_only_readable_collections() {
    let t = TestDb::new();
    let (owner, member, _viewer, private, doc) = private_world(&t).await;
    let open = t.collection(&owner, "Open").await;
    t.item(&owner, open.id, None, ItemKind::Doc, "Open plan")
        .await;
    // The member starred the hidden document once (a star survives losing access, and shows again with it).
    let (m, d) = (member.clone(), doc.clone());
    t.w(move |tx| stars::star(tx, &m, &d, None, NOW))
        .await
        .unwrap();
    let seen = |who: Viewer| {
        let t = &t;
        async move {
            t.r(move |c| {
                let sidebar = lists::sidebar(c, &who)?;
                let updated =
                    lists::updated(c, &who, None, &PageRequest::new("updated", None, None))?;
                let hits = search::search(
                    c,
                    &who,
                    &SearchQuery {
                        q: "plan".into(),
                        ..Default::default()
                    },
                )?;
                let titles = search::titles(c, &who, "plan", None)?;
                Ok((
                    sidebar
                        .collections
                        .into_iter()
                        .map(|c| c.name)
                        .collect::<Vec<_>>(),
                    sidebar.stars.len(),
                    updated
                        .items
                        .into_iter()
                        .map(|i| i.item.title)
                        .collect::<Vec<_>>(),
                    hits.items
                        .into_iter()
                        .map(|h| h.item.title)
                        .collect::<Vec<_>>(),
                    titles.into_iter().map(|h| h.item.title).collect::<Vec<_>>(),
                ))
            })
            .await
            .unwrap()
        }
    };
    let (cols, stars, updated, hits, titles) = seen(as_(&member, Role::Member)).await;
    assert_eq!(cols, ["Open"]);
    assert_eq!(stars, 0);
    assert_eq!(updated, ["Open plan"]);
    assert_eq!(hits, ["Open plan"]);
    assert_eq!(titles, ["Open plan"]);
    let (cols, ..) = seen(as_(&owner, Role::Owner)).await;
    assert_eq!(cols, ["Private", "Open"]);
    let (p, u, by) = (private.clone(), member.clone(), owner.clone());
    t.w(move |tx| access::set_member(tx, &p, &u, MemberAccess::Read, Some(&by), NOW))
        .await
        .unwrap();
    let (cols, stars, updated, hits, titles) = seen(as_(&member, Role::Member)).await;
    assert_eq!(cols, ["Private", "Open"]);
    assert_eq!(stars, 1);
    assert_eq!(updated.len(), 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(titles.len(), 2);
    // The collection's my_access is the member's.
    let m = as_(&member, Role::Member);
    let list = t.r(move |c| collections::list(c, &m)).await.unwrap();
    assert_eq!(
        list.iter()
            .map(|c| (c.name.as_str(), c.default_access, c.my_access))
            .collect::<Vec<_>>(),
        [
            ("Private", DefaultAccess::None, CollectionAccess::Read),
            ("Open", DefaultAccess::Write, CollectionAccess::Write),
        ]
    );
}

#[tokio::test]
async fn attachments_are_read_through_the_items_that_show_them() {
    let t = TestDb::new();
    let (owner, member, _viewer, private, doc) = private_world(&t).await;
    let open = t.collection(&owner, "Open").await;
    let shown = t.item(&owner, open.id, None, ItemKind::Doc, "Shown").await;
    // Attachment rows with a blob each, as an upload leaves them.
    let insert = |id: &'static str, created_by: String, item: String| {
        let t = &t;
        async move {
            t.w(move |tx| {
                tx.execute(
                    "INSERT OR IGNORE INTO blobs (sha256, size, mime, created_at) VALUES (?1, 1, 'image/png', 0)",
                    [format!("{:0>64}", 1)],
                )?;
                tx.execute(
                    "INSERT INTO attachments (id, item_id, sha256, filename, mime, created_by, created_at)
                     VALUES (?1, ?2, ?3, 'a.png', 'image/png', ?4, 0)",
                    rusqlite::params![id, item, format!("{:0>64}", 1), created_by],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        }
    };
    let hidden = "00000000-0000-7000-8000-0000000000a1";
    let open_one = "00000000-0000-7000-8000-0000000000a2";
    let mine = "00000000-0000-7000-8000-0000000000a3";
    let logo = "00000000-0000-7000-8000-0000000000a4";
    insert(hidden, owner.clone(), doc.clone()).await;
    insert(open_one, owner.clone(), doc.clone()).await;
    insert(mine, member.clone(), doc.clone()).await;
    insert(logo, owner.clone(), doc.clone()).await;
    let (d, s) = (doc.clone(), shown.id.to_string());
    t.w(move |tx| {
        tx.execute(
            "INSERT INTO item_attachments (item_id, attachment_id) VALUES (?1, ?2), (?3, ?4)",
            rusqlite::params![d, hidden, s, open_one],
        )?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES ('workspace.logo_attachment_id', ?1)",
            [format!("\"{logo}\"")],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let readable = |who: Viewer| {
        let t = &t;
        async move {
            t.r(move |c| {
                [
                    hidden,
                    open_one,
                    mine,
                    logo,
                    "00000000-0000-7000-8000-0000000000ff",
                ]
                .into_iter()
                .map(|id| access::attachment_readable(c, &who, id))
                .collect::<wikanvas_store::Result<Vec<bool>>>()
            })
            .await
            .unwrap()
        }
    };
    // The upload context (`attachments.item_id`, the hidden document for all four) never counts (D36).
    assert_eq!(
        readable(as_(&member, Role::Member)).await,
        [false, true, true, true, false]
    );
    assert_eq!(
        readable(as_(&owner, Role::Owner)).await,
        [true, true, true, true, false]
    );
    let (p, u, by) = (private, member.clone(), owner);
    t.w(move |tx| access::set_member(tx, &p, &u, MemberAccess::Read, Some(&by), NOW))
        .await
        .unwrap();
    assert_eq!(
        readable(as_(&member, Role::Member)).await,
        [true, true, true, true, false]
    );
}

#[tokio::test]
async fn memberships_move_the_access_epoch() {
    let t = TestDb::new();
    let (owner, member, viewer, private, _doc) = private_world(&t).await;
    let epoch = |user: String| {
        let t = &t;
        async move { t.r(move |c| access::epoch(c, &user)).await.unwrap() }
    };
    assert_eq!(epoch(member.clone()).await, 0);
    let set = |access: MemberAccess| {
        let (p, u, by, t) = (private.clone(), member.clone(), owner.clone(), &t);
        async move {
            t.w(move |tx| access::set_member(tx, &p, &u, access, Some(&by), NOW))
                .await
                .unwrap()
        }
    };
    let added = set(MemberAccess::Write).await;
    assert_eq!(
        (added.access, added.effective_access, added.disabled),
        (MemberAccess::Write, CollectionAccess::Write, false)
    );
    assert_eq!(epoch(member.clone()).await, 1);
    set(MemberAccess::Write).await;
    assert_eq!(
        epoch(member.clone()).await,
        1,
        "the same access again is no change"
    );
    set(MemberAccess::Read).await;
    assert_eq!(epoch(member.clone()).await, 2);
    let (p, u) = (private.clone(), member.clone());
    assert!(
        t.w(move |tx| access::remove_member(tx, &p, &u))
            .await
            .unwrap()
    );
    assert_eq!(epoch(member.clone()).await, 3);
    let (p, u) = (private.clone(), member.clone());
    assert!(
        !t.w(move |tx| access::remove_member(tx, &p, &u))
            .await
            .unwrap()
    );
    assert_eq!(epoch(member.clone()).await, 3);
    // A role change and disabling move it too.
    let (o, v) = (owner.clone(), viewer.clone());
    t.w(move |tx| {
        users::change(
            tx,
            Manager::User {
                id: &o,
                role: Role::Owner,
            },
            &v,
            &UserChange {
                role: Some(Role::Member),
                disabled: None,
            },
            NOW,
        )
    })
    .await
    .unwrap();
    assert_eq!(epoch(viewer.clone()).await, 1);
    let (o, v) = (owner.clone(), viewer.clone());
    t.w(move |tx| {
        users::change(
            tx,
            Manager::User {
                id: &o,
                role: Role::Owner,
            },
            &v,
            &UserChange {
                role: None,
                disabled: Some(true),
            },
            NOW,
        )
    })
    .await
    .unwrap();
    assert_eq!(epoch(viewer.clone()).await, 2);
    // The directory leaves the disabled account out; a disabled account cannot be added.
    let people = t.r(access::directory).await.unwrap();
    let mut roles: Vec<Role> = people.iter().map(|p| p.role).collect();
    roles.sort();
    assert_eq!(roles, [Role::Owner, Role::Member]);
    let (p, v, by) = (private.clone(), viewer.clone(), owner.clone());
    let e = t
        .w(move |tx| access::set_member(tx, &p, &v, MemberAccess::Read, Some(&by), NOW))
        .await
        .unwrap_err();
    assert!(e.is(wikanvas_core::api::ErrorCode::Validation));
    // A new collection gives its creator `manage` without moving anyone's epoch.
    let before = epoch(member.clone()).await;
    let m = as_(&member, Role::Member);
    let created = t
        .w(move |tx| {
            collections::create(
                tx,
                &m,
                &NewCollection {
                    name: "Mine".into(),
                    default_access: Some(DefaultAccess::Read),
                    ..Default::default()
                },
                NOW,
            )
        })
        .await
        .unwrap();
    assert_eq!(
        (created.default_access, created.my_access),
        (DefaultAccess::Read, CollectionAccess::Manage)
    );
    assert_eq!(epoch(member).await, before);
    let members = t
        .r(move |c| access::members(c, &created.id.to_string()))
        .await
        .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].access, MemberAccess::Manage);
}
