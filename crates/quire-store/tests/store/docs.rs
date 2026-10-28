//! Document storage tests (owner: W4)
//!
//! W0b: the initializers (empty `[0, 0]`, a given state, copy) and `is_empty` (§6.1: only empty paragraphs).

use quire_core::api::{ErrorCode, ItemKind, YFIELD};
use quire_store::docs;
use rusqlite::params;
use yrs::{Doc, ReadTxn, StateVector, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim};

use crate::common::{NOW, TestDb, count};

/// A Yjs state with one paragraph per entry (y-tiptap's shape: `paragraph` elements holding an XmlText).
fn ydoc(paragraphs: &[&str]) -> Vec<u8> {
    let doc = Doc::new();
    let frag = doc.get_or_insert_xml_fragment(YFIELD);
    {
        let mut txn = doc.transact_mut();
        for (i, text) in paragraphs.iter().enumerate() {
            let p = frag.insert(&mut txn, i as u32, XmlElementPrelim::empty("paragraph"));
            p.insert(&mut txn, 0, XmlTextPrelim::new(*text));
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
}

async fn is_empty(t: &TestDb, id: String) -> quire_store::Result<bool> {
    t.db.write(move |tx| docs::is_empty(tx, &id)).await
}

async fn state(t: &TestDb, id: String) -> (Vec<u8>, i64, Option<String>) {
    t.query(move |c| {
        c.query_row(
            "SELECT ydoc, next_seq, last_token_id FROM doc_state WHERE item_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    })
    .await
}

#[tokio::test]
async fn new_doc_is_the_empty_y_state_and_empty() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "").await;
    assert_eq!(state(&t, doc.id.to_string()).await, (vec![0, 0], 1, None));
    assert_eq!(docs::empty_state(), docs::EMPTY_YDOC);
    assert!(is_empty(&t, doc.id.to_string()).await.unwrap());
    // y-tiptap writes one empty paragraph when the editor starts: still empty.
    let id = doc.id.to_string();
    t.db.write(move |tx| docs::init_with(tx, &id, &ydoc(&[""]), NOW))
        .await
        .unwrap();
    assert!(is_empty(&t, doc.id.to_string()).await.unwrap());
    let id = doc.id.to_string();
    t.db.write(move |tx| docs::init_with(tx, &id, &ydoc(&["", ""]), NOW))
        .await
        .unwrap();
    assert!(is_empty(&t, doc.id.to_string()).await.unwrap());
    // A board is not a document.
    let board = t.item(&owner, c.id, None, ItemKind::Board, "").await;
    let e = is_empty(&t, board.id.to_string()).await.unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}

#[tokio::test]
async fn init_with_stores_the_state_and_its_text() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Seeded").await;
    let bytes = ydoc(&["Hello", "world"]);
    let (id, b) = (doc.id.to_string(), bytes.clone());
    t.db.write(move |tx| {
        tx.execute(
            "UPDATE doc_state SET next_seq = 5, last_token_id = 'tok' WHERE item_id = ?1",
            [&id],
        )?;
        tx.execute(
            "INSERT INTO doc_updates (item_id, seq, data, created_at) VALUES (?1, 4, ?2, ?3)",
            params![id, vec![0u8, 0], NOW],
        )?;
        docs::init_with(tx, &id, &b, NOW)
    })
    .await
    .unwrap();
    assert_eq!(
        state(&t, doc.id.to_string()).await,
        (bytes, 1, None),
        "replaced, seq 0 again"
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM doc_updates WHERE item_id = ?1",
            doc.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Hello\nworld'",
            doc.id.to_string()
        )
        .await,
        1
    );
    assert!(!is_empty(&t, doc.id.to_string()).await.unwrap());
    for bad in [vec![0xffu8, 0xff, 0xff], vec![1, 2, 3, 4, 5]] {
        let id = doc.id.to_string();
        let e =
            t.db.write(move |tx| docs::init_with(tx, &id, &bad, NOW))
                .await
                .unwrap_err();
        assert!(e.is(ErrorCode::InvalidUpdate), "{e:?}");
    }
}

#[tokio::test]
async fn copy_takes_state_updates_text_and_attachments() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let from = t.item(&owner, c.id, None, ItemKind::Doc, "From").await;
    let to = t.item(&owner, c.id, None, ItemKind::Doc, "To").await;
    let (f, tid, o) = (from.id.to_string(), to.id.to_string(), owner.clone());
    let blobs = t.blobs.clone();
    t.db.write(move |tx| {
        docs::init_with(tx, &f, &ydoc(&["Copied text"]), NOW)?;
        let blob = blobs.put(tx, b"image bytes", NOW)?;
        tx.execute(
            "INSERT INTO attachments (id, item_id, sha256, filename, mime, created_by, created_at)
             VALUES ('att-1', ?1, ?2, 'a.png', 'image/png', ?3, ?4)",
            params![f, blob.sha256.as_str(), o, NOW],
        )?;
        tx.execute(
            "INSERT INTO item_attachments (item_id, attachment_id) VALUES (?1, 'att-1')",
            [&f],
        )?;
        tx.execute(
            "INSERT INTO item_links (source_id, target_id, kind) VALUES (?1, 'target', 'mention')",
            [&f],
        )?;
        docs::copy(tx, &f, &tid, NOW)
    })
    .await
    .unwrap();
    assert!(!is_empty(&t, to.id.to_string()).await.unwrap());
    let to_id = to.id.to_string();
    for (sql, expect) in [
        (
            "SELECT count(*) FROM item_attachments WHERE item_id = ?1 AND attachment_id = 'att-1'",
            1,
        ),
        (
            "SELECT count(*) FROM item_links WHERE source_id = ?1 AND target_id = 'target'",
            1,
        ),
        (
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Copied text'",
            1,
        ),
        ("SELECT next_seq FROM doc_state WHERE item_id = ?1", 1),
    ] {
        assert_eq!(count(&t, sql, to_id.clone()).await, expect, "{sql}");
    }
    // Copying a board into a document is refused.
    let board = t.item(&owner, c.id, None, ItemKind::Board, "B").await;
    let (b, tid) = (board.id.to_string(), to.id.to_string());
    let e =
        t.db.write(move |tx| docs::copy(tx, &b, &tid, NOW))
            .await
            .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
}
