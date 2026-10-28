//! Item routes: create, get, patch with If-Match, publication, discard, text, moves, duplicate, stars, purge scope
//! (owner: W1).
//!
//! The helpers at the top ([`collection`], [`item`], [`get`]) are shared with the other W1 modules (`tree`,
//! `lists`, `search`).

use axum::http::StatusCode;
use quire_api::{
    Collection, ErrorCode, ItemDetail, ItemSummary, ItemText, ListedItem, Page, RestoredItem,
    Scope, Sidebar,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::{TestApp, assert_error};

/// Creates a collection through the API.
pub async fn collection(app: &TestApp, name: &str) -> Collection {
    let res = app
        .server
        .post("/api/v1/collections")
        .json(&json!({ "name": name }))
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

/// Creates an item through the API.
pub async fn item(
    app: &TestApp,
    collection: Uuid,
    parent: Option<Uuid>,
    kind: &str,
    title: &str,
) -> ItemSummary {
    let res = app
        .server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": collection, "parent_id": parent, "kind": kind, "title": title }))
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

/// `GET /items/{id}`.
pub async fn get(app: &TestApp, id: impl std::fmt::Display) -> ItemDetail {
    let res = app.server.get(&format!("/api/v1/items/{id}")).await;
    res.assert_status_ok();
    res.json()
}

/// The sidebar's drafts count.
pub async fn drafts_count(app: &TestApp) -> u64 {
    let res = app.server.get("/api/v1/sidebar").await;
    res.assert_status_ok();
    res.json::<Sidebar>().drafts_count
}

/// The titles of a collection's tree.
pub async fn tree_titles(app: &TestApp, collection: Uuid) -> Vec<String> {
    let res = app
        .server
        .get(&format!("/api/v1/collections/{collection}/tree"))
        .await;
    res.assert_status_ok();
    res.json::<Value>()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn create_is_published_and_initializes_content() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let before = drafts_count(&app).await;
    let doc = item(&app, c.id, None, "doc", "First").await;
    assert!(doc.published && !doc.archived, "{doc:?}");
    assert_eq!((doc.created_at, doc.content_at), (doc.updated_at, None));
    let board = item(&app, c.id, Some(doc.id), "board", "Sketch").await;
    assert_eq!(board.parent_id, Some(doc.id));
    let detail = get(&app, doc.id).await;
    assert!(detail.effectively_published && detail.item.published);
    assert_eq!((detail.item.child_count, detail.board.as_ref()), (1, None));
    let bdetail = get(&app, board.id).await;
    let meta = bdetail.board.expect("board meta");
    assert_eq!((meta.rev, meta.objects, meta.preview_rev), (1, 0, None));
    assert_eq!(meta.pages.len(), 1);
    assert_eq!(
        bdetail.breadcrumbs.iter().map(|c| c.id).collect::<Vec<_>>(),
        [doc.id]
    );
    assert_eq!(
        drafts_count(&app).await,
        before,
        "items are published on create (D23)"
    );
    // Both content rows exist.
    let (d, b) = (doc.id.to_string(), board.id.to_string());
    let (docs, boards): (i64, i64) = app
        .state
        .db
        .read(move |c| {
            Ok((
                c.query_row(
                    "SELECT count(*) FROM doc_state WHERE item_id = ?1",
                    [d],
                    |r| r.get(0),
                )?,
                c.query_row(
                    "SELECT count(*) FROM board_state WHERE item_id = ?1",
                    [b],
                    |r| r.get(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!((docs, boards), (1, 1));
    // Parent and collection rules.
    let res = app
        .server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": Uuid::now_v7(), "kind": "doc" }))
        .await;
    assert_error(&res, 409, ErrorCode::ParentInvalid);
    let res = app
        .server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": c.id, "kind": "doc", "title": "x".repeat(1001) }))
        .await;
    let body = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(body.error.details, Some(json!({"field": "title"})));
    let res = app
        .server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": c.id, "kind": "page" }))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn discard_empty_refuses_nonempty_409() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let discard = |id: Uuid| {
        app.server
            .delete(&format!("/api/v1/items/{id}?discard_empty=true"))
    };
    // A new, empty, published item is discarded for good.
    let fresh = item(&app, c.id, None, "doc", "").await;
    assert!(fresh.published);
    let res = discard(fresh.id).await;
    res.assert_status_ok();
    res.assert_json(&json!({ "trashed": 1 }));
    assert_error(
        &app.server.get(&format!("/api/v1/items/{}", fresh.id)).await,
        404,
        ErrorCode::NotFound,
    );
    let res = app.server.get("/api/v1/trash").await;
    assert_eq!(res.json::<Value>()["items"], json!([]), "not in the trash");
    // A titled one, one with a child, and an empty one older than 24 h: 409 not_empty, nothing deleted.
    let titled = item(&app, c.id, None, "doc", "Has a title").await;
    let parent = item(&app, c.id, None, "board", "").await;
    item(&app, c.id, Some(parent.id), "doc", "").await;
    let old = item(&app, c.id, None, "doc", "").await;
    app.exec(
        "UPDATE items SET created_at = created_at - 86400001 WHERE id = ?1",
        vec![old.id.to_string()],
    )
    .await;
    for id in [titled.id, parent.id, old.id] {
        assert_error(&discard(id).await, 409, ErrorCode::NotEmpty);
        get(&app, id).await;
    }
    // A plain DELETE still trashes it.
    let res = app
        .server
        .delete(&format!("/api/v1/items/{}", old.id))
        .await;
    res.assert_json(&json!({ "trashed": 1 }));
    assert!(get(&app, old.id).await.trashed_at.is_some());
}

#[tokio::test]
async fn unpublish_makes_draft_listed_in_drafts() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Quokka draft").await;
    item(&app, c.id, None, "doc", "Published").await;
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/unpublish", doc.id))
        .await;
    res.assert_status_ok();
    assert!(!res.json::<ItemSummary>().published);
    assert_eq!(drafts_count(&app).await, 1);
    let res = app.server.get("/api/v1/drafts").await;
    let page: Page<ListedItem> = res.json();
    assert_eq!(
        page.items.iter().map(|i| i.item.id).collect::<Vec<_>>(),
        [doc.id]
    );
    assert_eq!(page.next_cursor, None);
    // Single-user visibility ignores publication: the tree and the default search still show it.
    assert_eq!(tree_titles(&app, c.id).await, ["Quokka draft", "Published"]);
    let res = app.server.get("/api/v1/search?q=quokka").await;
    assert_eq!(res.json::<Value>()["items"].as_array().unwrap().len(), 1);
    assert!(!get(&app, doc.id).await.effectively_published);
}

#[tokio::test]
async fn publish_with_ancestors() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let parent = item(&app, c.id, None, "doc", "parent").await;
    let child = item(&app, c.id, Some(parent.id), "board", "child").await;
    for id in [child.id, parent.id] {
        app.server
            .post(&format!("/api/v1/items/{id}/unpublish"))
            .await
            .assert_status_ok();
    }
    assert_eq!(drafts_count(&app).await, 2);
    // Without ancestors (and with no body at all), only the child.
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/publish", child.id))
        .await;
    res.assert_status_ok();
    assert!(res.json::<ItemSummary>().published);
    assert!(!get(&app, child.id).await.effectively_published);
    assert_eq!(drafts_count(&app).await, 1);
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/publish", child.id))
        .json(&json!({ "with_ancestors": true }))
        .await;
    res.assert_status_ok();
    assert!(get(&app, child.id).await.effectively_published);
    assert!(get(&app, parent.id).await.item.published);
    assert_eq!(drafts_count(&app).await, 0);
    // A non-JSON body is refused.
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/publish", child.id))
        .text("with_ancestors=true")
        .content_type("text/plain")
        .await;
    assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
}

#[tokio::test]
async fn unpublish_always_allowed() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let parent = item(&app, c.id, None, "doc", "parent").await;
    let child = item(&app, c.id, Some(parent.id), "doc", "child").await;
    let grandchild = item(&app, c.id, Some(child.id), "doc", "grandchild").await;
    // A parent with published children, a child under a draft, a draft again: all 200.
    for id in [parent.id, child.id, child.id, grandchild.id] {
        let res = app
            .server
            .post(&format!("/api/v1/items/{id}/unpublish"))
            .await;
        res.assert_status_ok();
        assert!(!res.json::<ItemSummary>().published);
    }
    assert_eq!(drafts_count(&app).await, 3);
    // Moves, restores and unarchives need no publish rules either.
    app.server
        .post(&format!("/api/v1/items/{}/move", grandchild.id))
        .json(&json!({ "parent_id": null }))
        .await
        .assert_status_ok();
    app.server
        .delete(&format!("/api/v1/items/{}", parent.id))
        .await
        .assert_status_ok();
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/restore", parent.id))
        .await;
    res.assert_status_ok();
    assert!(!res.json::<RestoredItem>().item.published);
    // An item in the trash is read-only.
    app.server
        .delete(&format!("/api/v1/items/{}", parent.id))
        .await
        .assert_status_ok();
    assert_error(
        &app.server
            .post(&format!("/api/v1/items/{}/unpublish", parent.id))
            .await,
        409,
        ErrorCode::ReadOnly,
    );
}

#[tokio::test]
async fn get_by_url_id() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Findable").await;
    let by_url = get(&app, &doc.url_id).await;
    assert_eq!(by_url.item.id, doc.id);
    assert_eq!(by_url.collection.id, c.id);
    let res = app
        .server
        .get(&format!("/api/v1/collections/{}", c.url_id))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Collection>().id, c.id);
    for bad in ["AAAAAAAAAA", "short", "has-a-dash", "aB3dE5gH9k1"] {
        assert_error(
            &app.server.get(&format!("/api/v1/items/{bad}")).await,
            404,
            ErrorCode::NotFound,
        );
        assert_error(
            &app.server.get(&format!("/api/v1/collections/{bad}")).await,
            404,
            ErrorCode::NotFound,
        );
    }
    // Only GET takes a url_id; the other routes want the UUID.
    let res = app
        .server
        .patch(&format!("/api/v1/items/{}", doc.url_id))
        .json(&json!({"title": "x"}))
        .await;
    assert_error(&res, 404, ErrorCode::NotFound);
}

#[tokio::test]
async fn get_sets_meta_etag_and_patch_stale_if_match_412() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Old").await;
    let path = format!("/api/v1/items/{}", doc.id);
    let res = app.server.get(&path).await;
    assert_eq!(res.header("etag"), "\"m1\"");
    assert_eq!(res.json::<ItemDetail>().meta_version, 1);
    // Never 304, not even with a matching If-None-Match.
    let res = app
        .server
        .get(&path)
        .add_header("if-none-match", "\"m1\"")
        .await;
    res.assert_status_ok();
    let res = app
        .server
        .patch(&path)
        .add_header("if-match", "\"m1\"")
        .json(&json!({"title": "New"}))
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), "\"m2\"");
    assert_eq!(res.json::<ItemSummary>().title, "New");
    // The stale tag: 412 with the current one.
    let res = app
        .server
        .patch(&path)
        .add_header("if-match", "\"m1\"")
        .json(&json!({"title": "Lost"}))
        .await;
    let body = assert_error(&res, 412, ErrorCode::RevMismatch);
    assert_eq!(body.error.details, Some(json!({"etag": "\"m2\""})));
    assert_eq!(get(&app, doc.id).await.item.title, "New");
    // W/ is ignored, * and no If-Match win.
    let res = app
        .server
        .patch(&path)
        .add_header("if-match", "W/\"m2\"")
        .json(&json!({"full_width": true}))
        .await;
    assert_eq!(res.header("etag"), "\"m3\"");
    app.server
        .patch(&path)
        .add_header("if-match", "*")
        .json(&json!({"title": "Star"}))
        .await
        .assert_status_ok();
    app.server
        .patch(&path)
        .json(&json!({"title": "Plain"}))
        .await
        .assert_status_ok();
    let res = app
        .server
        .patch(&path)
        .add_header("if-match", "\"r5\"")
        .json(&json!({"title": "x"}))
        .await;
    assert_error(&res, 412, ErrorCode::RevMismatch);
    // A star changes the body but not the tag, and GET still answers 200.
    let res = app.server.get(&path).await;
    let etag = res.header("etag");
    assert!(!res.json::<ItemDetail>().starred);
    app.server
        .put(&format!("{path}/star"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let res = app
        .server
        .get(&path)
        .add_header("if-none-match", etag.clone())
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), etag);
    assert!(res.json::<ItemDetail>().starred);
    app.server
        .delete(&format!("{path}/star"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert!(!get(&app, doc.id).await.starred);
}

#[tokio::test]
async fn rename_bumps_tree_version() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Before").await;
    let tree = format!("/api/v1/collections/{}/tree", c.id);
    let etag = app.server.get(&tree).await.header("etag");
    app.server
        .get(&tree)
        .add_header("if-none-match", etag.clone())
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
    app.server
        .patch(&format!("/api/v1/items/{}", doc.id))
        .json(&json!({"title": "After"}))
        .await
        .assert_status_ok();
    let res = app
        .server
        .get(&tree)
        .add_header("if-none-match", etag.clone())
        .await;
    res.assert_status_ok();
    assert_ne!(res.header("etag"), etag);
    assert_eq!(tree_titles(&app, c.id).await, ["After"]);
    // full_width is not in the tree: the tag stays.
    let etag = res.header("etag");
    app.server
        .patch(&format!("/api/v1/items/{}", doc.id))
        .json(&json!({"full_width": true}))
        .await
        .assert_status_ok();
    app.server
        .get(&tree)
        .add_header("if-none-match", etag)
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn text_returns_derived_body() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "A doc").await;
    let board = item(&app, c.id, None, "board", "A board").await;
    let res = app
        .server
        .get(&format!("/api/v1/items/{}/text", doc.id))
        .await;
    res.assert_status_ok();
    let text: ItemText = res.json();
    assert_eq!(
        (text.kind.as_str(), text.title.as_str(), text.body.as_str()),
        ("doc", "A doc", "")
    );
    // A save sets the indexed body (W4's save path calls the same helper); the route returns it.
    let id = doc.id.to_string();
    app.state
        .db
        .write(move |tx| quire_store::shared::set_body_text(tx, &id, "first line\nsecond line"))
        .await
        .unwrap();
    let text: ItemText = app
        .server
        .get(&format!("/api/v1/items/{}/text", doc.id))
        .await
        .json();
    assert_eq!(text.body, "first line\nsecond line");
    // A new board's text is its page names.
    let text: ItemText = app
        .server
        .get(&format!("/api/v1/items/{}/text", board.id))
        .await
        .json();
    assert_eq!(
        (text.kind.as_str(), text.title.as_str(), text.body.as_str()),
        ("board", "A board", "Page 1")
    );
    assert_error(
        &app.server
            .get(&format!("/api/v1/items/{}/text", Uuid::now_v7()))
            .await,
        404,
        ErrorCode::NotFound,
    );
}

#[tokio::test]
async fn purge_needs_admin_scope() {
    let app = TestApp::with_token(Scope::Write).await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Doomed").await;
    app.server
        .delete(&format!("/api/v1/items/{}", doc.id))
        .await
        .assert_status_ok();
    for path in [
        format!("/api/v1/trash/items/{}", doc.id),
        format!("/api/v1/trash/collections/{}", c.id),
        "/api/v1/trash".to_owned(),
    ] {
        let body = assert_error(
            &app.server.delete(&path).await,
            403,
            ErrorCode::InsufficientScope,
        );
        assert_eq!(
            body.error.details,
            Some(json!({"required": "admin", "granted": ["read", "write"]}))
        );
    }
    let admin = app.token_client(&app.mint_token(app.owner(), Scope::Admin).await);
    admin
        .delete(&format!("/api/v1/trash/items/{}", doc.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_error(
        &app.server.get(&format!("/api/v1/items/{}", doc.id)).await,
        404,
        ErrorCode::NotFound,
    );
    // Purging a live item or collection is a conflict.
    let live = item(&app, c.id, None, "doc", "Live").await;
    assert_error(
        &admin
            .delete(&format!("/api/v1/trash/items/{}", live.id))
            .await,
        409,
        ErrorCode::Conflict,
    );
    assert_error(
        &admin
            .delete(&format!("/api/v1/trash/collections/{}", c.id))
            .await,
        409,
        ErrorCode::Conflict,
    );
    app.server
        .delete(&format!("/api/v1/collections/{}", c.id))
        .await
        .assert_status_ok();
    admin
        .delete(&format!("/api/v1/trash/collections/{}", c.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_error(
        &admin
            .post(&format!("/api/v1/collections/{}/restore", c.id))
            .await,
        404,
        ErrorCode::NotFound,
    );
    admin
        .delete("/api/v1/trash")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    // A read token cannot write at all.
    let read = app.token_client(&app.mint_token(app.owner(), Scope::Read).await);
    assert_error(
        &read
            .post(&format!("/api/v1/items/{}/archive", live.id))
            .await,
        403,
        ErrorCode::InsufficientScope,
    );
}

#[tokio::test]
async fn move_duplicate_archive_and_restore() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let other = collection(&app, "Other").await;
    let a = item(&app, c.id, None, "doc", "a").await;
    let b = item(&app, c.id, Some(a.id), "doc", "b").await;
    let z = item(&app, c.id, None, "board", "z").await;
    let mv = |id: Uuid, body: Value| {
        app.server
            .post(&format!("/api/v1/items/{id}/move"))
            .json(&body)
    };
    assert_error(
        &mv(a.id, json!({ "parent_id": b.id })).await,
        409,
        ErrorCode::Cycle,
    );
    assert_error(
        &mv(a.id, json!({ "parent_id": Uuid::now_v7() })).await,
        409,
        ErrorCode::ParentInvalid,
    );
    let body = assert_error(
        &mv(
            z.id,
            json!({ "parent_id": null, "position": { "after_id": b.id } }),
        )
        .await,
        422,
        ErrorCode::Validation,
    );
    assert_eq!(body.error.details, Some(json!({"field": "position"})));
    let res = mv(
        z.id,
        json!({ "parent_id": null, "position": { "before_id": a.id } }),
    )
    .await;
    res.assert_status_ok();
    assert_eq!(tree_titles(&app, c.id).await, ["z", "a", "b"]);
    let res = mv(
        a.id,
        json!({ "collection_id": other.id, "parent_id": null }),
    )
    .await;
    assert_eq!(res.json::<ItemSummary>().collection_id, other.id);
    assert_eq!(tree_titles(&app, other.id).await, ["a", "b"]);
    assert_eq!(get(&app, b.id).await.collection.id, other.id);
    // Duplicate: right after the original, " (copy)", children not copied.
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/duplicate", a.id))
        .await;
    res.assert_status(StatusCode::CREATED);
    let copy: ItemSummary = res.json();
    assert_eq!((copy.title.as_str(), copy.child_count), ("a (copy)", 0));
    assert_eq!(tree_titles(&app, other.id).await, ["a", "b", "a (copy)"]);
    // Archive and unarchive.
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/archive", a.id))
        .await;
    res.assert_json(&json!({ "archived": 2 }));
    assert_eq!(tree_titles(&app, other.id).await, ["a (copy)"]);
    assert_error(
        &app.server
            .post(&format!("/api/v1/items/{}/archive", b.id))
            .await,
        409,
        ErrorCode::ReadOnly,
    );
    let res = app
        .server
        .post(&format!("/api/v1/items/{}/unarchive", b.id))
        .await;
    let back: RestoredItem = res.json();
    assert_eq!((back.item.id, back.moved_to_top), (a.id, false));
    let raw: Value = app
        .server
        .post(&format!("/api/v1/items/{}/archive", a.id))
        .await
        .json();
    assert_eq!(raw["archived"], 2);
    let raw: Value = app
        .server
        .post(&format!("/api/v1/items/{}/unarchive", a.id))
        .await
        .json();
    assert_eq!(
        raw["moved_to_top"], false,
        "RestoredItem is the summary plus moved_to_top: {raw}"
    );
    assert_eq!(raw["id"], json!(a.id));
    // Trash a child, then its parent; restoring the child puts it at the top.
    app.server
        .delete(&format!("/api/v1/items/{}", b.id))
        .await
        .assert_json(&json!({ "trashed": 1 }));
    app.server
        .delete(&format!("/api/v1/items/{}", a.id))
        .await
        .assert_json(&json!({ "trashed": 1 }));
    let back: RestoredItem = app
        .server
        .post(&format!("/api/v1/items/{}/restore", b.id))
        .await
        .json();
    assert!(back.moved_to_top);
    assert_eq!(tree_titles(&app, other.id).await, ["a (copy)", "b"]);
    assert_error(
        &app.server
            .post(&format!("/api/v1/items/{}/restore", b.id))
            .await,
        409,
        ErrorCode::Conflict,
    );
    // Children of an item, and the view counter.
    let kids: Vec<ItemSummary> = app
        .server
        .get(&format!("/api/v1/items/{}/children", z.id))
        .await
        .json();
    assert!(kids.is_empty());
    app.server
        .post(&format!("/api/v1/items/{}/view", z.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let recent: Page<ListedItem> = app.server.get("/api/v1/recent").await.json();
    assert_eq!(
        recent.items.iter().map(|i| i.item.id).collect::<Vec<_>>(),
        [z.id]
    );
    assert_error(
        &app.server
            .post(&format!("/api/v1/items/{}/view", Uuid::now_v7()))
            .await,
        404,
        ErrorCode::NotFound,
    );
}
