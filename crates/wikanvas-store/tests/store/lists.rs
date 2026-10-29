//! List tests: visible_items, cursor pages and the lists (owner: W1).

use rusqlite::params;
use uuid::Uuid;
use wikanvas_core::api::{ErrorCode, ItemKind, Position, Role};
use wikanvas_store::lists::{self, PageRequest};
use wikanvas_store::{Viewer, items, shared, stars, views};

use crate::common::{MINUTE, NOW, TestDb, viewer};

#[tokio::test]
async fn visible_items_hides_trashed_items_and_deleted_collections() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let gone = t.collection(&owner, "Gone").await;
    let live = t.item(&owner, c.id, None, ItemKind::Doc, "live").await;
    let trashed = t.item(&owner, c.id, None, ItemKind::Doc, "trashed").await;
    let archived = t
        .item(&owner, c.id, None, ItemKind::Board, "archived")
        .await;
    t.item(
        &owner,
        gone.id,
        None,
        ItemKind::Doc,
        "in a deleted collection",
    )
    .await;
    let ids = (
        trashed.id.to_string(),
        archived.id.to_string(),
        gone.id.to_string(),
    );
    t.db.write(move |tx| {
        let (trashed, archived, gone) = ids;
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
            params![trashed, NOW],
        )?;
        tx.execute(
            "UPDATE items SET archived_at = ?2, archive_root_id = id WHERE id = ?1",
            params![archived, NOW],
        )?;
        // A collection that is deleted while its items are not (the store always trashes them too; the fragment
        // must not rely on it).
        tx.execute(
            "UPDATE collections SET deleted_at = ?2 WHERE id = ?1",
            params![gone, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let viewer = Viewer {
        user_id: owner.clone(),
        role: Role::Owner,
    };
    let sql = format!(
        "SELECT i.title FROM items i JOIN collections c ON c.id = i.collection_id WHERE {} ORDER BY i.title",
        shared::visible_items(&viewer)
    );
    let titles: Vec<String> = t
        .query(move |c| {
            c.prepare(&sql)?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect()
        })
        .await;
    // Archived items stay visible; the tree and default search filter them themselves (§5).
    assert_eq!(titles, ["archived", "live"]);
    let _ = live;
}

/// Every page of `/updated`, `limit` at a time; between the first and second page, `between` runs.
async fn all_pages<F, Fut>(t: &TestDb, owner: &str, limit: u32, between: F) -> Vec<Uuid>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut between = Some(between);
    let mut cursor = None;
    let mut out = Vec::new();
    loop {
        let (v, c) = (viewer(owner), cursor.clone());
        let page = t
            .r(move |conn| {
                lists::updated(conn, &v, None, &PageRequest::new("updated", c, Some(limit)))
            })
            .await
            .unwrap();
        assert!(page.items.len() <= limit as usize);
        out.extend(page.items.iter().map(|i| i.item.id));
        let Some(next) = page.next_cursor else { break };
        cursor = Some(next);
        if let Some(f) = between.take() {
            f().await;
        }
    }
    out
}

#[tokio::test]
async fn cursor_stable_under_concurrent_insert() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let mut ids = Vec::new();
    for i in 0..23 {
        let item = t
            .item(&owner, c.id, None, ItemKind::Doc, &format!("item {i}"))
            .await;
        ids.push(item.id);
    }
    // Pairs of equal times, so the id breaks ties across a page boundary.
    for (i, id) in ids.iter().enumerate() {
        let (id, at) = (id.to_string(), NOW + (i as i64 / 2) * MINUTE);
        t.w(move |tx| {
            tx.execute(
                "UPDATE items SET updated_at = ?2 WHERE id = ?1",
                params![id, at],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let undisturbed = all_pages(&t, &owner, 5, || async {}).await;
    let mut expected = ids.clone();
    expected.reverse();
    // Newest first; within equal times, the larger id first.
    let mut sorted = undisturbed.clone();
    sorted
        .sort_by_key(|id| std::cmp::Reverse((ids.iter().position(|x| x == id).unwrap() / 2, *id)));
    assert_eq!(undisturbed, sorted);
    // Between the first two pages: insert an item that sorts before the cursor (the newest) and one that sorts after
    // it (the oldest), and touch an already listed item to the top. Nothing repeats, nothing that existed throughout
    // is skipped, and only the insert after the cursor shows up.
    let newest = t.item(&owner, c.id, None, ItemKind::Doc, "newest").await;
    let oldest = t.item(&owner, c.id, None, ItemKind::Doc, "oldest").await;
    let set_time = |id: Uuid, at: i64| {
        let id = id.to_string();
        t.w(move |tx| {
            tx.execute(
                "UPDATE items SET updated_at = ?2 WHERE id = ?1",
                params![id, at],
            )?;
            Ok(())
        })
    };
    // Both wait in the trash (out of every listing) and come back between the first two pages.
    for id in [newest.id, oldest.id] {
        let id = id.to_string();
        t.w(move |tx| {
            tx.execute(
                "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
                params![id, NOW],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let disturbed = all_pages(&t, &owner, 5, || async {
        for (id, at) in [(newest.id, NOW + 50 * MINUTE), (oldest.id, NOW - MINUTE)] {
            set_time(id, at).await.unwrap();
            let id = id.to_string();
            t.w(move |tx| {
                tx.execute(
                    "UPDATE items SET deleted_at = NULL, trash_root_id = NULL WHERE id = ?1",
                    [id],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        }
        set_time(ids[22], NOW + 99 * MINUTE).await.unwrap();
    })
    .await;
    let mut seen = std::collections::HashSet::new();
    assert!(
        disturbed.iter().all(|id| seen.insert(*id)),
        "no repeats: {disturbed:?}"
    );
    for id in &ids {
        assert!(seen.contains(id), "{id} skipped");
    }
    assert!(seen.contains(&oldest.id) && !seen.contains(&newest.id));
    assert_eq!(disturbed.len(), ids.len() + 1);
}

#[tokio::test]
async fn cursor_is_bound_to_its_query() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    for i in 0..3 {
        t.item(&owner, c.id, None, ItemKind::Doc, &format!("{i}"))
            .await;
    }
    let v = viewer(&owner);
    let page = t
        .r(move |conn| lists::updated(conn, &v, None, &PageRequest::new("updated", None, Some(2))))
        .await
        .unwrap();
    let cursor = page.next_cursor.unwrap();
    let (v, cur, cid) = (viewer(&owner), cursor.clone(), c.id.to_string());
    let e = t
        .r(move |conn| {
            lists::updated(
                conn,
                &v,
                Some(&cid),
                &PageRequest::new("updated", Some(cur), None).param("collection_id", &cid),
            )
        })
        .await
        .unwrap_err();
    assert_eq!(
        (e.code, e.details),
        (
            ErrorCode::Validation,
            Some(serde_json::json!({"field": "cursor"}))
        )
    );
    let (v, cur) = (viewer(&owner), cursor.clone());
    let e = t
        .r(move |conn| lists::drafts(conn, &v, &PageRequest::new("drafts", Some(cur), None)))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
    for bad in [0, 101] {
        let v = viewer(&owner);
        let e = t
            .r(move |conn| {
                lists::updated(
                    conn,
                    &v,
                    None,
                    &PageRequest::new("updated", None, Some(bad)),
                )
            })
            .await
            .unwrap_err();
        assert_eq!(e.details, Some(serde_json::json!({"field": "limit"})));
    }
    // The page size may change between pages.
    let v = viewer(&owner);
    let rest = t
        .r(move |conn| {
            lists::updated(
                conn,
                &v,
                None,
                &PageRequest::new("updated", Some(cursor), Some(50)),
            )
        })
        .await
        .unwrap();
    assert_eq!((rest.items.len(), rest.next_cursor), (1, None));
}

#[tokio::test]
async fn recent_starred_drafts_and_trash_lists() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let d = t.item(&owner, c.id, None, ItemKind::Board, "d").await;
    let o = owner.clone();
    let (aid, bid, did) = (a.id.to_string(), b.id.to_string(), d.id.to_string());
    t.w(move |tx| {
        views::record(tx, &o, &aid, NOW)?;
        views::record(tx, &o, &bid, NOW + MINUTE)?;
        views::record(tx, &o, &aid, NOW + 2 * MINUTE)?;
        stars::star(tx, &o, &did, None, NOW)?;
        stars::star(tx, &o, &bid, None, NOW + MINUTE)?;
        // Move b before d among the stars; starring again without a position keeps the place.
        stars::star(
            tx,
            &o,
            &bid,
            Some(&Position {
                after_id: None,
                before_id: Some(d.id),
            }),
            NOW,
        )?;
        stars::star(tx, &o, &bid, None, NOW)?;
        Ok(())
    })
    .await
    .unwrap();
    let v = viewer(&owner);
    let recent = t
        .r(move |c| lists::recent(c, &v, &PageRequest::new("recent", None, None)))
        .await
        .unwrap();
    let r: Vec<(&str, Option<i64>)> = recent
        .items
        .iter()
        .map(|i| (i.item.title.as_str(), i.last_viewed_at.map(|t| t.millis())))
        .collect();
    assert_eq!(
        r,
        [("a", Some(NOW + 2 * MINUTE)), ("b", Some(NOW + MINUTE))]
    );
    assert_eq!(recent.items[1].path.len(), 1);
    let views: i64 = t
        .query(|c| c.query_row("SELECT max(view_count) FROM item_views", [], |r| r.get(0)))
        .await;
    assert_eq!(views, 2);
    let v = viewer(&owner);
    let starred = t
        .r(move |c| lists::starred(c, &v, &PageRequest::new("starred", None, None)))
        .await
        .unwrap();
    let s: Vec<&str> = starred
        .items
        .iter()
        .map(|i| i.item.title.as_str())
        .collect();
    assert_eq!(s, ["b", "d"]);
    assert_eq!(starred.items[1].starred_at.map(|t| t.millis()), Some(NOW));
    let v = viewer(&owner);
    let sidebar = t.r(move |c| lists::sidebar(c, &v)).await.unwrap();
    assert_eq!(
        sidebar
            .stars
            .iter()
            .map(|i| i.title.as_str())
            .collect::<Vec<_>>(),
        ["b", "d"]
    );
    assert_eq!((sidebar.collections.len(), sidebar.drafts_count), (1, 0));
    // Trash b: it leaves recent, starred and the sidebar's stars, and shows in the trash.
    let (bid, o) = (b.id.to_string(), owner.clone());
    t.w(move |tx| items::trash(tx, &bid, &o, NOW))
        .await
        .unwrap();
    let v = viewer(&owner);
    let recent = t
        .r(move |c| lists::recent(c, &v, &PageRequest::new("recent", None, None)))
        .await
        .unwrap();
    assert_eq!(recent.items.len(), 1);
    let v = viewer(&owner);
    let trash = t.r(move |c| lists::trash(c, &v)).await.unwrap();
    assert_eq!(trash.items.len(), 1);
    assert_eq!(
        (trash.items[0].item.id, trash.items[0].descendant_count),
        (b.id, 0)
    );
    assert_eq!(trash.items[0].purge_at.millis(), NOW + 30 * 86_400_000);
    assert_eq!(trash.items[0].path[0].id, a.id);
    // A star on a trashed item cannot be made; unstarring is idempotent.
    let (bid, o) = (b.id.to_string(), owner.clone());
    assert!(
        t.w(move |tx| stars::star(tx, &o, &bid, None, NOW))
            .await
            .unwrap_err()
            .is(ErrorCode::ReadOnly)
    );
    let (did, o) = (d.id.to_string(), owner.clone());
    t.w(move |tx| {
        stars::unstar(tx, &o, &did)?;
        stars::unstar(tx, &o, &did)
    })
    .await
    .unwrap();
    // Drafts.
    let (did, o) = (d.id.to_string(), owner.clone());
    t.w(move |tx| items::unpublish(tx, &did, &o, NOW))
        .await
        .unwrap();
    let v = viewer(&owner);
    let drafts = t
        .r(move |c| lists::drafts(c, &v, &PageRequest::new("drafts", None, None)))
        .await
        .unwrap();
    assert_eq!(
        drafts.items.iter().map(|i| i.item.id).collect::<Vec<_>>(),
        [d.id]
    );
}
