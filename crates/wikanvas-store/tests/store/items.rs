//! Item tests: create, rename, trash, restore, archive, collection delete, publication, duplicate, discard,
//! rebalance, and the revision rules (auto, pre-api, prune) (owner: W1).

use wikanvas_core::api::{
    ErrorCode, ItemKind, ItemPatch, NewCollection, NewItem, Position, SearchQuery,
};
use wikanvas_store::revisions::{self, RevisionDraft};
use wikanvas_store::{WriteBy, collections, items, lists, search, shared};
use rusqlite::params;
use uuid::Uuid;

use crate::common::{DAY, MINUTE, NOW, TestDb, count, viewer};

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
    // A year later both fall into one weekly bucket: the newer one stays.
    assert_eq!(
        t.db.write(|tx| revisions::prune(tx, NOW + 365 * DAY))
            .await
            .unwrap(),
        1
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

// ---------------------------------------------------------------- W1: trash, restore, archive, collections

async fn trash(t: &TestDb, owner: &str, id: Uuid) -> wikanvas_store::Result<u64> {
    let (id, o) = (id.to_string(), owner.to_owned());
    t.w(move |tx| items::trash(tx, &id, &o, NOW)).await
}

async fn restore(t: &TestDb, id: Uuid) -> wikanvas_store::Result<wikanvas_core::api::RestoredItem> {
    let id = id.to_string();
    t.w(move |tx| items::restore(tx, &id)).await
}

#[tokio::test]
async fn trash_restore_subtree_roundtrip() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let cc = t.item(&owner, c.id, Some(b.id), ItemKind::Board, "c").await;
    let early = t
        .item(&owner, c.id, Some(b.id), ItemKind::Doc, "early")
        .await;
    t.item(&owner, c.id, None, ItemKind::Doc, "d").await;
    // `early` goes first, as its own root; then a takes the rest of the subtree.
    assert_eq!(trash(&t, &owner, early.id).await.unwrap(), 1);
    let tv = t.tree_version(c.id).await;
    let meta = t.row(b.id).await.meta_version;
    assert_eq!(trash(&t, &owner, a.id).await.unwrap(), 3);
    assert!(t.tree_version(c.id).await > tv);
    assert_eq!(t.tree_titles(&owner, c.id).await, ["d"]);
    for (id, root) in [
        (a.id, a.id),
        (b.id, a.id),
        (cc.id, a.id),
        (early.id, early.id),
    ] {
        let row = t.row(id).await;
        assert_eq!(row.trash_root_id, Some(root.to_string()), "{id}");
        assert_eq!(
            (row.deleted_at, row.deleted_by.as_deref()),
            (Some(NOW), Some(owner.as_str()))
        );
    }
    assert_eq!(t.row(b.id).await.meta_version, meta + 1);
    // Trashing again is read-only.
    assert!(
        trash(&t, &owner, b.id)
            .await
            .unwrap_err()
            .is(ErrorCode::ReadOnly)
    );
    // Restoring a descendant restores its whole unit (root a), in place.
    let restored = restore(&t, cc.id).await.unwrap();
    assert_eq!((restored.item.id, restored.moved_to_top), (a.id, false));
    assert_eq!(t.tree_titles(&owner, c.id).await, ["a", "b", "c", "d"]);
    // `early` keeps its own root and stays in the trash.
    assert_eq!(
        t.row(early.id).await.trash_root_id,
        Some(early.id.to_string())
    );
    let back = restore(&t, early.id).await.unwrap();
    assert_eq!(
        (back.item.parent_id, back.moved_to_top),
        (Some(b.id), false)
    );
    // Restoring a live item is a conflict.
    assert!(restore(&t, a.id).await.unwrap_err().is(ErrorCode::Conflict));
    assert_eq!(t.row(a.id).await.deleted_by, None);
}

#[tokio::test]
async fn restore_with_trashed_parent_goes_top() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let top = t.item(&owner, c.id, None, ItemKind::Doc, "top").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let b1 = t.item(&owner, c.id, Some(b.id), ItemKind::Doc, "b1").await;
    trash(&t, &owner, b.id).await.unwrap();
    trash(&t, &owner, a.id).await.unwrap();
    let r = restore(&t, b1.id).await.unwrap();
    assert_eq!(
        (r.item.id, r.item.parent_id, r.moved_to_top),
        (b.id, None, true)
    );
    assert!(
        r.item.sort_key > top.sort_key && r.item.sort_key > a.sort_key,
        "goes last"
    );
    assert_eq!(t.tree_titles(&owner, c.id).await, ["top", "b", "b1"]);
    // An archived parent: an unarchived root restored under it goes to the top as well.
    let p = t.item(&owner, c.id, None, ItemKind::Doc, "p").await;
    let q = t.item(&owner, c.id, Some(p.id), ItemKind::Doc, "q").await;
    trash(&t, &owner, q.id).await.unwrap();
    let pid = p.id.to_string();
    t.w(move |tx| items::archive(tx, &pid, NOW)).await.unwrap();
    let r = restore(&t, q.id).await.unwrap();
    assert!(r.moved_to_top && !r.item.archived);
    // Unarchive under an archived parent (archived after the child, as its own root): to the top.
    let u = t.item(&owner, c.id, None, ItemKind::Doc, "u").await;
    let v = t.item(&owner, c.id, Some(u.id), ItemKind::Doc, "v").await;
    let (vid, uid) = (v.id.to_string(), u.id.to_string());
    t.w(move |tx| {
        items::archive(tx, &vid, NOW)?;
        items::archive(tx, &uid, NOW)
    })
    .await
    .unwrap();
    let vid = v.id.to_string();
    let r = t.w(move |tx| items::unarchive(tx, &vid)).await.unwrap();
    assert!(r.moved_to_top, "{r:?}");
    assert_eq!(r.item.parent_id, None);
}

#[tokio::test]
async fn restored_archived_child_becomes_its_own_archive_root() {
    // Archive a, trash its child r (it stays archived, D5), trash a, restore r: r leaves a's subtree for the top
    // level, so its part of a's archive unit becomes a unit of its own, listed in /archive and unarchived by itself,
    // also after a is purged.
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let r = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "r").await;
    let d = t.item(&owner, c.id, Some(r.id), ItemKind::Doc, "d").await;
    let s = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "s").await;
    let aid = a.id.to_string();
    assert_eq!(
        t.w(move |tx| items::archive(tx, &aid, NOW)).await.unwrap(),
        4
    );
    assert_eq!(trash(&t, &owner, r.id).await.unwrap(), 2);
    assert_eq!(trash(&t, &owner, a.id).await.unwrap(), 2);
    let back = restore(&t, d.id).await.unwrap();
    assert_eq!(
        (
            back.item.id,
            back.item.parent_id,
            back.moved_to_top,
            back.item.archived
        ),
        (r.id, None, true, true)
    );
    for x in [r.id, d.id] {
        assert_eq!(t.row(x).await.archive_root_id, Some(r.id.to_string()));
    }
    assert_eq!(t.row(s.id).await.archive_root_id, Some(a.id.to_string()));
    assert_eq!(archive_list(&t, &owner).await, [r.id]);
    assert!(t.tree_titles(&owner, c.id).await.is_empty());
    let aid = a.id.to_string();
    assert_eq!(t.w(move |tx| items::purge(tx, &aid)).await.unwrap(), 2);
    let did = d.id.to_string();
    let back = t.w(move |tx| items::unarchive(tx, &did)).await.unwrap();
    assert_eq!((back.item.id, back.moved_to_top), (r.id, false));
    assert_eq!(t.tree_titles(&owner, c.id).await, ["r", "d"]);
    assert!(archive_list(&t, &owner).await.is_empty());
}

/// The ids `/archive` lists.
async fn archive_list(t: &TestDb, owner: &str) -> Vec<Uuid> {
    let v = viewer(owner);
    t.r(move |conn| lists::archive(conn, &v, &lists::PageRequest::new("archive", None, None)))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.item.id)
        .collect()
}

#[tokio::test]
async fn collection_delete_restore_roundtrip() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let arch = t.item(&owner, c.id, None, ItemKind::Board, "arch").await;
    let early = t.item(&owner, c.id, None, ItemKind::Doc, "early").await;
    trash(&t, &owner, early.id).await.unwrap();
    let aid = arch.id.to_string();
    t.w(move |tx| items::archive(tx, &aid, NOW)).await.unwrap();
    let (cid, o) = (c.id.to_string(), owner.clone());
    let trashed = t
        .w(move |tx| collections::delete(tx, &cid, &o, NOW + MINUTE))
        .await
        .unwrap();
    assert_eq!(
        trashed, 3,
        "a, b and the archived item; not the item trashed earlier"
    );
    for id in [a.id, b.id, arch.id] {
        assert_eq!(t.row(id).await.trash_root_id, Some(c.id.to_string()));
    }
    let cid = c.id.to_string();
    let (deleted, at) = t
        .r(move |conn| collections::get_with_state(conn, &cid))
        .await
        .unwrap();
    assert_eq!((deleted.item_count, at), (3, Some(NOW + MINUTE)));
    let cid = c.id.to_string();
    assert!(
        t.r(move |conn| collections::get_live(conn, &cid))
            .await
            .unwrap_err()
            .is(ErrorCode::NotFound)
    );
    let sidebar = {
        let v = viewer(&owner);
        t.r(move |conn| lists::sidebar(conn, &v)).await.unwrap()
    };
    assert!(sidebar.collections.is_empty());
    // An item of the deleted collection cannot be restored on its own.
    assert!(
        restore(&t, a.id)
            .await
            .unwrap_err()
            .is(ErrorCode::ParentInvalid)
    );
    // Deleting it again: not found (it is only in the trash).
    let (cid, o) = (c.id.to_string(), owner.clone());
    assert!(
        t.w(move |tx| collections::delete(tx, &cid, &o, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::NotFound)
    );
    let (cid, o) = (c.id.to_string(), owner.clone());
    let back = t
        .w(move |tx| collections::restore(tx, &cid, &o, NOW + 2 * MINUTE))
        .await
        .unwrap();
    assert_eq!(back.item_count, 2, "a and b; arch is archived");
    assert_eq!(t.tree_titles(&owner, c.id).await, ["a", "b"]);
    assert!(
        t.row(arch.id).await.archived_at.is_some() && t.row(arch.id).await.deleted_at.is_none()
    );
    assert_eq!(
        t.row(early.id).await.trash_root_id,
        Some(early.id.to_string()),
        "trashed earlier: stays"
    );
    let (cid, o) = (c.id.to_string(), owner.clone());
    assert!(
        t.w(move |tx| collections::restore(tx, &cid, &o, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::Conflict)
    );
    // Purge needs the collection in the trash; then everything in it goes, leaves first.
    let cid = c.id.to_string();
    assert!(
        t.w(move |tx| collections::purge(tx, &cid))
            .await
            .unwrap_err()
            .is(ErrorCode::Conflict)
    );
    let (cid, o) = (c.id.to_string(), owner.clone());
    t.w(move |tx| collections::delete(tx, &cid, &o, NOW))
        .await
        .unwrap();
    let cid = c.id.to_string();
    assert_eq!(
        t.w(move |tx| collections::purge(tx, &cid)).await.unwrap(),
        4
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM collections WHERE id = ?1",
            c.id.to_string()
        )
        .await,
        0
    );
}

#[tokio::test]
async fn create_restore_or_move_into_deleted_collection_409() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let x = t.collection(&owner, "X").await;
    let y = t.collection(&owner, "Y").await;
    let earlier = t.item(&owner, x.id, None, ItemKind::Doc, "earlier").await;
    let elsewhere = t.item(&owner, y.id, None, ItemKind::Doc, "elsewhere").await;
    trash(&t, &owner, earlier.id).await.unwrap();
    let (xid, o) = (x.id.to_string(), owner.clone());
    t.w(move |tx| collections::delete(tx, &xid, &o, NOW))
        .await
        .unwrap();
    let o = owner.clone();
    let e = t
        .w(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: x.id,
                    parent_id: None,
                    kind: ItemKind::Doc,
                    title: None,
                    position: None,
                },
                false,
                NOW,
            )
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "create: {e:?}");
    let e = restore(&t, earlier.id).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "restore: {e:?}");
    let (eid, o) = (elsewhere.id.to_string(), owner.clone());
    let e = t
        .w(move |tx| {
            items::move_item(
                tx,
                &eid,
                &wikanvas_core::api::MoveItem {
                    collection_id: Some(x.id),
                    parent_id: None,
                    position: None,
                },
                &o,
                NOW,
            )
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "move: {e:?}");
    // After the collection is restored, the earlier item restores too.
    let (xid, o) = (x.id.to_string(), owner.clone());
    t.w(move |tx| collections::restore(tx, &xid, &o, NOW))
        .await
        .unwrap();
    assert!(!restore(&t, earlier.id).await.unwrap().moved_to_top);
}

#[tokio::test]
async fn archive_hides_subtree_from_tree_and_default_search() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "alpha").await;
    let b = t
        .item(&owner, c.id, Some(a.id), ItemKind::Doc, "quokka habits")
        .await;
    let gone = t
        .item(&owner, c.id, Some(a.id), ItemKind::Doc, "trashed child")
        .await;
    t.item(&owner, c.id, None, ItemKind::Doc, "other").await;
    trash(&t, &owner, gone.id).await.unwrap();
    let aid = a.id.to_string();
    assert_eq!(
        t.w(move |tx| items::archive(tx, &aid, NOW)).await.unwrap(),
        2,
        "not the trashed child"
    );
    assert_eq!(t.tree_titles(&owner, c.id).await, ["other"]);
    assert_eq!(t.row(b.id).await.archive_root_id, Some(a.id.to_string()));
    assert_eq!(t.row(gone.id).await.archived_at, None);
    let find = |include_archived: bool| {
        let v = viewer(&owner);
        t.r(move |conn| {
            search::search(
                conn,
                &v,
                &SearchQuery {
                    q: "quokka".into(),
                    include_archived,
                    ..Default::default()
                },
            )
        })
    };
    assert!(find(false).await.unwrap().items.is_empty());
    assert_eq!(find(true).await.unwrap().items[0].item.id, b.id);
    let v = viewer(&owner);
    let archived = t
        .r(move |conn| lists::archive(conn, &v, &lists::PageRequest::new("archive", None, None)))
        .await
        .unwrap();
    assert_eq!(archived.items.len(), 1, "roots only");
    assert_eq!(archived.items[0].item.id, a.id);
    assert_eq!(archived.items[0].archived_at.map(|t| t.millis()), Some(NOW));
    // Archived items are read-only.
    let (bid, o) = (b.id.to_string(), owner.clone());
    assert!(
        t.w(move |tx| items::rename(tx, &bid, "x", &o, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::ReadOnly)
    );
    let aid = a.id.to_string();
    assert!(
        t.w(move |tx| items::archive(tx, &aid, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::ReadOnly)
    );
    // Unarchiving a descendant unarchives the unit.
    let bid = b.id.to_string();
    let r = t.w(move |tx| items::unarchive(tx, &bid)).await.unwrap();
    assert_eq!((r.item.id, r.moved_to_top), (a.id, false));
    assert_eq!(
        t.tree_titles(&owner, c.id).await,
        ["alpha", "quokka habits", "other"]
    );
    let bid = b.id.to_string();
    assert!(
        t.w(move |tx| items::unarchive(tx, &bid))
            .await
            .unwrap_err()
            .is(ErrorCode::Conflict)
    );
}

#[tokio::test]
async fn publish_and_unpublish() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let leaf = t
        .item(&owner, c.id, Some(b.id), ItemKind::Doc, "leaf")
        .await;
    let set = |id: Uuid, publish: Option<bool>| {
        let (id, o) = (id.to_string(), owner.clone());
        t.w(move |tx| match publish {
            None => items::unpublish(tx, &id, &o, NOW + MINUTE),
            Some(with_ancestors) => items::publish(tx, &id, with_ancestors, &o, NOW + 2 * MINUTE),
        })
    };
    // Unpublish works whatever the children are, and twice.
    assert!(!set(a.id, None).await.unwrap().published);
    assert!(!set(b.id, None).await.unwrap().published);
    assert!(!set(b.id, None).await.unwrap().published);
    let check = |id: Uuid| {
        let id = id.to_string();
        t.r(move |c| shared::effectively_published(c, &id))
    };
    assert!(!check(leaf.id).await.unwrap() && t.row(leaf.id).await.published_at.is_some());
    let v = viewer(&owner);
    assert_eq!(
        t.r(move |conn| lists::drafts_count(conn, &v))
            .await
            .unwrap(),
        2
    );
    // Publishing the leaf alone leaves the drafts; with ancestors, all are published.
    set(leaf.id, Some(false)).await.unwrap();
    assert!(!check(leaf.id).await.unwrap());
    set(leaf.id, Some(true)).await.unwrap();
    assert!(check(leaf.id).await.unwrap());
    assert_eq!(t.row(a.id).await.published_at, Some(NOW + 2 * MINUTE));
    assert_eq!(
        t.row(leaf.id).await.published_at,
        Some(NOW),
        "an already published item keeps its date"
    );
}

#[tokio::test]
async fn duplicate_copies_content_after_the_original() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "Plan").await;
    let after = t.item(&owner, c.id, None, ItemKind::Doc, "After").await;
    t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "child")
        .await;
    let board = t.item(&owner, c.id, None, ItemKind::Board, "").await;
    let aid = a.id.to_string();
    t.w(move |tx| shared::set_body_text(tx, &aid, "the body"))
        .await
        .unwrap();
    let (aid, o) = (a.id.to_string(), owner.clone());
    let copy = t
        .w(move |tx| items::duplicate(tx, &aid, &o, NOW + MINUTE))
        .await
        .unwrap();
    assert_eq!(
        (copy.title.as_str(), copy.child_count, copy.published),
        ("Plan (copy)", 0, true)
    );
    assert!(a.sort_key < copy.sort_key && copy.sort_key < after.sort_key);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'the body'",
            copy.id.to_string()
        )
        .await,
        1
    );
    let (bid, o) = (board.id.to_string(), owner.clone());
    let bcopy = t
        .w(move |tx| items::duplicate(tx, &bid, &o, NOW))
        .await
        .unwrap();
    assert_eq!((bcopy.title.as_str(), bcopy.kind), ("", ItemKind::Board));
    assert_eq!(
        count(
            &t,
            "SELECT rev FROM board_state WHERE item_id = ?1",
            bcopy.id.to_string()
        )
        .await,
        1
    );
    trash(&t, &owner, a.id).await.unwrap();
    let (aid, o) = (a.id.to_string(), owner.clone());
    assert!(
        t.w(move |tx| items::duplicate(tx, &aid, &o, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::ReadOnly)
    );
}

#[tokio::test]
async fn discard_empty_rules() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let discard = |id: Uuid, now: i64| {
        let id = id.to_string();
        t.w(move |tx| items::discard_empty(tx, &id, now))
    };
    let fresh = t.item(&owner, c.id, None, ItemKind::Doc, "").await;
    let titled = t.item(&owner, c.id, None, ItemKind::Doc, "Title").await;
    let parent = t.item(&owner, c.id, None, ItemKind::Board, " ").await;
    t.item(&owner, c.id, Some(parent.id), ItemKind::Doc, "")
        .await;
    let old = t.item(&owner, c.id, None, ItemKind::Doc, "").await;
    // A board with one object has a body.
    let body = t.item(&owner, c.id, None, ItemKind::Board, "").await;
    let bid = body.id.to_string();
    t.w(move |tx| {
        tx.execute(
            "UPDATE board_state SET meta = json_set(meta, '$.objects', 1) WHERE item_id = ?1",
            [bid],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    for (id, now) in [
        (titled.id, NOW),
        (parent.id, NOW),
        (old.id, NOW + DAY),
        (body.id, NOW),
    ] {
        let e = discard(id, now).await.unwrap_err();
        assert!(e.is(ErrorCode::NotEmpty), "{id}: {e:?}");
    }
    assert_eq!(discard(fresh.id, NOW + DAY - 1).await.unwrap(), 1);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM items WHERE id = ?1",
            fresh.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM doc_state WHERE item_id = ?1",
            fresh.id.to_string()
        )
        .await,
        0
    );
    // An unpublished one is discarded as well.
    let draft = t.item(&owner, c.id, None, ItemKind::Board, "").await;
    let (did, o) = (draft.id.to_string(), owner.clone());
    t.w(move |tx| items::unpublish(tx, &did, &o, NOW))
        .await
        .unwrap();
    assert_eq!(discard(draft.id, NOW).await.unwrap(), 1);
}

#[tokio::test]
async fn rebalance_when_key_over_64() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, None, ItemKind::Doc, "b").await;
    let z = t.item(&owner, c.id, None, ItemKind::Doc, "z").await;
    // b sits so close after a that the key between them is 66 characters long.
    let long = format!("a0{}1", "0".repeat(62));
    let (aid, bid, l) = (a.id.to_string(), b.id.to_string(), long.clone());
    t.w(move |tx| {
        tx.execute("UPDATE items SET sort_key = 'a0' WHERE id = ?1", [&aid])?;
        tx.execute(
            "UPDATE items SET sort_key = ?2 WHERE id = ?1",
            params![bid, l],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert!(
        wikanvas_core::rank::key_between(Some("a0"), Some(&long))
            .unwrap()
            .len()
            > 64
    );
    let tv = t.tree_version(c.id).await;
    let o = owner.clone();
    let mid = t
        .w(move |tx| {
            items::create(
                tx,
                &o,
                &NewItem {
                    collection_id: c.id,
                    parent_id: None,
                    kind: ItemKind::Doc,
                    title: Some("mid".into()),
                    position: Some(Position {
                        after_id: Some(a.id),
                        before_id: Some(b.id),
                    }),
                },
                false,
                NOW,
            )
        })
        .await
        .unwrap();
    assert!(mid.sort_key.len() <= 4, "{}", mid.sort_key);
    for id in [a.id, b.id, z.id] {
        assert!(t.row(id).await.sort_key.len() <= 4);
    }
    assert!(t.tree_version(c.id).await > tv);
    assert_eq!(t.tree_titles(&owner, c.id).await, ["a", "mid", "b", "z"]);
    // Collections rebalance the same way.
    let first = t.collection(&owner, "first").await;
    let (fid, cid) = (first.id.to_string(), c.id.to_string());
    t.w(move |tx| {
        tx.execute(
            "UPDATE collections SET sort_key = 'a0' WHERE id = ?1",
            [&fid],
        )?;
        tx.execute(
            "UPDATE collections SET sort_key = ?2 WHERE id = ?1",
            params![cid, long],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let (tv_first, tv_notes) = (t.tree_version(first.id).await, t.tree_version(c.id).await);
    let o = owner.clone();
    let between = t
        .w(move |tx| {
            collections::create(
                tx,
                &o,
                &NewCollection {
                    name: "between".into(),
                    position: Some(Position {
                        after_id: Some(first.id),
                        before_id: Some(c.id),
                    }),
                    ..Default::default()
                },
                NOW,
            )
        })
        .await
        .unwrap();
    let v = viewer(&owner);
    let all = t.r(move |conn| collections::list(conn, &v)).await.unwrap();
    let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["first", "between", "Notes"]);
    assert!(all.iter().all(|c| c.sort_key.len() <= 4), "{all:?}");
    assert!(between.sort_key.len() <= 4);
    // Each tree response holds its collection's sort_key, so a collection whose key the rebalance rewrote gets a new
    // tree ETag (and one whose key stayed does not).
    assert_eq!(
        t.tree_version(first.id).await > tv_first,
        all[0].sort_key != "a0"
    );
    assert!(
        t.tree_version(c.id).await > tv_notes,
        "Notes got a short key"
    );
}

#[tokio::test]
async fn patch_with_if_match() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let patch = |p: ItemPatch, if_match: Option<i64>| {
        let (id, o) = (a.id.to_string(), owner.clone());
        t.w(move |tx| items::update(tx, &id, &p, if_match, &o, NOW))
    };
    let tv = t.tree_version(c.id).await;
    let s = patch(
        ItemPatch {
            title: None,
            full_width: Some(true),
        },
        Some(1),
    )
    .await
    .unwrap();
    assert_eq!(s.title, "a");
    assert_eq!(t.row(a.id).await.meta_version, 2);
    assert_eq!(
        t.tree_version(c.id).await,
        tv + 1,
        "full_width alone changes updated_at, which the tree shows and the `updated` order sorts by"
    );
    let e = patch(
        ItemPatch {
            title: Some("b".into()),
            full_width: None,
        },
        Some(1),
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::RevMismatch));
    assert_eq!(e.details, Some(serde_json::json!({"etag": "\"m2\""})));
    patch(
        ItemPatch {
            title: Some("b".into()),
            full_width: Some(false),
        },
        Some(2),
    )
    .await
    .unwrap();
    assert_eq!(
        t.row(a.id).await.meta_version,
        3,
        "one bump for both fields"
    );
    // An empty patch, or one that sets the current values, changes nothing.
    let tv = t.tree_version(c.id).await;
    patch(ItemPatch::default(), None).await.unwrap();
    patch(
        ItemPatch {
            title: Some("b".into()),
            full_width: Some(false),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(t.row(a.id).await.meta_version, 3);
    assert_eq!(t.tree_version(c.id).await, tv);
}

#[tokio::test]
async fn prune_thins_by_age() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Doc").await;
    let d = doc.id.to_string();
    let hour = 60 * MINUTE;
    t.w(move |tx| {
        let at = |age: i64| NOW - age;
        // Under 24 h: all four stay.
        for m in [1, 2, 3, 4] {
            revisions::write(
                tx,
                &d,
                revisions::RevisionReason::Auto,
                yjs_draft(m),
                at(m * MINUTE),
            )?;
        }
        // Two days old: three in one hour bucket → one, plus one in the next hour.
        let base = (NOW - 2 * DAY).div_euclid(hour) * hour;
        for m in [5, 10, 20] {
            revisions::write(
                tx,
                &d,
                revisions::RevisionReason::PreApi,
                yjs_draft(m),
                base + m * MINUTE,
            )?;
        }
        revisions::write(
            tx,
            &d,
            revisions::RevisionReason::Auto,
            yjs_draft(99),
            base + hour + MINUTE,
        )?;
        // Ten days old: two in one day → one; a manual one in the same day is never pruned.
        let day = (NOW - 10 * DAY).div_euclid(DAY) * DAY;
        revisions::write(
            tx,
            &d,
            revisions::RevisionReason::Auto,
            yjs_draft(100),
            day + hour,
        )?;
        revisions::write(
            tx,
            &d,
            revisions::RevisionReason::Auto,
            yjs_draft(101),
            day + 2 * hour,
        )?;
        revisions::write(
            tx,
            &d,
            revisions::RevisionReason::Manual,
            yjs_draft(102),
            day + 3 * hour,
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(t.w(|tx| revisions::prune(tx, NOW)).await.unwrap(), 3);
    let d = doc.id.to_string();
    let mut left: Vec<i64> = t
        .query(move |c| {
            c.prepare("SELECT source_seq FROM revisions WHERE item_id = ?1")?
                .query_map([d], |r| r.get(0))?
                .collect()
        })
        .await;
    left.sort();
    assert_eq!(
        left,
        [1, 2, 3, 4, 20, 99, 101, 102],
        "the newest of each bucket"
    );
    assert_eq!(
        t.w(|tx| revisions::prune(tx, NOW)).await.unwrap(),
        0,
        "stable"
    );
}
