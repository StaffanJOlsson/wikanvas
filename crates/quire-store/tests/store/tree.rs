//! Tree tests: CTEs, triggers, cycles, cross-collection moves, deep purges, tree order, and the store against the
//! reference model of `quire_core::tree` (owner: W1).

use quire_core::api::{ChildOrder, CollectionPatch, ErrorCode, ItemKind, MoveItem, Position};
use quire_core::tree::{Model, TreeError};
use quire_store::{collections, items, tree};
use rusqlite::params;
use uuid::Uuid;

use crate::common::{DAY, MINUTE, NOW, TestDb, count, viewer};

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
    // The breadcrumbs of the deepest item walk all 2,000 ancestors, root first.
    let deepest: String = t
        .query(|c| {
            c.query_row(
                "SELECT id FROM items WHERE url_id = 'deep001999'",
                [],
                |r| r.get(0),
            )
        })
        .await;
    let crumbs = t.r(move |c| tree::breadcrumbs(c, &deepest)).await.unwrap();
    assert_eq!(crumbs.len(), 2000);
    assert_eq!(crumbs[0].title, "root");
    assert_eq!(crumbs[1].url_id, "deep000000");
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

async fn mv(
    t: &TestDb,
    owner: &str,
    id: Uuid,
    collection: Option<Uuid>,
    parent: Option<Uuid>,
) -> quire_store::Result<()> {
    let (id, owner) = (id.to_string(), owner.to_owned());
    t.w(move |tx| {
        items::move_item(
            tx,
            &id,
            &MoveItem {
                collection_id: collection,
                parent_id: parent,
                position: None,
            },
            &owner,
            NOW,
        )
        .map(|_| ())
    })
    .await
}

#[tokio::test]
async fn move_into_descendant_is_cycle() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "b").await;
    let cc = t.item(&owner, c.id, Some(b.id), ItemKind::Board, "c").await;
    for (moving, target) in [(a.id, cc.id), (a.id, b.id), (a.id, a.id), (b.id, cc.id)] {
        let e = mv(&t, &owner, moving, None, Some(target))
            .await
            .unwrap_err();
        assert!(e.is(ErrorCode::Cycle), "{moving} under {target}: {e:?}");
    }
    // Nothing moved.
    assert_eq!(t.row(a.id).await.parent_id, None);
    // The other way round is fine: c to the top, then a under c.
    mv(&t, &owner, cc.id, None, None).await.unwrap();
    mv(&t, &owner, a.id, None, Some(cc.id)).await.unwrap();
    assert_eq!(t.row(a.id).await.parent_id, Some(cc.id.to_string()));
    assert_eq!(t.tree_titles(&owner, c.id).await, ["c", "a", "b"]);
    // Moving under a trashed or archived item, or a missing one, is parent_invalid.
    let x = t.item(&owner, c.id, None, ItemKind::Doc, "x").await;
    let (xid, o) = (x.id.to_string(), owner.clone());
    t.w(move |tx| items::trash(tx, &xid, &o, NOW))
        .await
        .unwrap();
    let e = mv(&t, &owner, b.id, None, Some(x.id)).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
    let e = mv(&t, &owner, b.id, None, Some(Uuid::now_v7()))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
    // A trashed item cannot be moved.
    let e = mv(&t, &owner, x.id, None, None).await.unwrap_err();
    assert!(e.is(ErrorCode::ReadOnly), "{e:?}");
}

#[tokio::test]
async fn cross_collection_move_updates_descendants() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let x = t.collection(&owner, "X").await;
    let y = t.collection(&owner, "Y").await;
    let a = t.item(&owner, x.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, x.id, Some(a.id), ItemKind::Doc, "b").await;
    let c = t.item(&owner, x.id, Some(b.id), ItemKind::Doc, "c").await;
    let d = t.item(&owner, x.id, Some(a.id), ItemKind::Board, "d").await;
    let e = t.item(&owner, x.id, Some(d.id), ItemKind::Doc, "e").await;
    let y0 = t.item(&owner, y.id, None, ItemKind::Doc, "y0").await;
    // c is trashed (its own root), d archived with e below it.
    let (cid, did, o) = (c.id.to_string(), d.id.to_string(), owner.clone());
    t.w(move |tx| {
        items::trash(tx, &cid, &o, NOW)?;
        items::archive(tx, &did, NOW)
    })
    .await
    .unwrap();
    let keys_before: Vec<String> = futures_keys(&t, &[b.id, c.id, d.id, e.id]).await;
    let (tvx, tvy) = (t.tree_version(x.id).await, t.tree_version(y.id).await);

    // a moves to the top of Y, after y0.
    let (aid, o) = (a.id.to_string(), owner.clone());
    let moved = t
        .w(move |tx| {
            items::move_item(
                tx,
                &aid,
                &MoveItem {
                    collection_id: Some(y.id),
                    parent_id: None,
                    position: Some(Position {
                        after_id: Some(y0.id),
                        before_id: None,
                    }),
                },
                &o,
                NOW + MINUTE,
            )
        })
        .await
        .unwrap();
    assert_eq!((moved.collection_id, moved.parent_id), (y.id, None));
    assert_eq!(moved.updated_at.millis(), NOW + MINUTE);
    for id in [a.id, b.id, c.id, d.id, e.id] {
        assert_eq!(t.row(id).await.collection_id, y.id.to_string(), "{id}");
    }
    // Descendants keep their keys, parents and states.
    assert_eq!(
        futures_keys(&t, &[b.id, c.id, d.id, e.id]).await,
        keys_before
    );
    assert_eq!(t.row(c.id).await.trash_root_id, Some(c.id.to_string()));
    assert_eq!(t.row(e.id).await.archive_root_id, Some(d.id.to_string()));
    assert_eq!(t.row(e.id).await.parent_id, Some(d.id.to_string()));
    assert!(t.tree_version(x.id).await > tvx && t.tree_version(y.id).await > tvy);
    assert!(t.tree_titles(&owner, x.id).await.is_empty());
    assert_eq!(t.tree_titles(&owner, y.id).await, ["y0", "a", "b"]);
    // The trashed descendant restores in its new collection, under its parent.
    let cid = c.id.to_string();
    let restored = t.w(move |tx| items::restore(tx, &cid)).await.unwrap();
    assert!(!restored.moved_to_top);
    assert_eq!(restored.item.collection_id, y.id);
    assert_eq!(t.tree_titles(&owner, y.id).await, ["y0", "a", "b", "c"]);
    // Into a deleted collection: parent_invalid.
    let (yid, o) = (y.id.to_string(), owner.clone());
    t.w(move |tx| collections::delete(tx, &yid, &o, NOW))
        .await
        .unwrap();
    let e = mv(&t, &owner, y0.id, Some(x.id), None).await.unwrap_err();
    assert!(
        e.is(ErrorCode::ReadOnly),
        "a trashed item cannot move: {e:?}"
    );
    let z = t.item(&owner, x.id, None, ItemKind::Doc, "z").await;
    let e = mv(&t, &owner, z.id, Some(y.id), None).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
}

async fn futures_keys(t: &TestDb, ids: &[Uuid]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        out.push(t.row(*id).await.sort_key);
    }
    out
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

#[tokio::test]
async fn collection_trigger_rejects_foreign_parent() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let x = t.collection(&owner, "X").await;
    let y = t.collection(&owner, "Y").await;
    let a = t.item(&owner, x.id, None, ItemKind::Doc, "a").await;
    let b = t.item(&owner, x.id, Some(a.id), ItemKind::Doc, "b").await;
    let set_collection = |id: Uuid, collection: Uuid| {
        let (id, collection) = (id.to_string(), collection.to_string());
        t.w(move |tx| {
            tx.execute(
                "UPDATE items SET collection_id = ?2 WHERE id = ?1",
                params![id, collection],
            )?;
            Ok(())
        })
    };
    // The child alone: its parent stays in X, so the row would point across collections.
    let e = set_collection(b.id, y.id).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
    // Top-down works: the root first, then the child.
    set_collection(a.id, y.id).await.unwrap();
    set_collection(b.id, y.id).await.unwrap();
    // A missing or deleted target collection is refused too.
    let e = set_collection(a.id, Uuid::now_v7()).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
    let xid = x.id.to_string();
    t.w(move |tx| {
        tx.execute("UPDATE collections SET deleted_at = 1 WHERE id = ?1", [xid])?;
        Ok(())
    })
    .await
    .unwrap();
    let e = set_collection(a.id, x.id).await.unwrap_err();
    assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
}

#[tokio::test]
async fn tree_is_flat_parents_first_in_child_order() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let b = t.item(&owner, c.id, None, ItemKind::Doc, "b").await;
    let a = t.item(&owner, c.id, None, ItemKind::Doc, "A").await;
    t.item(&owner, c.id, Some(b.id), ItemKind::Doc, "b2").await;
    t.item(&owner, c.id, Some(b.id), ItemKind::Board, "b1")
        .await;
    t.item(&owner, c.id, Some(a.id), ItemKind::Doc, "a1").await;
    assert_eq!(
        t.tree_titles(&owner, c.id).await,
        ["b", "b2", "b1", "A", "a1"]
    );
    let tree = {
        let (v, cid) = (viewer(&owner), c.id.to_string());
        t.r(move |conn| tree::collection_tree(conn, &v, &cid))
            .await
            .unwrap()
    };
    let b_summary = tree.items.iter().find(|i| i.title == "b").unwrap();
    assert_eq!(b_summary.child_count, 2);
    assert_eq!(tree.collection.item_count, 5);
    let patch = |order: ChildOrder| {
        let (cid, o) = (c.id.to_string(), owner.clone());
        t.w(move |tx| {
            collections::update(
                tx,
                &cid,
                &CollectionPatch {
                    child_order: Some(order),
                    ..Default::default()
                },
                &o,
                NOW,
            )
        })
    };
    let tv = t.tree_version(c.id).await;
    patch(ChildOrder::Title).await.unwrap();
    assert!(
        t.tree_version(c.id).await > tv,
        "a child_order change changes the tree"
    );
    assert_eq!(
        t.tree_titles(&owner, c.id).await,
        ["A", "a1", "b", "b1", "b2"]
    );
    // Newest first by updated_at: touch b1, which also bumps the tree version under this order.
    patch(ChildOrder::Updated).await.unwrap();
    let tv = t.tree_version(c.id).await;
    let b1: String = t
        .query(|c| c.query_row("SELECT id FROM items WHERE title = 'b1'", [], |r| r.get(0)))
        .await;
    let o = owner.clone();
    t.w(move |tx| quire_store::shared::touch_content(tx, &b1, &o, NOW + MINUTE))
        .await
        .unwrap();
    assert_eq!(t.tree_version(c.id).await, tv + 1);
    // Equal times fall back to the id (creation order): b before A; under b, the touched b1 first.
    assert_eq!(
        t.tree_titles(&owner, c.id).await,
        ["b", "b1", "b2", "A", "a1"]
    );
    // sort_key is untouched by a non-manual order.
    patch(ChildOrder::Manual).await.unwrap();
    assert_eq!(
        t.tree_titles(&owner, c.id).await,
        ["b", "b2", "b1", "A", "a1"]
    );
    // A deleted collection has no tree.
    let (cid, o) = (c.id.to_string(), owner.clone());
    t.w(move |tx| collections::delete(tx, &cid, &o, NOW))
        .await
        .unwrap();
    let (v, cid) = (viewer(&owner), c.id.to_string());
    let e = t
        .r(move |conn| tree::collection_tree(conn, &v, &cid))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}

// ---------------------------------------------------------------- the store against the reference model

#[derive(Clone, Debug)]
enum Op {
    Create(usize, Option<usize>),
    Move(usize, usize, Option<usize>),
    Trash(usize),
    Restore(usize),
    Archive(usize),
    Unarchive(usize),
    DeleteCollection(usize),
    RestoreCollection(usize),
    Purge(usize),
}

fn op() -> impl proptest::strategy::Strategy<Value = Op> {
    use proptest::prelude::*;
    let i = 0..40usize;
    prop_oneof![
        4 => (0..2usize, prop::option::weighted(0.7, i.clone())).prop_map(|(c, p)| Op::Create(c, p)),
        3 => (i.clone(), 0..2usize, prop::option::weighted(0.7, i.clone())).prop_map(|(x, c, p)| Op::Move(x, c, p)),
        2 => i.clone().prop_map(Op::Trash),
        2 => i.clone().prop_map(Op::Restore),
        2 => i.clone().prop_map(Op::Archive),
        2 => i.clone().prop_map(Op::Unarchive),
        1 => (0..2usize).prop_map(Op::DeleteCollection),
        1 => (0..2usize).prop_map(Op::RestoreCollection),
        1 => i.prop_map(Op::Purge),
    ]
}

/// The store's answer for one op: the error code, or a count / (root index, moved) encoded like the model's.
async fn apply_store(
    t: &TestDb,
    owner: &str,
    ids: &mut Vec<Uuid>,
    cols: &[Uuid],
    op: &Op,
) -> Result<u64, ErrorCode> {
    let n = ids.len().max(1);
    let id = |x: usize| {
        ids.get(x % n)
            .copied()
            .unwrap_or_else(Uuid::nil)
            .to_string()
    };
    let index =
        |ids: &[Uuid], u: Uuid| ids.iter().position(|x| *x == u).expect("a known id") as u64;
    let o = owner.to_owned();
    let res: quire_store::Result<u64> = match op.clone() {
        Op::Create(c, p) => {
            let parent = p.map(|p| ids.get(p % n).copied().unwrap_or_else(Uuid::nil));
            let new = quire_core::api::NewItem {
                collection_id: cols[c],
                parent_id: parent,
                kind: ItemKind::Doc,
                title: None,
                position: None,
            };
            match t.w(move |tx| items::create(tx, &o, &new, false, NOW)).await {
                Ok(s) => {
                    ids.push(s.id);
                    Ok(ids.len() as u64 - 1)
                }
                Err(e) => Err(e),
            }
        }
        Op::Move(x, c, p) => {
            let (x, parent) = (
                id(x),
                p.map(|p| ids.get(p % n).copied().unwrap_or_else(Uuid::nil)),
            );
            let col = cols[c];
            t.w(move |tx| {
                items::move_item(
                    tx,
                    &x,
                    &MoveItem {
                        collection_id: Some(col),
                        parent_id: parent,
                        position: None,
                    },
                    &o,
                    NOW,
                )
                .map(|_| 0)
            })
            .await
        }
        Op::Trash(x) => {
            let x = id(x);
            t.w(move |tx| items::trash(tx, &x, &o, NOW)).await
        }
        Op::Archive(x) => {
            let x = id(x);
            t.w(move |tx| items::archive(tx, &x, NOW)).await
        }
        Op::Restore(x) | Op::Unarchive(x) => {
            let x = id(x);
            let restore = matches!(op, Op::Restore(_));
            let ids2 = ids.clone();
            t.w(move |tx| {
                let r = if restore {
                    items::restore(tx, &x)?
                } else {
                    items::unarchive(tx, &x)?
                };
                Ok(index(&ids2, r.item.id) * 2 + u64::from(r.moved_to_top))
            })
            .await
        }
        Op::DeleteCollection(c) => {
            let c = cols[c].to_string();
            t.w(move |tx| collections::delete(tx, &c, &o, NOW)).await
        }
        Op::RestoreCollection(c) => {
            let c = cols[c].to_string();
            t.w(move |tx| collections::restore(tx, &c, &o, NOW).map(|_| 0))
                .await
        }
        Op::Purge(x) => {
            let x = id(x);
            t.w(move |tx| items::purge(tx, &x)).await
        }
    };
    res.map_err(|e| e.code)
}

fn apply_model(m: &mut Model, op: &Op) -> Result<u64, ErrorCode> {
    let n = m.nodes.len().max(1);
    let code = |e: TreeError| quire_store::Error::from(e).code;
    let r = match *op {
        Op::Create(c, p) => m.create(c, p.map(|p| p % n)).map(|i| i as u64),
        Op::Move(x, c, p) => m.move_to(x % n, c, p.map(|p| p % n)).map(|()| 0),
        Op::Trash(x) => m.trash(x % n),
        Op::Restore(x) => m
            .restore(x % n)
            .map(|(r, moved)| r as u64 * 2 + u64::from(moved)),
        Op::Archive(x) => m.archive(x % n),
        Op::Unarchive(x) => m
            .unarchive(x % n)
            .map(|(r, moved)| r as u64 * 2 + u64::from(moved)),
        Op::DeleteCollection(c) => m.delete_collection(c),
        Op::RestoreCollection(c) => m.restore_collection(c).map(|()| 0),
        Op::Purge(x) => m.purge(x % n),
    };
    r.map_err(code)
}

/// The visible tree of the store as `(index, collection index, parent index)`, like `Model::visible`.
async fn visible_store(
    t: &TestDb,
    owner: &str,
    ids: &[Uuid],
    cols: &[Uuid],
) -> std::collections::BTreeSet<(usize, usize, Option<usize>)> {
    let mut out = std::collections::BTreeSet::new();
    for (ci, c) in cols.iter().enumerate() {
        let (v, cid) = (viewer(owner), c.to_string());
        let Ok(tree) = t.r(move |conn| tree::collection_tree(conn, &v, &cid)).await else {
            continue; // a deleted collection
        };
        for item in tree.items {
            let i = ids.iter().position(|x| *x == item.id).unwrap();
            let p = item
                .parent_id
                .map(|p| ids.iter().position(|x| *x == p).unwrap());
            out.insert((i, ci, p));
        }
    }
    out
}

#[test]
fn store_matches_the_core_model() {
    use proptest::prelude::*;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let config = ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    proptest!(config, |(ops in prop::collection::vec(op(), 1..50))| {
        rt.block_on(async {
            let t = TestDb::new();
            let owner = t.owner().await;
            let cols = [t.collection(&owner, "c0").await.id, t.collection(&owner, "c1").await.id];
            let mut model = Model::new(2);
            let mut ids = Vec::new();
            for op in &ops {
                let want = apply_model(&mut model, op);
                let got = apply_store(&t, &owner, &mut ids, &cols, op).await;
                prop_assert_eq!(got, want, "{:?}", op);
                prop_assert_eq!(visible_store(&t, &owner, &ids, &cols).await, model.visible(), "after {:?}", op);
            }
            let fk: i64 = t.query(|c| c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))).await;
            prop_assert_eq!(fk, 0);
            Ok(())
        })?;
    });
}
