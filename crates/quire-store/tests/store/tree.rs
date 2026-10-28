//! Tree tests: CTEs, triggers, cycles, cross-collection moves, deep purges (owner: W1).
//!
//! W0b: the leaves-first purge at depth 2,000 and the parent trigger.

use quire_core::api::{ErrorCode, ItemKind};
use quire_store::items;
use rusqlite::params;

use crate::common::{DAY, NOW, TestDb, count};

#[tokio::test]
async fn purge_depth_2000_leaves_first() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Deep").await;
    let root = t.item(&owner, c.id, None, ItemKind::Doc, "root").await;
    let (cid, rid) = (c.id.to_string(), root.id.to_string());
    t.db.write(move |tx| {
        // A chain of 2,000 items under `root`, trashed with it 31 days ago. Deleting the root alone would cascade
        // 2,000 levels deep, past SQLite's limit of 1,000.
        let mut parent = rid.clone();
        let mut insert = tx.prepare(
            "INSERT INTO items (id, url_id, collection_id, parent_id, kind, title, sort_key, created_at, updated_at,
               published_at)
             VALUES (?1, ?2, ?3, ?4, 'doc', '', 'a0', ?5, ?5, ?5)",
        )?;
        for i in 0..2000 {
            let id = uuid::Uuid::now_v7().to_string();
            insert.execute(params![id, format!("deep{i:06}"), cid, parent, NOW])?;
            parent = id;
        }
        drop(insert);
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = ?1 WHERE collection_id = ?3",
            params![rid, NOW - 31 * DAY, cid],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM items WHERE collection_id = ?1",
            c.id.to_string()
        )
        .await,
        2001
    );
    let purged =
        t.db.write(|tx| items::purge_expired(tx, NOW - 30 * DAY))
            .await
            .unwrap();
    assert_eq!(purged, 2001);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM items WHERE collection_id = ?1",
            c.id.to_string()
        )
        .await,
        0
    );
    let integrity: String = t
        .query(|c| c.query_row("SELECT count(*) || '' FROM item_text", [], |r| r.get(0)))
        .await;
    assert_eq!(integrity, "0");
}

#[tokio::test]
async fn parent_trigger_rejects_trashed_or_archived_parent() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, None, ItemKind::Doc, "b").await;
    let child = t.item(&owner, c.id, None, ItemKind::Doc, "child").await;
    for (parent, state) in [
        (a.id.to_string(), "deleted_at = ?2, trash_root_id = id"),
        (b.id.to_string(), "archived_at = ?2, archive_root_id = id"),
    ] {
        let child = child.id.to_string();
        let e =
            t.db.write(move |tx| {
                tx.execute(
                    &format!("UPDATE items SET {state} WHERE id = ?1"),
                    params![parent, NOW],
                )?;
                tx.execute(
                    "UPDATE items SET parent_id = ?1 WHERE id = ?2",
                    params![parent, child],
                )?;
                Ok(())
            })
            .await
            .unwrap_err();
        // The trigger's RAISE(ABORT, 'parent_invalid') arrives as that error code.
        assert!(e.is(ErrorCode::ParentInvalid), "{state}: {e:?}");
    }
}
