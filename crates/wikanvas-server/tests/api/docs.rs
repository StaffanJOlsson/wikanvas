//! Document route tests (owner: W4)
//!
//! Yjs sync over HTTP (two clients converge, `?since`), the content routes (ETags, warnings, 304, media types,
//! `If-Match` rules, `?title=from_h1`), the two-permit PUT, and `GET /schema/doc`.

use std::time::Duration;

use axum::http::StatusCode;
use axum_test::TestServer;
use base64::Engine as _;
use serde_json::{Value, json};
use wikanvas_api::{
    DocJson, DocSchema, DocState, DocUpdateResult, DocWriteResult, ErrorCode, ItemKind,
    NewCollection, NewItem, Scope, YFIELD,
};
use wikanvas_core::convert::yrs;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Doc, GetString, Options, ReadTxn, StateVector, Text, Transact, Update, XmlElementPrelim,
    XmlFragment, XmlOut, XmlTextPrelim,
};

use crate::common::{TestApp, assert_error};

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn b64d(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

/// A new document in a new collection (store functions: W1's item routes are not here yet).
async fn new_doc(app: &TestApp, title: &str) -> String {
    let owner = app.owner().to_owned();
    let title = title.to_owned();
    let item = app
        .state
        .db
        .write(move |tx| {
            let by = wikanvas_store::Viewer {
                user_id: owner.clone(),
                role: wikanvas_api::Role::Owner,
            };
            let c = wikanvas_store::collections::create(
                tx,
                &by,
                &NewCollection {
                    name: "Notes".into(),
                    ..Default::default()
                },
                1,
            )?;
            wikanvas_store::items::create(
                tx,
                &owner,
                &NewItem {
                    collection_id: c.id,
                    parent_id: None,
                    kind: ItemKind::Doc,
                    title: Some(title),
                    position: None,
                },
                false,
                1,
            )
        })
        .await
        .expect("create a document");
    item.id.to_string()
}

/// A Y client of the HTTP API.
struct Client {
    doc: Doc,
}

impl Client {
    fn new(id: u64) -> Client {
        Client {
            doc: Doc::with_options(Options {
                offset_kind: yrs::OffsetKind::Utf16,
                ..Options::with_client_id(yrs::ClientID::new(id))
            }),
        }
    }

    async fn sync(&self, server: &TestServer, doc_id: &str) -> DocState {
        let sv = b64(&self.doc.transact().state_vector().encode_v1());
        let res = server
            .get(&format!("/api/v1/items/{doc_id}/doc"))
            .add_query_param("since", sv)
            .await;
        res.assert_status_ok();
        let state: DocState = res.json();
        self.doc
            .transact_mut()
            .apply_update(Update::decode_v1(&b64d(&state.update)).unwrap())
            .unwrap();
        state
    }

    fn edit(&self, f: impl FnOnce(&mut yrs::TransactionMut, &yrs::XmlFragmentRef)) -> Vec<u8> {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let before = self.doc.transact().state_vector();
        f(&mut self.doc.transact_mut(), &frag);
        self.doc.transact().encode_diff_v1(&before)
    }

    fn add_paragraph(&self, text: &str) -> Vec<u8> {
        self.edit(|txn, frag| {
            let n = frag.len(txn);
            let p = frag.insert(txn, n, XmlElementPrelim::empty("paragraph"));
            p.insert(txn, 0, XmlTextPrelim::new(text));
        })
    }

    fn texts(&self) -> Vec<String> {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let txn = self.doc.transact();
        frag.children(&txn)
            .filter_map(|c| match c {
                XmlOut::Element(e) => Some(
                    e.children(&txn)
                        .filter_map(|t| match t {
                            XmlOut::Text(t) => Some(t.get_string(&txn)),
                            _ => None,
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()
    }

    async fn post(
        &self,
        server: &TestServer,
        doc_id: &str,
        update: &[u8],
    ) -> axum_test::TestResponse {
        server
            .post(&format!("/api/v1/items/{doc_id}/doc/updates"))
            .json(&json!({"update": b64(update)}))
            .await
    }
}

#[tokio::test]
async fn two_clients_converge_over_http() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Shared").await;
    let (a, b) = (Client::new(1001), Client::new(1002));
    a.sync(&app.server, &id).await;
    b.sync(&app.server, &id).await;
    let ua = a.add_paragraph("From A");
    let ub = b.add_paragraph("From B");
    let ra = a.post(&app.server, &id, &ua).await;
    ra.assert_status_ok();
    let r: DocUpdateResult = ra.json();
    assert_eq!(r.seq, 1);
    b.post(&app.server, &id, &ub).await.assert_status_ok();
    a.sync(&app.server, &id).await;
    let state = b.sync(&app.server, &id).await;
    assert_eq!(state.seq, 2);
    assert_eq!(a.texts(), b.texts());
    assert_eq!(a.texts().len(), 2);
    assert_eq!(
        a.doc.transact().state_vector(),
        b.doc.transact().state_vector()
    );
    // A re-sent update is a no-op: same seq.
    let again: DocUpdateResult = a.post(&app.server, &id, &ua).await.json();
    assert_eq!(again.seq, 2);
}

#[tokio::test]
async fn since_returns_diff_only() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Diff").await;
    let a = Client::new(2001);
    a.sync(&app.server, &id).await;
    let big = a.add_paragraph(&"x".repeat(5000));
    a.post(&app.server, &id, &big).await.assert_status_ok();
    let b = Client::new(2002);
    b.sync(&app.server, &id).await;
    let small = a.add_paragraph("small");
    a.post(&app.server, &id, &small).await.assert_status_ok();
    let sv = b64(&b.doc.transact().state_vector().encode_v1());
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc"))
        .add_query_param("since", sv)
        .await;
    let state: DocState = res.json();
    let diff = b64d(&state.update);
    assert!(
        diff.len() < 200,
        "only the new paragraph: {} bytes",
        diff.len()
    );
    let full: DocState = app
        .server
        .get(&format!("/api/v1/items/{id}/doc"))
        .await
        .json();
    assert!(b64d(&full.update).len() > 5000);
    assert_eq!(
        (state.field.as_str(), state.schema_version, state.seq),
        ("default", 1, 2)
    );
    let sv = StateVector::decode_v1(&b64d(&state.state_vector)).unwrap();
    assert_eq!(sv, a.doc.transact().state_vector());
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc"))
        .add_query_param("since", "!!")
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn missing_deps_over_http_409() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Deps").await;
    let a = Client::new(3001);
    let _first = a.add_paragraph("one");
    let second = a.add_paragraph("two");
    let res = a.post(&app.server, &id, &second).await;
    let body = assert_error(&res, 409, ErrorCode::MissingDeps);
    assert!(body.error.details.unwrap()["state_vector"].is_string());
    let res = app
        .server
        .post(&format!("/api/v1/items/{id}/doc/updates"))
        .json(&json!({"update": b64(&[1, 2, 3])}))
        .await;
    assert_error(&res, 422, ErrorCode::InvalidUpdate);
}

async fn put_md(
    server: &TestServer,
    id: &str,
    md: &str,
    if_match: Option<&str>,
) -> axum_test::TestResponse {
    let mut req = server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .content_type("text/markdown")
        .bytes(md.to_owned().into_bytes().into());
    if let Some(m) = if_match {
        req = req.add_header("if-match", m.to_owned());
    }
    req.await
}

#[tokio::test]
async fn markdown_get_etag_and_warnings_header_never_304() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Title with spaces").await;
    let res = put_md(
        &app.server,
        &id,
        "Text\n\n<span>html</span>\n",
        Some("\"d0\""),
    )
    .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), "\"d1\"");
    let w: DocWriteResult = res.json();
    assert_eq!(w.warnings[0].code.as_str(), "html_as_text");
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/markdown"))
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "text/markdown; charset=utf-8");
    assert_eq!(res.header("etag"), "\"d1\"");
    // §11: never stored, sandboxed, not sniffed (the body is user text, raw HTML included).
    assert_eq!(res.header("cache-control"), "no-store");
    assert_eq!(
        res.header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(res.text(), "Text\n\n\\<span>html\\</span>\n");
    assert!(res.maybe_header("wikanvas-conversion-warnings").is_none());
    // Never 304, even with the current ETag.
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/markdown"))
        .add_header("if-none-match", "\"d1\"")
        .await;
    assert_eq!(res.status_code(), StatusCode::OK);
    // A lossy GET lists its warnings; ?title=include and ?download=true.
    let json = json!({"type": "doc", "content": [{"type": "paragraph"}, {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]});
    app.server
        .put(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .add_header("if-match", "*")
        .json(&json!({"doc": json}))
        .await
        .assert_status_ok();
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/markdown"))
        .add_query_param("title", "include")
        .add_query_param("download", "true")
        .await;
    assert_eq!(
        res.header("wikanvas-conversion-warnings"),
        "empty_paragraph_dropped"
    );
    assert_eq!(res.text(), "# Title with spaces\n\nx\n");
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename=\"title-with-spaces.md\""
    );
    assert_eq!(res.header("cache-control"), "no-store");
    assert_eq!(
        res.header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert_eq!(res.header("x-content-type-options"), "nosniff");
}

#[tokio::test]
async fn json_get_304() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "J").await;
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .await;
    res.assert_status_ok();
    let etag = res.header("etag");
    assert_eq!(etag, "\"d0\"");
    let doc: DocJson = res.json();
    assert_eq!(doc.seq, 0);
    assert_eq!(
        doc.doc,
        json!({"type": "doc"}),
        "a new document has no content yet"
    );
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .add_header("if-none-match", "W/\"d0\"")
        .await;
    assert_eq!(res.status_code(), StatusCode::NOT_MODIFIED);
    assert_eq!(res.header("etag"), "\"d0\"");
    put_md(&app.server, &id, "Changed\n", Some("*"))
        .await
        .assert_status_ok();
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .add_header("if-none-match", "\"d0\"")
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), "\"d1\"");
}

#[tokio::test]
async fn markdown_put_accepts_text_markdown_but_never_text_plain() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "M").await;
    for ct in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "application/json",
    ] {
        let res = app
            .server
            .put(&format!("/api/v1/items/{id}/doc/markdown"))
            .content_type(ct)
            .add_header("if-match", "*")
            .bytes("x".as_bytes().to_vec().into())
            .await;
        assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
    }
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .content_type("text/markdown; charset=utf-8")
        .add_header("if-match", "*")
        .bytes("ok".as_bytes().to_vec().into())
        .await;
    res.assert_status_ok();
    let res = put_md(&app.server, &id, &"x".repeat(1024 * 1024 + 1), Some("*")).await;
    let body = assert_error(&res, 413, ErrorCode::PayloadTooLarge);
    assert_eq!(
        body.error.details.unwrap()["limit_bytes"],
        json!(1024 * 1024)
    );
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .content_type("text/plain")
        .add_header("if-match", "*")
        .bytes("{}".as_bytes().to_vec().into())
        .await;
    assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
}

#[tokio::test]
async fn put_holds_two_permits() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Permits").await;
    let sem = app.state.doc_work.clone();
    assert_eq!(sem.available_permits(), 3);
    // A PUT in flight (two permits held): one Yjs POST still proceeds…
    let put = sem.clone().acquire_many_owned(2).await.unwrap();
    let a = Client::new(4001);
    a.sync(&app.server, &id).await;
    let u1 = a.add_paragraph("one");
    a.post(&app.server, &id, &u1).await.assert_status_ok();
    // …and while it (here: a read) holds the third, a second waits.
    let read = sem.clone().acquire_owned().await.unwrap();
    let u2 = a.add_paragraph("two");
    let server = app.client();
    let mut cookie_server = server;
    cookie_server.add_header(
        "cookie",
        format!("wikanvas_session={}", app.session.clone().unwrap()),
    );
    cookie_server.add_header("host", "localhost");
    cookie_server.add_header("sec-fetch-site", "same-origin");
    let path = format!("/api/v1/items/{id}/doc/updates");
    let body = json!({"update": b64(&u2)});
    let waiting =
        tokio::spawn(async move { cookie_server.post(&path).json(&body).await.status_code() });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "the second POST waits for a permit");
    drop(read);
    assert_eq!(waiting.await.unwrap(), StatusCode::OK);
    drop(put);
    // Two concurrent PUTs both finish (each takes its two permits at once).
    let s1 = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .content_type("text/markdown")
        .add_header("if-match", "*")
        .bytes("one\n\ntwo\n\nA\n".as_bytes().to_vec().into());
    let s2 = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .content_type("text/markdown")
        .add_header("if-match", "*")
        .bytes("one\n\ntwo\n\nB\n".as_bytes().to_vec().into());
    let (r1, r2) = tokio::time::timeout(Duration::from_secs(20), async { tokio::join!(s1, s2) })
        .await
        .expect("no deadlock");
    r1.assert_status_ok();
    r2.assert_status_ok();
    assert_eq!(sem.available_permits(), 3);
}

#[tokio::test]
async fn put_without_if_match_428() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "P").await;
    let res = put_md(&app.server, &id, "x\n", None).await;
    assert_error(&res, 428, ErrorCode::PreconditionRequired);
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .json(&json!({"doc": {"type": "doc"}}))
        .await;
    assert_error(&res, 428, ErrorCode::PreconditionRequired);
}

#[tokio::test]
async fn put_stale_if_match_412_with_mergeable() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "S").await;
    put_md(
        &app.server,
        &id,
        "First paragraph of text\n",
        Some("\"d0\""),
    )
    .await
    .assert_status_ok();
    let res = put_md(
        &app.server,
        &id,
        "Something completely different\n",
        Some("\"d0\""),
    )
    .await;
    let body = assert_error(&res, 412, ErrorCode::RevMismatch);
    assert_eq!(
        body.error.details.unwrap(),
        json!({"etag": "\"d1\"", "mergeable": true})
    );
    // Merge it instead.
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .add_query_param("merge", "true")
        .content_type("text/markdown")
        .add_header("if-match", "\"d0\"")
        .bytes(
            "First paragraph of text\n\nAdded by merge\n"
                .as_bytes()
                .to_vec()
                .into(),
        )
        .await;
    res.assert_status_ok();
    let w: DocWriteResult = res.json();
    assert!(w.merged);
    // A garbage If-Match is stale.
    let res = put_md(&app.server, &id, "x\n", Some("\"r1\"")).await;
    assert_error(&res, 412, ErrorCode::RevMismatch);
}

#[tokio::test]
async fn put_if_match_star_overwrites() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Star").await;
    put_md(&app.server, &id, "One\n", Some("*"))
        .await
        .assert_status_ok();
    let res = put_md(&app.server, &id, "Two\n\nThree\n", Some("*")).await;
    res.assert_status_ok();
    let w: DocWriteResult = res.json();
    assert!(w.changed && !w.merged);
    assert_eq!(
        app.server
            .get(&format!("/api/v1/items/{id}/doc/markdown"))
            .await
            .text(),
        "Two\n\nThree\n"
    );
}

#[tokio::test]
async fn title_from_h1_sets_item_title() {
    let app = TestApp::with_token(Scope::Write).await;
    let id = new_doc(&app, "Old title").await;
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .add_query_param("title", "from_h1")
        .content_type("text/markdown")
        .add_header("if-match", "*")
        .bytes("# The new title\n\nBody text\n".as_bytes().to_vec().into())
        .await;
    res.assert_status_ok();
    let id2 = id.clone();
    let title: String = app
        .state
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT title FROM items WHERE id = ?1", [id2], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(title, "The new title");
    assert_eq!(
        app.server
            .get(&format!("/api/v1/items/{id}/doc/markdown"))
            .await
            .text(),
        "Body text\n"
    );
}

#[tokio::test]
async fn open_session_sees_api_write_on_since() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Live").await;
    let web = Client::new(5001);
    web.sync(&app.server, &id).await;
    let u = web.add_paragraph("Typed in the editor");
    web.post(&app.server, &id, &u).await.assert_status_ok();
    // An agent writes through the API (a token), while the editor is open at seq 1.
    let token = app.mint_token(app.owner(), Scope::Write).await;
    let agent = app.token_client(&token);
    let res = agent
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .content_type("text/markdown")
        .add_header("if-match", "\"d1\"")
        .bytes(
            "Typed in the editor\n\nAdded by the agent\n"
                .as_bytes()
                .to_vec()
                .into(),
        )
        .await;
    res.assert_status_ok();
    let state = web.sync(&app.server, &id).await;
    assert_eq!(state.seq, 2);
    assert_eq!(
        web.texts(),
        vec!["Typed in the editor", "Added by the agent"]
    );
    // The editor's own unsent edit still merges.
    let mine = web.edit(|txn, frag| {
        let Some(XmlOut::Element(p)) = frag.get(txn, 0) else {
            panic!()
        };
        let Some(XmlOut::Text(t)) = p.get(txn, 0) else {
            panic!()
        };
        let n = t.len(txn);
        t.insert(txn, n, "!");
    });
    web.post(&app.server, &id, &mine).await.assert_status_ok();
    assert_eq!(
        app.server
            .get(&format!("/api/v1/items/{id}/doc/markdown"))
            .await
            .text(),
        "Typed in the editor!\n\nAdded by the agent\n"
    );
}

#[tokio::test]
async fn schema_doc_route_matches_fixture() {
    let app = TestApp::with_token(Scope::Read).await;
    let res = app.server.get("/api/v1/schema/doc").await;
    res.assert_status_ok();
    let served: Value = res.json();
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/schema/v1.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(served, fixture);
    let _: DocSchema = res.json();
}

#[tokio::test]
async fn prosemirror_put_validates_with_a_pointer() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "V").await;
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .add_header("if-match", "*")
        .json(&json!({"doc": {"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]}]}]}}))
        .await;
    let body = assert_error(&res, 422, ErrorCode::InvalidDocument);
    assert_eq!(
        body.error.details.unwrap()["path"],
        "/doc/content/0/content/0/marks/0/attrs/href"
    );
    let res = app
        .server
        .put(&format!("/api/v1/items/{id}/doc/prosemirror"))
        .add_header("if-match", "*")
        .json(&json!({"schema_version": 2, "doc": {"type": "doc", "content": [{"type": "paragraph"}]}}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

/// Markdown whose nesting spreads over lines (several markers per line, the next lines indented to the innermost
/// item) is 422 `invalid_document`, and the server keeps serving: this body once nested about 950 levels and
/// reached the recursive block conversion.
#[tokio::test]
async fn markdown_put_nested_over_lines_422_and_server_survives() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Deep").await;
    let mut md = String::new();
    let mut markers = 0;
    for k in 1..=10 {
        md.push_str(&" ".repeat(2 * markers));
        md.push_str(&"- ".repeat(100 - k));
        md.push_str("x\n");
        markers += 100 - k;
    }
    let res = put_md(&app.server, &id, &md, Some("*")).await;
    let body = assert_error(&res, 422, ErrorCode::InvalidDocument);
    assert!(
        body.error.details.unwrap()["reason"]
            .as_str()
            .unwrap()
            .contains("deeper than 100")
    );
    // A tab-indented variant, and one just inside the limit.
    let tabs: String = (0..300)
        .map(|i| format!("{}- x\n", "\t".repeat(i)))
        .collect();
    let res = put_md(&app.server, &id, &tabs, Some("*")).await;
    assert_error(&res, 422, ErrorCode::InvalidDocument);
    let ok: String = (0..100)
        .map(|i| format!("{}- x\n", "  ".repeat(i)))
        .collect();
    put_md(&app.server, &id, &ok, Some("*"))
        .await
        .assert_status_ok();
    app.server
        .get(&format!("/api/v1/items/{id}/doc/markdown"))
        .await
        .assert_status_ok();
}

/// A writing route takes its `doc_work` permits before it reads the body: with every permit held, a PUT or a Yjs
/// POST with a malformed body waits instead of answering 422 at once, and answers it once the permits are free.
#[tokio::test]
async fn doc_writes_parse_bodies_under_permits() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Permits first").await;
    let sem = app.state.doc_work.clone();
    let all = sem.clone().acquire_many_owned(3).await.unwrap();
    let session = app.session.clone().unwrap();
    let put = app.cookie_client(&session);
    let path = format!("/api/v1/items/{id}/doc/prosemirror");
    let put = tokio::spawn(async move {
        put.put(&path)
            .add_header("if-match", "*")
            .content_type("application/json")
            .bytes("{not json".as_bytes().to_vec().into())
            .await
            .status_code()
    });
    let post = app.cookie_client(&session);
    let path = format!("/api/v1/items/{id}/doc/updates");
    let post = tokio::spawn(async move {
        post.post(&path)
            .content_type("application/json")
            .bytes("[0,0,0".as_bytes().to_vec().into())
            .await
            .status_code()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !put.is_finished() && !post.is_finished(),
        "both wait for permits"
    );
    drop(all);
    assert_eq!(put.await.unwrap(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(post.await.unwrap(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(sem.available_permits(), 3);
}

/// `pm_json` is stored as the text it was sent as; it must be a JSON object, and null is the same as absent.
#[tokio::test]
async fn yjs_post_pm_json_is_an_object() {
    let app = TestApp::with_owner().await;
    let id = new_doc(&app, "Cache").await;
    let a = Client::new(4101);
    a.sync(&app.server, &id).await;
    let u = a.add_paragraph("one");
    let path = format!("/api/v1/items/{id}/doc/updates");
    let res = app
        .server
        .post(&path)
        .json(&json!({"update": b64(&u), "pm_json": [0, 0, 0]}))
        .await;
    let body = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(body.error.details.unwrap()["field"], "pm_json");
    let body = format!(
        r#"{{"update": "{}", "pm_json": {{"type":"doc",  "content":[]}}}}"#,
        b64(&u)
    );
    app.server
        .post(&path)
        .content_type("application/json")
        .bytes(body.into_bytes().into())
        .await
        .assert_status_ok();
    let id2 = id.clone();
    let stored: String = app
        .state
        .db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT pm_json FROM doc_state WHERE item_id = ?1",
                [id2],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(stored, r#"{"type":"doc",  "content":[]}"#);
    let u2 = a.add_paragraph("two");
    app.server
        .post(&path)
        .json(&json!({"update": b64(&u2), "pm_json": null}))
        .await
        .assert_status_ok();
}
