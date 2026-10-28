//! Item tests: trash, restore, archive, collection delete, publication, rebalance (owner: W1).
//!
//! W0b: create (published on create, content initialized, positions, parent rules), rename (meta and tree versions),
//! purge of expired trash roots, the shared write helpers, and the revision rules (auto, pre-api).

use quire_core::api::{ErrorCode, ItemKind, NewCollection, NewItem, Position};
use quire_store::revisions::{self, RevisionDraft};
use quire_store::{WriteBy, collections, items, shared};
use rusqlite::params;

use crate::common::{DAY, MINUTE, NOW, TestDb, count};

async fn tree_version(t: &TestDb, collection: String) -> i64 {
    count(
        t,
        "SELECT tree_version FROM collections WHERE id = ?1",
        collection,
    )
    .await
}

#[tokio::test]
async fn create_is_published_and_initializes_content() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let before = tree_version(&t, c.id.to_string()).await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "First").await;
    assert!(doc.published && !doc.archived);
    assert_eq!(doc.created_at, doc.updated_at);
    assert_eq!(doc.content_at, None);
    assert_eq!((doc.title.as_str(), doc.child_count), ("First", 0));
    assert_eq!(doc.url_id.len(), 10);
    assert_eq!(tree_version(&t, c.id.to_string()).await, before + 1);

    let id = doc.id.to_string();
    let (published_at, created_at, ydoc, next_seq): (i64, i64, Vec<u8>, i64) = t
        .query(move |c| {
            c.query_row(
                "SELECT i.published_at, i.created_at, d.ydoc, d.next_seq FROM items i
                 JOIN doc_state d ON d.item_id = i.id WHERE i.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await;
    assert_eq!(published_at, created_at, "D23: published_at = created_at");
    assert_eq!((ydoc, next_seq), (vec![0, 0], 1));
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND title = 'First'",
            doc.id.to_string()
        )
        .await,
        1
    );

    let board = t
        .item(&owner, c.id, Some(doc.id), ItemKind::Board, "Sketch")
        .await;
    assert_eq!(board.parent_id, Some(doc.id));
    let id = board.id.to_string();
    let (rev, engine): (i64, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT rev, engine_version FROM board_state WHERE item_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await;
    assert_eq!((rev, engine.as_str()), (1, "server"));
    let parent = doc.id.to_string();
    let summary =
        t.db.read(move |c| items::summary(c, &parent))
            .await
            .unwrap();
    assert_eq!(summary.child_count, 1);
}

#[tokio::test]
async fn create_positions_among_siblings() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, None, ItemKind::Doc, "b").await;
    assert!(a.sort_key < b.sort_key);
    let (o, cid) = (owner.clone(), c.id);
    let between =
        t.db.write(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: cid,
                    parent_id: None,
                    kind: ItemKind::Doc,
                    title: None,
                    position: Some(Position {
                        after_id: Some(a.id),
                        before_id: None,
                    }),
                },
                false,
                NOW,
            )
        })
        .await
        .unwrap();
    assert!(a.sort_key < between.sort_key && between.sort_key < b.sort_key);
    assert_eq!(between.title, "");
    let (o, cid) = (owner.clone(), c.id);
    let first =
        t.db.write(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: cid,
                    parent_id: None,
                    kind: ItemKind::Board,
                    title: None,
                    position: Some(Position {
                        after_id: None,
                        before_id: Some(a.id),
                    }),
                },
                true,
                NOW,
            )
        })
        .await
        .unwrap();
    assert!(first.sort_key < a.sort_key);
    // A position naming an item that is not a sibling is a validation error.
    let (o, cid) = (owner.clone(), c.id);
    let e =
        t.db.write(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: cid,
                    parent_id: Some(a.id),
                    kind: ItemKind::Doc,
                    title: None,
                    position: Some(Position {
                        after_id: Some(b.id),
                        before_id: None,
                    }),
                },
                false,
                NOW,
            )
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation), "{e:?}");
}

#[tokio::test]
async fn create_into_deleted_collection_or_bad_parent_is_parent_invalid() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let other = t.collection(&owner, "Other").await;
    let foreign = t
        .item(&owner, other.id, None, ItemKind::Doc, "elsewhere")
        .await;
    let archived = t.item(&owner, c.id, None, ItemKind::Doc, "archived").await;
    let (aid, cid) = (archived.id.to_string(), c.id.to_string());
    t.db.write(move |tx| {
        tx.execute(
            "UPDATE items SET archived_at = ?2, archive_root_id = id WHERE id = ?1",
            params![aid, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let create = |collection, parent| {
        let o = owner.clone();
        t.db.write(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: collection,
                    parent_id: parent,
                    kind: ItemKind::Doc,
                    title: None,
                    position: None,
                },
                false,
                NOW,
            )
        })
    };
    for parent in [
        Some(foreign.id),
        Some(archived.id),
        Some(uuid::Uuid::now_v7()),
    ] {
        let e = create(c.id, parent).await.unwrap_err();
        assert!(e.is(ErrorCode::ParentInvalid), "{parent:?}: {e:?}");
    }
    let e = create(uuid::Uuid::now_v7(), None).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid));
    t.db.write(move |tx| {
        tx.execute(
            "UPDATE collections SET deleted_at = ?2 WHERE id = ?1",
            params![cid, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let e = create(c.id, None).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
    let o = owner.clone();
    let e =
        t.db.write(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: other.id,
                    parent_id: None,
                    kind: ItemKind::Doc,
                    title: Some("x".repeat(1001)),
                    position: None,
                },
                false,
                NOW,
            )
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
}

#[tokio::test]
async fn rename_bumps_meta_and_tree_version() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Old").await;
    let tv = tree_version(&t, c.id.to_string()).await;
    let (id, o) = (doc.id.to_string(), owner.clone());
    let renamed =
        t.db.write(move |tx| items::rename(tx, &id, "New", &o, NOW + MINUTE))
            .await
            .unwrap();
    assert_eq!(renamed.title, "New");
    assert_eq!(renamed.updated_at.millis(), NOW + MINUTE);
    assert_eq!(tree_version(&t, c.id.to_string()).await, tv + 1);
    assert_eq!(
        count(
            &t,
            "SELECT meta_version FROM items WHERE id = ?1",
            doc.id.to_string()
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND title = 'New'",
            doc.id.to_string()
        )
        .await,
        1
    );
    // The same title again changes nothing.
    let (id, o) = (doc.id.to_string(), owner.clone());
    let same =
        t.db.write(move |tx| items::rename(tx, &id, "New", &o, NOW + 2 * MINUTE))
            .await
            .unwrap();
    assert_eq!(same.updated_at.millis(), NOW + MINUTE);
    assert_eq!(tree_version(&t, c.id.to_string()).await, tv + 1);
    assert_eq!(
        count(
            &t,
            "SELECT meta_version FROM items WHERE id = ?1",
            doc.id.to_string()
        )
        .await,
        2
    );
    // A trashed item is read-only; an unknown one is not found.
    let (id, o) = (doc.id.to_string(), owner.clone());
    let e =
        t.db.write(move |tx| {
            tx.execute(
                "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
                params![id, NOW],
            )?;
            items::rename(tx, &id, "Newer", &o, NOW)
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ReadOnly), "{e:?}");
    let o = owner.clone();
    let e =
        t.db.write(move |tx| items::rename(tx, &uuid::Uuid::now_v7().to_string(), "x", &o, NOW))
            .await
            .unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}

#[tokio::test]
async fn purge_expired_deletes_old_roots_leaves_first() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let gone = t.collection(&owner, "Gone").await;
    let old_root = t.item(&owner, c.id, None, ItemKind::Doc, "old").await;
    let child = t
        .item(&owner, c.id, Some(old_root.id), ItemKind::Board, "child")
        .await;
    let grandchild = t
        .item(&owner, c.id, Some(child.id), ItemKind::Doc, "grandchild")
        .await;
    let recent = t.item(&owner, c.id, None, ItemKind::Doc, "recent").await;
    let live = t.item(&owner, c.id, None, ItemKind::Doc, "live").await;
    let in_gone = t
        .item(&owner, gone.id, None, ItemKind::Doc, "in gone")
        .await;
    t.item(
        &owner,
        gone.id,
        Some(in_gone.id),
        ItemKind::Doc,
        "deep in gone",
    )
    .await;
    let ids = (
        old_root.id.to_string(),
        recent.id.to_string(),
        gone.id.to_string(),
        c.id.to_string(),
    );
    t.db.write(move |tx| {
        let (old, recent, gone, c) = ids;
        let old_time = NOW - 31 * DAY;
        // The old subtree, trashed 31 days ago as one unit (root = old).
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = ?1
             WHERE id = ?1 OR parent_id = ?1 OR parent_id IN (SELECT id FROM items WHERE parent_id = ?1)",
            params![old, old_time],
        )?;
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
            params![recent, NOW - DAY],
        )?;
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = ?1 WHERE collection_id = ?1",
            params![gone, old_time],
        )?;
        tx.execute(
            "UPDATE collections SET deleted_at = ?2 WHERE id = ?1",
            params![gone, old_time],
        )?;
        let _ = c;
        Ok(())
    })
    .await
    .unwrap();
    let purged =
        t.db.write(|tx| items::purge_expired(tx, NOW - 30 * DAY))
            .await
            .unwrap();
    assert_eq!(purged, 5);
    for (id, expect) in [
        (old_root.id, 0),
        (child.id, 0),
        (grandchild.id, 0),
        (recent.id, 1),
        (live.id, 1),
        (in_gone.id, 0),
    ] {
        assert_eq!(
            count(
                &t,
                "SELECT count(*) FROM items WHERE id = ?1",
                id.to_string()
            )
            .await,
            expect,
            "{id}"
        );
    }
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM collections WHERE id = ?1",
            gone.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM collections WHERE id = ?1",
            c.id.to_string()
        )
        .await,
        1
    );
    // Content rows went with their items.
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM doc_state WHERE item_id = ?1",
            old_root.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM board_state WHERE item_id = ?1",
            child.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1",
            grandchild.id.to_string()
        )
        .await,
        0
    );
    // Nothing left to purge.
    assert_eq!(
        t.db.write(|tx| items::purge_expired(tx, NOW - 30 * DAY))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn shared_write_helpers() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Doc").await;
    let board = t.item(&owner, c.id, None, ItemKind::Board, "Board").await;
    let (d, b, o) = (doc.id.to_string(), board.id.to_string(), owner.clone());
    t.db.write(move |tx| {
        let row = shared::ensure_writable(tx, &d, ItemKind::Doc)?;
        assert_eq!(row.title, "Doc");
        let e = shared::ensure_writable(tx, &b, ItemKind::Doc).unwrap_err();
        assert!(e.is(ErrorCode::Validation));
        let e = shared::ensure_writable(tx, "nope", ItemKind::Doc).unwrap_err();
        assert!(e.is(ErrorCode::NotFound));
        shared::touch_content(tx, &d, &o, NOW + MINUTE)?;
        shared::set_body_text(tx, &d, "hello body")?;
        shared::set_body_text(tx, &d, "hello body")?;
        assert!(
            shared::touch_content(tx, "nope", &o, NOW)
                .unwrap_err()
                .is(ErrorCode::NotFound)
        );
        tx.execute(
            "UPDATE items SET archived_at = ?2, archive_root_id = id WHERE id = ?1",
            params![b, NOW],
        )?;
        let e = shared::ensure_writable(tx, &b, ItemKind::Board).unwrap_err();
        assert!(e.is(ErrorCode::ReadOnly));
        Ok(())
    })
    .await
    .unwrap();
    let row =
        t.db.read({
            let d = doc.id.to_string();
            move |c| shared::item_row(c, &d)
        })
        .await
        .unwrap();
    assert_eq!(
        (row.content_at, row.updated_at, row.updated_by.as_deref()),
        (Some(NOW + MINUTE), NOW + MINUTE, Some(owner.as_str()))
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'hello body'",
            doc.id.to_string()
        )
        .await,
        1
    );
}

fn yjs_draft(seq: i64) -> RevisionDraft {
    RevisionDraft::yjs(vec![0, 0], seq, vec![], None)
}

#[tokio::test]
async fn maybe_auto_writes_at_most_every_10_minutes() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Doc").await;
    let d = doc.id.to_string();
    let written =
        t.db.write(move |tx| {
            Ok([
                revisions::maybe_auto(tx, &d, yjs_draft(1), NOW)?,
                revisions::maybe_auto(tx, &d, yjs_draft(2), NOW + 5 * MINUTE)?,
                revisions::maybe_auto(tx, &d, yjs_draft(3), NOW + 10 * MINUTE)?,
                revisions::maybe_auto(tx, &d, yjs_draft(4), NOW + 10 * MINUTE + 1)?,
            ])
        })
        .await
        .unwrap();
    assert_eq!(written, [true, false, false, true]);
    let d = doc.id.to_string();
    let rows: Vec<(String, i64, String, String)> = t
        .query(move |c| {
            c.prepare(
                "SELECT reason, source_seq, title, format FROM revisions WHERE item_id = ?1 ORDER BY created_at",
            )?
            .query_map([d], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect()
        })
        .await;
    assert_eq!(
        rows,
        [
            ("auto".into(), 1, "Doc".into(), "yjs".into()),
            ("auto".into(), 4, "Doc".into(), "yjs".into())
        ]
    );
    assert_eq!(
        t.db.write(|tx| revisions::prune(tx, NOW + 365 * DAY))
            .await
            .unwrap(),
        0
    );
}

/// Appends a Yjs update to the document as a content write by `token` (`None`: the session).
fn simulate_doc_write(
    tx: &rusqlite::Transaction<'_>,
    item: &str,
    token: Option<&str>,
) -> rusqlite::Result<()> {
    let seq: i64 = tx.query_row(
        "SELECT next_seq FROM doc_state WHERE item_id = ?1",
        [item],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO doc_updates (item_id, seq, data, user_id, created_at) VALUES (?1, ?2, ?3, NULL, ?4)",
        params![item, seq, vec![0u8, 0], NOW],
    )?;
    tx.execute(
        "UPDATE doc_state SET next_seq = next_seq + 1, last_token_id = ?2 WHERE item_id = ?1",
        params![item, token],
    )?;
    Ok(())
}

#[tokio::test]
async fn pre_api_rule() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Doc").await;
    let (d, o) = (doc.id.to_string(), owner.clone());
    let steps =
        t.db.write(move |tx| {
            let session = WriteBy {
                user_id: &o,
                token_id: None,
            };
            let t1 = WriteBy {
                user_id: &o,
                token_id: Some("token-1"),
            };
            let t2 = WriteBy {
                user_id: &o,
                token_id: Some("token-2"),
            };
            let mut steps = Vec::new();
            // A session write never makes a recovery point.
            steps.push(revisions::pre_api(tx, &d, ItemKind::Doc, &session, NOW)?);
            simulate_doc_write(tx, &d, None)?;
            // An owner edit, then a token write: one pre-api revision of the owner's state.
            steps.push(revisions::pre_api(tx, &d, ItemKind::Doc, &t1, NOW)?);
            simulate_doc_write(tx, &d, Some("token-1"))?;
            // The same token's next write: none.
            steps.push(revisions::pre_api(tx, &d, ItemKind::Doc, &t1, NOW)?);
            simulate_doc_write(tx, &d, Some("token-1"))?;
            // Another token, while the newest revision already holds the current state (seq 3): none.
            revisions::maybe_auto(tx, &d, yjs_draft(3), NOW + 20 * MINUTE)?;
            steps.push(revisions::pre_api(tx, &d, ItemKind::Doc, &t2, NOW)?);
            // The owner edits again; now the other token gets a recovery point.
            simulate_doc_write(tx, &d, None)?;
            steps.push(revisions::pre_api(tx, &d, ItemKind::Doc, &t2, NOW)?);
            Ok(steps)
        })
        .await
        .unwrap();
    assert_eq!(steps, [false, true, false, false, true]);
    let d = doc.id.to_string();
    let rows: Vec<(String, i64, Vec<u8>)> = t
        .query(move |c| {
            c.prepare("SELECT reason, source_seq, data FROM revisions WHERE item_id = ?1 ORDER BY created_at, id")?
                .query_map([d], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .await;
    let summary: Vec<(&str, i64)> = rows.iter().map(|(r, s, _)| (r.as_str(), *s)).collect();
    assert_eq!(summary, [("pre-api", 1), ("pre-api", 4), ("auto", 3)]);
    // The recovery point holds a decodable full state.
    assert!(rows.iter().all(|(_, _, data)| !data.is_empty()));
}

#[tokio::test]
async fn pre_api_rule_for_boards_uses_the_rev() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let board = t.item(&owner, c.id, None, ItemKind::Board, "Board").await;
    let (b, o) = (board.id.to_string(), owner.clone());
    let steps = t
        .db
        .write(move |tx| {
            let tok = WriteBy {
                user_id: &o,
                token_id: Some("token-1"),
            };
            let mut steps = vec![revisions::pre_api(tx, &b, ItemKind::Board, &tok, NOW)?];
            tx.execute(
                "UPDATE board_state SET rev = rev + 1, last_token_id = 'token-1' WHERE item_id = ?1",
                [&b],
            )?;
            steps.push(revisions::pre_api(tx, &b, ItemKind::Board, &tok, NOW)?);
            // The wrong kind is not found.
            assert!(
                revisions::pre_api(tx, &b, ItemKind::Doc, &tok, NOW)
                    .unwrap_err()
                    .is(ErrorCode::NotFound)
            );
            Ok(steps)
        })
        .await
        .unwrap();
    assert_eq!(steps, [true, false]);
    let b = board.id.to_string();
    let (format, version, source, assets): (String, i64, i64, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT format, format_version, source_seq, assets FROM revisions WHERE item_id = ?1",
                [b],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await;
    assert_eq!(
        (format.as_str(), version, source, assets.as_str()),
        ("inkboard", 2, 1, "{}")
    );
}

#[tokio::test]
async fn collections_create_validates_and_orders() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let first = t.collection(&owner, "  First  ").await;
    assert_eq!(first.name, "First");
    assert_eq!(
        (
            first.color.as_str(),
            first.description.as_str(),
            first.icon.as_deref()
        ),
        ("#8D8D8D", "", None)
    );
    assert_eq!((first.item_count, first.url_id.len()), (0, 10));
    assert_eq!(first.updated_at.millis(), NOW);
    let create = |new: NewCollection| {
        let o = owner.clone();
        t.db.write(move |tx| collections::create(tx, &o, &new, NOW))
    };
    let second = create(NewCollection {
        name: "Second".into(),
        icon: Some("book-open".into()),
        color: Some("#00aaFF".into()),
        description: Some("about".into()),
        position: Some(Position {
            after_id: None,
            before_id: Some(first.id),
        }),
    })
    .await
    .unwrap();
    assert!(second.sort_key < first.sort_key);
    assert_eq!(
        (second.icon.as_deref(), second.color.as_str()),
        (Some("book-open"), "#00aaFF")
    );
    for (new, field) in [
        (
            NewCollection {
                name: " ".into(),
                ..Default::default()
            },
            "name",
        ),
        (
            NewCollection {
                name: "n".repeat(101),
                ..Default::default()
            },
            "name",
        ),
        (
            NewCollection {
                name: "x".into(),
                color: Some("red".into()),
                ..Default::default()
            },
            "color",
        ),
        (
            NewCollection {
                name: "x".into(),
                icon: Some("Bad Icon".into()),
                ..Default::default()
            },
            "icon",
        ),
        (
            NewCollection {
                name: "x".into(),
                description: Some("d".repeat(501)),
                ..Default::default()
            },
            "description",
        ),
    ] {
        let e = create(new).await.unwrap_err();
        assert_eq!(
            e.details,
            Some(serde_json::json!({ "field": field })),
            "{e:?}"
        );
    }
    t.item(&owner, first.id, None, ItemKind::Doc, "one").await;
    let id = first.id.to_string();
    let again = t.db.read(move |c| collections::get(c, &id)).await.unwrap();
    assert_eq!(again.item_count, 1);
    assert!(
        t.db.read(|c| collections::get(c, "nope"))
            .await
            .unwrap_err()
            .is(ErrorCode::NotFound)
    );
}

#[tokio::test]
async fn effectively_published_needs_published_ancestors() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let leaf = t
        .item(&owner, c.id, Some(b.id), ItemKind::Board, "leaf")
        .await;
    let check = |id: uuid::Uuid| {
        let id = id.to_string();
        t.db.read(move |c| shared::effectively_published(c, &id))
    };
    assert!(check(leaf.id).await.unwrap(), "published on create (D23)");
    let aid = a.id.to_string();
    t.db.write(move |tx| {
        tx.execute("UPDATE items SET published_at = NULL WHERE id = ?1", [aid])?;
        Ok(())
    })
    .await
    .unwrap();
    assert!(!check(leaf.id).await.unwrap());
    assert!(!check(a.id).await.unwrap());
    let e = check(uuid::Uuid::now_v7()).await.unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}
