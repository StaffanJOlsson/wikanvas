//! List tests: cursor pages (owner: W1)
//!
//! W0b: `shared::visible_items`, the condition every list query selects items with.

use quire_core::api::{ItemKind, Role};
use quire_store::{Viewer, shared};
use rusqlite::params;

use crate::common::{NOW, TestDb};

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
