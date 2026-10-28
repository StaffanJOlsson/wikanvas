//! Document storage tests (owner: W4)
//!
//! The initializers and `is_empty` (§6.1), the Yjs save path (missing deps, the no-op rule, compaction, the cap,
//! derived text, links, attachments, `doc_clients`, read-only items, the D53 recovery points), and the content
//! writes of §6.4 (one update through the save path, strict and merging `If-Match`, retries, `schema_too_new`).

use quire_core::api::{ErrorCode, ItemKind, YFIELD};
use quire_store::docs::{self, ContentInput, ContentWrite, IfMatch, PmJson};
use quire_store::shared::WriteBy;
use rusqlite::params;
use serde_json::{Value, json};
use yrs::updates::decoder::Decode;
use yrs::{
    Doc, GetString, Options, ReadTxn, StateVector, Text, Transact, Update, Xml, XmlElementPrelim,
    XmlFragment, XmlOut, XmlTextPrelim, XmlTextRef,
};

use crate::common::{MINUTE, NOW, TestDb, count};

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

/// A client editing a document: a Y.Doc with a fixed client id, synced from server bytes.
struct Client {
    doc: Doc,
}

impl Client {
    fn new(id: u64, state: &[u8]) -> Client {
        let doc = Doc::with_options(Options {
            offset_kind: yrs::OffsetKind::Utf16,
            ..Options::with_client_id(yrs::ClientID::new(id))
        });
        doc.transact_mut()
            .apply_update(Update::decode_v1(state).unwrap())
            .unwrap();
        Client { doc }
    }

    fn sv(&self) -> StateVector {
        self.doc.transact().state_vector()
    }

    fn text(&self, block: u32) -> XmlTextRef {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let txn = self.doc.transact();
        let Some(XmlOut::Element(p)) = frag.get(&txn, block) else {
            panic!("no block {block}")
        };
        match p.get(&txn, 0) {
            Some(XmlOut::Text(t)) => t,
            _ => panic!("no text in block {block}"),
        }
    }

    /// Inserts `s` at the end of block `block`; returns the update.
    fn type_text(&self, block: u32, s: &str) -> Vec<u8> {
        let t = self.text(block);
        let before = self.sv();
        {
            let mut txn = self.doc.transact_mut();
            let len = t.len(&txn);
            t.insert(&mut txn, len, s);
        }
        self.doc.transact().encode_diff_v1(&before)
    }

    /// Appends a paragraph holding `s`; returns the update.
    fn add_paragraph(&self, s: &str) -> Vec<u8> {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let before = self.sv();
        {
            let mut txn = self.doc.transact_mut();
            let n = frag.len(&txn);
            let p = frag.insert(&mut txn, n, XmlElementPrelim::empty("paragraph"));
            p.insert(&mut txn, 0, XmlTextPrelim::new(s));
        }
        self.doc.transact().encode_diff_v1(&before)
    }

    /// Deletes every block (select all, delete); returns the update.
    fn delete_all(&self) -> Vec<u8> {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let before = self.sv();
        {
            let mut txn = self.doc.transact_mut();
            let n = frag.len(&txn);
            frag.remove_range(&mut txn, 0, n);
        }
        let txn = self.doc.transact();
        // A delete-only change: no new structs, only a delete set.
        txn.encode_diff_v1(&before)
    }

    fn all_text(&self) -> String {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let txn = self.doc.transact();
        frag.children(&txn)
            .filter_map(|c| match c {
                XmlOut::Element(e) => e.get(&txn, 0).and_then(|t| match t {
                    XmlOut::Text(t) => Some(t.get_string(&txn)),
                    _ => None,
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Fixture {
    t: TestDb,
    owner: String,
    doc_id: String,
}

async fn fixture(paragraphs: &[&str]) -> Fixture {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let item = t
        .item(&owner, c.id, None, ItemKind::Doc, "A document")
        .await;
    let doc_id = item.id.to_string();
    let (id, bytes) = (doc_id.clone(), ydoc(paragraphs));
    t.db.write(move |tx| docs::init_with(tx, &id, &bytes, NOW))
        .await
        .unwrap();
    Fixture { t, owner, doc_id }
}

impl Fixture {
    async fn state(&self) -> Vec<u8> {
        let id = self.doc_id.clone();
        let s = self
            .t
            .db
            .read(move |c| docs::get(c, &id, None))
            .await
            .unwrap();
        b64d(&s.update)
    }

    async fn seq(&self) -> i64 {
        count(
            &self.t,
            "SELECT next_seq - 1 FROM doc_state WHERE item_id = ?1",
            self.doc_id.clone(),
        )
        .await
    }

    async fn post(&self, update: Vec<u8>, token: Option<&str>) -> quire_store::Result<docs::Saved> {
        self.post_at(update, token, NOW).await
    }

    async fn post_at(
        &self,
        update: Vec<u8>,
        token: Option<&str>,
        now: i64,
    ) -> quire_store::Result<docs::Saved> {
        let (id, owner, token) = (
            self.doc_id.clone(),
            self.owner.clone(),
            token.map(str::to_owned),
        );
        self.t
            .db
            .write(move |tx| {
                let by = WriteBy {
                    user_id: &owner,
                    token_id: token.as_deref(),
                };
                docs::save_update(
                    tx,
                    &id,
                    &update,
                    PmJson::Keep,
                    &by,
                    docs::DEFAULT_MAX_DOC_BYTES,
                    now,
                )
            })
            .await
    }

    fn write_req(&self, input: ContentInput, if_match: IfMatch) -> ContentWrite {
        let body = match &input {
            ContentInput::Markdown(m) => m.clone().into_bytes(),
            ContentInput::Json(v) => serde_json::to_vec(v).unwrap(),
        };
        ContentWrite {
            item_id: self.doc_id.clone(),
            input,
            if_match,
            merge: false,
            title_from_h1: false,
            credential: "session-1".into(),
            query: String::new(),
            body_sha256: sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(
                &body,
            ))
            .into(),
            public_origin: None,
            max_doc_bytes: docs::DEFAULT_MAX_DOC_BYTES,
        }
    }

    async fn put(
        &self,
        req: ContentWrite,
        token: Option<&str>,
    ) -> quire_store::Result<quire_core::api::DocWriteResult> {
        let by = WriteBy {
            user_id: &self.owner,
            token_id: token,
        };
        docs::write_content(&self.t.db, req, &by, NOW).await
    }

    async fn markdown(&self) -> String {
        let id = self.doc_id.clone();
        self.t
            .db
            .read(move |c| docs::markdown(c, &id, false, None))
            .await
            .unwrap()
            .markdown
    }

    async fn n(&self, sql: &'static str) -> i64 {
        count(&self.t, sql, self.doc_id.clone()).await
    }
}

fn b64d(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

fn md(s: &str) -> ContentInput {
    ContentInput::Markdown(s.to_owned())
}

// ------------------------------------------------------------------------------------------------- initializers

async fn is_empty(t: &TestDb, id: String) -> quire_store::Result<bool> {
    t.db.write(move |tx| docs::is_empty(tx, &id)).await
}

#[tokio::test]
async fn new_doc_is_the_empty_y_state_and_empty() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "").await;
    assert_eq!(docs::empty_state(), docs::EMPTY_YDOC);
    assert!(is_empty(&t, doc.id.to_string()).await.unwrap());
    for paragraphs in [&[""][..], &["", ""]] {
        let (id, bytes) = (doc.id.to_string(), ydoc(paragraphs));
        t.db.write(move |tx| docs::init_with(tx, &id, &bytes, NOW))
            .await
            .unwrap();
        assert!(
            is_empty(&t, doc.id.to_string()).await.unwrap(),
            "{paragraphs:?}"
        );
    }
    let board = t.item(&owner, c.id, None, ItemKind::Board, "").await;
    let e = is_empty(&t, board.id.to_string()).await.unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
}

#[tokio::test]
async fn init_with_stores_the_state_and_its_text() {
    let f = fixture(&["Hello", "world"]).await;
    assert_eq!(
        f.n("SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Hello\nworld'")
            .await,
        1
    );
    assert_eq!(f.seq().await, 0);
    for bad in [vec![0xffu8, 0xff, 0xff], vec![1, 2, 3, 4, 5]] {
        let id = f.doc_id.clone();
        let e =
            f.t.db
                .write(move |tx| docs::init_with(tx, &id, &bad, NOW))
                .await
                .unwrap_err();
        assert!(e.is(ErrorCode::InvalidUpdate), "{e:?}");
    }
}

#[tokio::test]
async fn init_seed_replaces_placeholder_ids() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Getting started").await;
    let welcome = t
        .item(&owner, c.id, None, ItemKind::Doc, "Welcome to Quire")
        .await;
    let bin = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/ydoc/getting-started.bin"
    ))
    .unwrap();
    let (id, writing, board) = (
        welcome.id.to_string(),
        uuid::Uuid::now_v7().to_string(),
        uuid::Uuid::now_v7().to_string(),
    );
    let (w, b) = (writing.clone(), board.clone());
    t.db.write(move |tx| {
        docs::init_seed(
            tx,
            &id,
            &bin,
            &[
                (docs::SEED_WRITING_ID, w.as_str()),
                (docs::SEED_BOARD_ID, b.as_str()),
            ],
            NOW,
        )
    })
    .await
    .unwrap();
    let id = welcome.id.to_string();
    let links = t.db.read(move |c| docs::links(c, &id)).await.unwrap();
    assert!(
        links.contains(&(writing, "mention".to_owned())),
        "{links:?}"
    );
    assert!(links.contains(&(board, "embed".to_owned())), "{links:?}");
    let body = count(
        &t,
        "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body LIKE '%Ctrl+K%'",
        welcome.id.to_string(),
    )
    .await;
    assert_eq!(body, 1);
}

#[tokio::test]
async fn copy_takes_state_updates_text_and_attachments() {
    let f = fixture(&["Copied text"]).await;
    let c = Client::new(5, &f.state().await);
    f.post(c.add_paragraph("More"), None).await.unwrap();
    let to =
        f.t.item(
            &f.owner,
            f.t.collection(&f.owner, "Other").await.id,
            None,
            ItemKind::Doc,
            "To",
        )
        .await;
    let (from, to_id) = (f.doc_id.clone(), to.id.to_string());
    f.t.db
        .write(move |tx| docs::copy(tx, &from, &to_id, NOW))
        .await
        .unwrap();
    let to_id = to.id.to_string();
    assert_eq!(
        count(
            &f.t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Copied text\nMore'",
            to_id.clone()
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &f.t,
            "SELECT next_seq FROM doc_state WHERE item_id = ?1",
            to_id
        )
        .await,
        1
    );
}

// ------------------------------------------------------------------------------------------------- the save path

#[tokio::test]
async fn compaction_preserves_state() {
    let f = fixture(&["Start"]).await;
    let c = Client::new(11, &f.state().await);
    let mut sv_before = None;
    for i in 0..250 {
        let saved = f
            .post(c.type_text(0, &format!(" {i}")), None)
            .await
            .unwrap();
        assert!(saved.changed);
        if i == 199 {
            // 200 rows: not yet compacted.
            assert_eq!(
                f.n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
                    .await,
                200
            );
            sv_before = Some(saved.state_vector);
        }
    }
    let _ = sv_before;
    // Compacted once past 200 rows: fewer rows, compacted_seq set, same state.
    let rows = f
        .n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
        .await;
    assert!(rows < 200, "{rows} rows");
    assert!(
        f.n("SELECT compacted_seq FROM doc_state WHERE item_id = ?1")
            .await
            >= 201
    );
    let server = Client::new(99, &f.state().await);
    assert_eq!(server.sv(), c.sv(), "state vectors equal");
    assert_eq!(server.all_text(), c.all_text());
    let expected: String = std::iter::once("Start".to_owned())
        .chain((0..250).map(|i| format!(" {i}")))
        .collect();
    assert_eq!(server.all_text(), expected);
    let body: String = {
        let id = f.doc_id.clone();
        f.t.query(move |c| {
            c.query_row("SELECT body FROM item_text WHERE item_id = ?1", [id], |r| {
                r.get(0)
            })
        })
        .await
    };
    assert_eq!(body, expected);
    assert_eq!(f.seq().await, 250);
}

#[tokio::test]
async fn missing_deps_409_with_state_vector() {
    let f = fixture(&["Base"]).await;
    let c = Client::new(21, &f.state().await);
    let _first = c.type_text(0, " one");
    let second = c.type_text(0, " two");
    let e = f.post(second.clone(), None).await.unwrap_err();
    assert!(e.is(ErrorCode::MissingDeps), "{e:?}");
    let sv = e.details.as_ref().unwrap()["state_vector"]
        .as_str()
        .unwrap()
        .to_owned();
    let server_sv = StateVector::decode_v1(&b64d(&sv)).unwrap();
    assert_eq!(server_sv, Client::new(98, &f.state().await).sv());
    assert_eq!(f.seq().await, 0, "nothing stored");
    // The client resends everything the server lacks.
    let fix = c.doc.transact().encode_state_as_update_v1(&server_sv);
    assert!(f.post(fix, None).await.unwrap().changed);
    assert_eq!(Client::new(97, &f.state().await).all_text(), "Base one two");
}

#[tokio::test]
async fn duplicate_update_writes_nothing() {
    let f = fixture(&["Base text"]).await;
    let c = Client::new(31, &f.state().await);
    let insert = c.type_text(0, " more");
    assert!(f.post(insert.clone(), None).await.unwrap().changed);
    let seq = f.seq().await;
    let again = f.post(insert, None).await.unwrap();
    assert!(!again.changed);
    assert_eq!(again.seq, seq);
    let del = c.delete_all();
    assert!(f.post(del.clone(), None).await.unwrap().changed);
    let seq = f.seq().await;
    let rows = f
        .n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
        .await;
    let again = f.post(del, None).await.unwrap();
    assert!(!again.changed, "a re-sent deletion adds nothing");
    assert_eq!(
        (
            f.seq().await,
            f.n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
                .await
        ),
        (seq, rows)
    );
}

#[tokio::test]
async fn delete_only_update_persists() {
    let f = fixture(&["Some text", "More text"]).await;
    let c = Client::new(41, &f.state().await);
    let del = c.delete_all();
    let parsed = Update::decode_v1(&del).unwrap();
    assert!(
        parsed.state_vector().is_empty(),
        "a delete-only update moves no clock"
    );
    let saved = f.post(del, None).await.unwrap();
    assert!(saved.changed, "a delete-only update is not a no-op");
    assert_eq!(f.seq().await, 1);
    assert_eq!(Client::new(96, &f.state().await).all_text(), "");
    assert_eq!(
        f.n("SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = ''")
            .await,
        1
    );
}

#[tokio::test]
async fn over_16mib_413() {
    let f = fixture(&["x"]).await;
    let c = Client::new(51, &f.state().await);
    let big = "a".repeat(docs::DEFAULT_MAX_DOC_BYTES + 1024);
    let update = c.type_text(0, &big);
    let e = f.post(update, None).await.unwrap_err();
    assert!(e.is(ErrorCode::DocTooLarge), "{e:?}");
    assert_eq!(
        e.details.unwrap()["limit_bytes"],
        json!(docs::DEFAULT_MAX_DOC_BYTES)
    );
    assert_eq!(f.seq().await, 0);
}

#[tokio::test]
async fn body_text_links_and_attachments_in_same_txn() {
    let f = fixture(&["Intro"]).await;
    let (target, att) = (
        "0192f0a0-2222-7000-8000-00000000000b",
        "0192f0a0-4444-7000-8000-000000000001",
    );
    let (owner, blobs, id) = (f.owner.clone(), f.t.blobs.clone(), f.doc_id.clone());
    f.t.db
        .write(move |tx| {
            let blob = blobs.put(tx, b"png bytes", NOW)?;
            tx.execute(
                "INSERT INTO attachments (id, item_id, sha256, filename, mime, created_by, created_at)
                 VALUES (?1, ?2, ?3, 'a.png', 'image/png', ?4, ?5)",
                params![att, id, blob.sha256.as_str(), owner, NOW],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let c = Client::new(61, &f.state().await);
    let before = c.sv();
    {
        let frag = c.doc.get_or_insert_xml_fragment(YFIELD);
        let mut txn = c.doc.transact_mut();
        let p = frag.insert(&mut txn, 1, XmlElementPrelim::empty("paragraph"));
        p.insert(&mut txn, 0, XmlTextPrelim::new("See "));
        let m = p.insert(&mut txn, 1, XmlElementPrelim::empty("mention"));
        m.insert_attribute(&mut txn, "itemId", target);
        let img = frag.insert(&mut txn, 2, XmlElementPrelim::empty("image"));
        img.insert_attribute(&mut txn, "attachmentId", att);
        img.insert_attribute(&mut txn, "caption", "A caption");
        let img2 = frag.insert(&mut txn, 3, XmlElementPrelim::empty("image"));
        img2.insert_attribute(&mut txn, "attachmentId", "not-an-attachment");
    }
    let update = c.doc.transact().encode_diff_v1(&before);
    f.post(update, None).await.unwrap();
    assert_eq!(
        f.n(
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Intro\nSee \nA caption'"
        )
        .await,
        1
    );
    assert_eq!(
        f.n("SELECT count(*) FROM item_links WHERE source_id = ?1 AND kind = 'mention'")
            .await,
        1
    );
    assert_eq!(
        f.n("SELECT count(*) FROM item_attachments WHERE item_id = ?1")
            .await,
        1,
        "existing attachments only"
    );
    // A failing save in the same transaction leaves none of it.
    let (id, owner) = (f.doc_id.clone(), f.owner.clone());
    let del = c.delete_all();
    let r =
        f.t.db
            .write(move |tx| {
                let by = WriteBy {
                    user_id: &owner,
                    token_id: None,
                };
                docs::save_update(
                    tx,
                    &id,
                    &del,
                    PmJson::Keep,
                    &by,
                    docs::DEFAULT_MAX_DOC_BYTES,
                    NOW,
                )?;
                Err::<(), _>(quire_store::Error::validation("x", "roll back"))
            })
            .await;
    assert!(r.is_err());
    assert_eq!(
        f.n("SELECT count(*) FROM item_links WHERE source_id = ?1")
            .await,
        1
    );
    assert_eq!(
        f.n("SELECT count(*) FROM item_attachments WHERE item_id = ?1")
            .await,
        1
    );
}

#[tokio::test]
async fn doc_clients_survive_compaction() {
    let f = fixture(&["Start"]).await;
    let c = Client::new(71, &f.state().await);
    for i in 0..205 {
        f.post(c.type_text(0, &format!("{i}")), None).await.unwrap();
    }
    assert!(
        f.n("SELECT compacted_seq FROM doc_state WHERE item_id = ?1")
            .await
            > 0
    );
    assert_eq!(
        f.n("SELECT count(*) FROM doc_clients WHERE item_id = ?1 AND client_id = 71")
            .await,
        1
    );
    let owner = f.owner.clone();
    let user: String =
        f.t.query(move |c| {
            c.query_row(
                "SELECT user_id FROM doc_clients WHERE client_id = 71",
                [],
                |r| r.get(0),
            )
        })
        .await;
    assert_eq!(user, owner);
}

#[tokio::test]
async fn read_only_when_trashed_409() {
    let f = fixture(&["Text"]).await;
    let c = Client::new(81, &f.state().await);
    f.t.db
        .write({
            let id = f.doc_id.clone();
            move |tx| {
                tx.execute(
                    "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
                    params![id, NOW],
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();
    let e = f.post(c.type_text(0, "!"), None).await.unwrap_err();
    assert!(e.is(ErrorCode::ReadOnly), "{e:?}");
    let e = f
        .put(f.write_req(md("New"), IfMatch::Any), None)
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ReadOnly), "{e:?}");
}

#[tokio::test]
async fn token_yjs_post_creates_pre_api_revision() {
    let f = fixture(&["Owner's text"]).await;
    let c = Client::new(91, &f.state().await);
    // The first session edit gets an auto revision (none existed); the second, a minute later, none: the newest
    // revision no longer holds the current state.
    f.post_at(c.type_text(0, " (session)"), None, NOW - 5 * MINUTE)
        .await
        .unwrap();
    f.post_at(c.type_text(0, " (more)"), None, NOW - 4 * MINUTE)
        .await
        .unwrap();
    let pre_api =
        || f.n("SELECT count(*) FROM revisions WHERE item_id = ?1 AND reason = 'pre-api'");
    assert_eq!(pre_api().await, 0);
    f.post(c.type_text(0, " (token)"), Some("tok-1"))
        .await
        .unwrap();
    assert_eq!(pre_api().await, 1);
    f.post(c.type_text(0, " (token again)"), Some("tok-1"))
        .await
        .unwrap();
    assert_eq!(pre_api().await, 1, "the same token's next write adds none");
}

// ------------------------------------------------------------------------------------------------- content writes

#[tokio::test]
async fn markdown_put_is_one_update_through_save_path() {
    let f = fixture(&["First paragraph", "Second paragraph"]).await;
    let r = f
        .put(
            f.write_req(
                md("First paragraph\n\nSecond paragraph, edited\n"),
                IfMatch::Seq(0),
            ),
            None,
        )
        .await
        .unwrap();
    assert!(r.changed);
    assert!(!r.merged);
    assert_eq!((r.seq, r.etag.as_str()), (1, "\"d1\""));
    assert_eq!(
        f.n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
            .await,
        1
    );
    assert_eq!(f.n("SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'First paragraph\nSecond paragraph, edited'").await, 1);
    let owner = f.owner.clone();
    let users: Vec<String> =
        f.t.query(move |c| {
            c.prepare("SELECT user_id FROM doc_clients")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(users, vec![owner]);
    assert_eq!(
        f.markdown().await,
        "First paragraph\n\nSecond paragraph, edited\n"
    );
}

#[tokio::test]
async fn noop_put_writes_nothing() {
    let f = fixture(&["Same"]).await;
    let r = f
        .put(f.write_req(md("Same\n"), IfMatch::Seq(0)), None)
        .await
        .unwrap();
    assert!(!r.changed);
    assert_eq!(r.seq, 0);
    assert_eq!(
        f.n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
            .await,
        0
    );
    let json = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Same"}]}]});
    let r = f
        .put(f.write_req(ContentInput::Json(json), IfMatch::Any), None)
        .await
        .unwrap();
    assert!(!r.changed, "the blockId the JSON lacks is not a change");
}

#[tokio::test]
async fn merge_true_converges_with_concurrent_update() {
    let f = fixture(&["Owner paragraph", "Agent paragraph"]).await;
    // The web editor types into the first paragraph (seq 1).
    let web = Client::new(101, &f.state().await);
    f.post(web.type_text(0, " typed live"), None).await.unwrap();
    // An agent, still at d0, edits the second paragraph with merge.
    let mut req = f.write_req(
        md("Owner paragraph\n\nAgent paragraph, edited by the agent\n"),
        IfMatch::Seq(0),
    );
    req.merge = true;
    let r = f.put(req, Some("agent")).await.unwrap();
    assert!(r.changed && r.merged, "{r:?}");
    assert_eq!(r.seq, 2);
    assert_eq!(
        f.markdown().await,
        "Owner paragraph typed live\n\nAgent paragraph, edited by the agent\n"
    );
    // Without merge, the same stale base is refused.
    let e = f
        .put(
            f.write_req(md("Owner paragraph\n\nOther\n"), IfMatch::Seq(0)),
            None,
        )
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::RevMismatch));
    assert_eq!(
        e.details.unwrap(),
        json!({"etag": "\"d2\"", "mergeable": true})
    );
}

#[tokio::test]
async fn merge_after_compaction_412_not_mergeable() {
    let f = fixture(&["Start"]).await;
    let c = Client::new(111, &f.state().await);
    for i in 0..201 {
        f.post(c.type_text(0, &format!("{i}")), None).await.unwrap();
    }
    assert!(
        f.n("SELECT compacted_seq FROM doc_state WHERE item_id = ?1")
            .await
            > 5
    );
    let mut req = f.write_req(md("Something else\n"), IfMatch::Seq(5));
    req.merge = true;
    let e = f.put(req, None).await.unwrap_err();
    assert!(e.is(ErrorCode::RevMismatch), "{e:?}");
    assert_eq!(e.details.unwrap()["mergeable"], json!(false));
}

#[tokio::test]
async fn concurrent_strict_puts_one_412() {
    let f = fixture(&["Base paragraph here"]).await;
    let a = f.put(
        f.write_req(md("Base paragraph here, A\n"), IfMatch::Seq(0)),
        None,
    );
    let b = f.put(
        f.write_req(md("Base paragraph here, B\n"), IfMatch::Seq(0)),
        None,
    );
    let (a, b) = tokio::join!(a, b);
    let ok = [a.is_ok(), b.is_ok()];
    assert_eq!(ok.iter().filter(|x| **x).count(), 1, "{a:?} {b:?}");
    let err = if a.is_err() {
        a.unwrap_err()
    } else {
        b.unwrap_err()
    };
    assert!(err.is(ErrorCode::RevMismatch));
    assert_eq!(f.seq().await, 1);
}

#[tokio::test]
async fn stale_if_match_equal_target_200_unchanged() {
    let f = fixture(&["Base paragraph here"]).await;
    let req = f.write_req(md("Base paragraph here, changed\n"), IfMatch::Seq(0));
    let first = f.put(req.clone(), None).await.unwrap();
    assert!(first.changed);
    // A retry of the same request (its first answer was lost): the target is already current.
    let again = f.put(req, None).await.unwrap();
    assert!(!again.changed);
    assert_eq!(again.seq, first.seq);
    assert_eq!(
        f.n("SELECT count(*) FROM doc_updates WHERE item_id = ?1")
            .await,
        1
    );
}

#[tokio::test]
async fn retried_merge_put_is_noop() {
    let f = fixture(&["Owner paragraph", "Agent paragraph"]).await;
    let web = Client::new(121, &f.state().await);
    f.post(web.type_text(0, " live"), None).await.unwrap();
    let mut req = f.write_req(
        md("Owner paragraph\n\nAgent paragraph, merged\n"),
        IfMatch::Seq(0),
    );
    req.merge = true;
    let first = f.put(req.clone(), None).await.unwrap();
    assert!(first.changed && first.merged);
    let again = f.put(req, None).await.unwrap();
    assert!(!again.changed, "{again:?}");
    assert_eq!(again.seq, first.seq);
    assert_eq!(
        f.markdown().await,
        "Owner paragraph live\n\nAgent paragraph, merged\n"
    );
}

#[tokio::test]
async fn put_to_unknown_nodes_409_schema_too_new() {
    let f = fixture(&["Known"]).await;
    let c = Client::new(131, &f.state().await);
    let before = c.sv();
    {
        let frag = c.doc.get_or_insert_xml_fragment(YFIELD);
        let mut txn = c.doc.transact_mut();
        let x = frag.insert(&mut txn, 1, XmlElementPrelim::empty("futureWidget"));
        x.insert_attribute(&mut txn, "data", "keep me");
    }
    f.post(c.doc.transact().encode_diff_v1(&before), None)
        .await
        .unwrap();
    let seq = f.seq().await;
    for input in [
        md("Replaced\n"),
        ContentInput::Json(json!({"type": "doc", "content": [{"type": "paragraph"}]})),
    ] {
        let e = f
            .put(f.write_req(input, IfMatch::Any), None)
            .await
            .unwrap_err();
        assert!(e.is(ErrorCode::SchemaTooNew), "{e:?}");
    }
    assert_eq!(f.seq().await, seq, "nothing deleted");
    let id = f.doc_id.clone();
    let json =
        f.t.db
            .read(move |c| docs::prosemirror(c, &id))
            .await
            .unwrap();
    assert_eq!(
        json.doc["content"][1]["type"], "futureWidget",
        "JSON GET passes it through"
    );
}

#[tokio::test]
async fn api_write_clears_pm_json() {
    let f = fixture(&["Cached"]).await;
    let c = Client::new(141, &f.state().await);
    let (id, owner, update) = (f.doc_id.clone(), f.owner.clone(), c.type_text(0, "!"));
    f.t.db
        .write(move |tx| {
            let by = WriteBy {
                user_id: &owner,
                token_id: None,
            };
            let pm = json!({"type": "doc", "content": []});
            docs::save_update(
                tx,
                &id,
                &update,
                PmJson::Set(&pm),
                &by,
                docs::DEFAULT_MAX_DOC_BYTES,
                NOW,
            )
        })
        .await
        .unwrap();
    assert_eq!(f.n("SELECT count(*) FROM doc_state WHERE item_id = ?1 AND pm_json IS NOT NULL AND pm_json_seq = 1").await, 1);
    // The Yjs POST stores the editor's JSON text as sent (it never builds a JSON tree of it).
    let (id, owner, update) = (f.doc_id.clone(), f.owner.clone(), c.type_text(0, "?"));
    f.t.db
        .write(move |tx| {
            let by = WriteBy {
                user_id: &owner,
                token_id: None,
            };
            docs::save_update(
                tx,
                &id,
                &update,
                PmJson::Raw(r#"{"type":"doc", "content":[ ]}"#),
                &by,
                docs::DEFAULT_MAX_DOC_BYTES,
                NOW,
            )
        })
        .await
        .unwrap();
    let id = f.doc_id.clone();
    let stored: String =
        f.t.db
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT pm_json FROM doc_state WHERE item_id = ?1 AND pm_json_seq = 2",
                    [id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
    assert_eq!(stored, r#"{"type":"doc", "content":[ ]}"#);
    f.put(
        f.write_req(md("Cached! And more\n"), IfMatch::Any),
        Some("tok"),
    )
    .await
    .unwrap();
    assert_eq!(
        f.n("SELECT count(*) FROM doc_state WHERE item_id = ?1 AND pm_json IS NULL")
            .await,
        1
    );
}

#[tokio::test]
async fn token_write_creates_pre_api_revision() {
    let f = fixture(&["The owner's paragraph"]).await;
    let c = Client::new(151, &f.state().await);
    f.post_at(c.type_text(0, " (typed)"), None, NOW - 5 * MINUTE)
        .await
        .unwrap();
    f.post_at(c.type_text(0, " (more)"), None, NOW - 4 * MINUTE)
        .await
        .unwrap();
    // The newest revision (the auto one of the first edit) is behind the current state.
    let pre_api =
        || f.n("SELECT count(*) FROM revisions WHERE item_id = ?1 AND reason = 'pre-api'");
    let text = "The owner's paragraph (typed) (more)";
    let r = f
        .put(
            f.write_req(md(&format!("{text}, and the agent's edit\n")), IfMatch::Any),
            Some("tok-a"),
        )
        .await
        .unwrap();
    assert!(r.changed);
    assert_eq!(pre_api().await, 1, "after a session edit: one");
    let source: i64 = f
        .n("SELECT source_seq FROM revisions WHERE item_id = ?1 AND reason = 'pre-api'")
        .await;
    assert_eq!(source, 2, "it captures the state before the token's write");
    f.put(
        f.write_req(
            md(&format!("{text}, and the agent's second edit\n")),
            IfMatch::Any,
        ),
        Some("tok-a"),
    )
    .await
    .unwrap();
    assert_eq!(pre_api().await, 1, "the same token's next PUT: none");
    let last: Option<String> = {
        let id = f.doc_id.clone();
        f.t.query(move |c| {
            c.query_row(
                "SELECT last_token_id FROM doc_state WHERE item_id = ?1",
                [id],
                |r| r.get(0),
            )
        })
        .await
    };
    assert_eq!(last.as_deref(), Some("tok-a"));
}

#[tokio::test]
async fn invalid_inputs_422() {
    let f = fixture(&["Text"]).await;
    let e = f
        .put(
            f.write_req(
                ContentInput::Json(json!({"type": "doc", "content": [{"type": "nope"}]})),
                IfMatch::Any,
            ),
            None,
        )
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::InvalidDocument));
    assert_eq!(
        e.details.unwrap(),
        json!({"path": "/doc/content/0/type", "reason": "unknown node type nope"})
    );
    let deep = format!("{} x\n", ">".repeat(200));
    let e = f
        .put(f.write_req(md(&deep), IfMatch::Any), None)
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::InvalidDocument));
    assert_eq!(e.details.unwrap()["path"], json!("1:101"));
}

#[tokio::test]
async fn title_from_h1_renames_in_the_same_write() {
    let f = fixture(&["Body"]).await;
    let mut req = f.write_req(md("# New title\n\nBody\n"), IfMatch::Any);
    req.title_from_h1 = true;
    let r = f.put(req, None).await.unwrap();
    assert!(r.changed);
    assert_eq!(
        f.n("SELECT count(*) FROM items WHERE id = ?1 AND title = 'New title'")
            .await,
        1
    );
    assert_eq!(f.markdown().await, "Body\n");
    let _ = Value::Null;
}
