//! Search tests: FTS sync and the query builder (owner: W1)
//!
//! W0b: the FTS index follows titles (create, rename) and body text (`shared::set_body_text`).

use quire_core::api::ItemKind;
use quire_store::{items, shared};

use crate::common::{NOW, TestDb};

async fn hits(t: &TestDb, q: &'static str) -> Vec<String> {
    t.query(move |c| {
        c.prepare(
            "SELECT t.item_id FROM item_fts JOIN item_text t ON t.text_id = item_fts.rowid
             WHERE item_fts MATCH ?1 ORDER BY t.item_id",
        )?
        .query_map([q], |r| r.get::<_, String>(0))?
        .collect()
    })
    .await
}

#[tokio::test]
async fn fts_follows_title_and_body() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t
        .item(&owner, c.id, None, ItemKind::Doc, "Quokka plans")
        .await;
    let id = doc.id.to_string();
    assert_eq!(hits(&t, "quokka").await, std::slice::from_ref(&id));
    let (i, o) = (id.clone(), owner.clone());
    t.db.write(move |tx| {
        items::rename(tx, &i, "Wombat plans", &o, NOW)?;
        shared::set_body_text(tx, &i, "a café by the river")
    })
    .await
    .unwrap();
    assert!(hits(&t, "quokka").await.is_empty());
    assert_eq!(hits(&t, "wombat").await, std::slice::from_ref(&id));
    // unicode61 remove_diacritics 2: "cafe" finds "café".
    assert_eq!(hits(&t, "cafe").await, std::slice::from_ref(&id));
    // The external-content index is consistent with item_text (an inconsistency is an SQLITE_CORRUPT error).
    t.db.write(|tx| {
        tx.execute(
            "INSERT INTO item_fts(item_fts) VALUES ('integrity-check')",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}
