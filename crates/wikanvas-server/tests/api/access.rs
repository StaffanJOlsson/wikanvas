//! Access per collection (D152): the leak suite, the effective-access rules through the API, the members and
//! directory routes, and the tree ETag across a change of access (phase 3 of docs/multi-user.md).
//!
//! The leak suite puts everything a member must not see in a private (`none`) collection, every title and text with
//! one marker word, and points at it from a readable collection: a document that mentions, links and embeds the
//! private items, a board with a card for the private document, an item whose parent (by a corrupt tree) is private.
//! A member without a membership must find the marker nowhere: not in any list, search, title search, tree, export,
//! board, preview, text or error; the private items are 404 as missing ones are. Then a `read` membership shows them
//! all, still read-only; then `write` lets the member edit.

use axum::http::StatusCode;
use axum_test::{TestResponse, TestServer};
use base64::Engine as _;
use serde_json::{Value, json};
use wikanvas_api::{
    Collection, CollectionAccess, CollectionMember, CollectionTree, DefaultAccess, DirectoryUser,
    ErrorCode, ItemSummary, MemberAccess, Role, Sidebar,
};

use crate::common::{TestApp, assert_error};
use crate::items::{collection, item};

/// The word every private title and text holds. One token, so search can look for it.
const MARKER: &str = "zqxsecretzq";

/// The fixture `fixtures/boards/embed.inkboard` (§18.2): two cards and a plain box.
fn embed_fixture() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/boards/embed.inkboard");
    std::fs::read_to_string(&path).expect("the embed fixture")
}
const P1: &str = "0192f0e0-0000-7000-8000-000000000001";
const P2: &str = "0192f0e0-0000-7000-8000-000000000002";

/// The response's whole text must not hold the marker (any case).
#[track_caller]
fn clean(res: &TestResponse, what: &str) {
    let text = res.text().to_lowercase();
    assert!(
        !text.contains(MARKER),
        "{what} shows the marker: {}",
        res.text()
    );
}

/// GET `path` as `client`: 2xx, and no marker.
#[track_caller]
fn ok_clean(res: TestResponse, what: &str) -> TestResponse {
    assert!(
        res.status_code().is_success(),
        "{what}: {} {}",
        res.status_code(),
        res.text()
    );
    clean(&res, what);
    res
}

/// A 404 worded as for a missing resource, without the marker.
#[track_caller]
fn missing(res: TestResponse, what: &str) {
    let body = assert_error(&res, 404, ErrorCode::NotFound);
    assert!(
        body.error.message.starts_with("no such "),
        "{what}: {}",
        body.error.message
    );
    clean(&res, what);
}

async fn put_markdown(client: &TestServer, id: uuid::Uuid, md: &str) -> TestResponse {
    client
        .put(&format!("/api/v1/items/{id}/doc/markdown"))
        .add_header("if-match", "*")
        .bytes(md.to_owned().into())
        .content_type("text/markdown")
        .await
}

async fn set_member(app: &TestApp, collection: uuid::Uuid, user: &str, access: &str) {
    let res = app
        .server
        .put(&format!("/api/v1/collections/{collection}/members/{user}"))
        .json(&json!({ "access": access }))
        .await;
    res.assert_status_ok();
}

async fn remove_member(app: &TestApp, collection: uuid::Uuid, user: &str) {
    app.server
        .delete(&format!("/api/v1/collections/{collection}/members/{user}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

async fn set_default(app: &TestApp, collection: uuid::Uuid, access: &str) -> Collection {
    let res = app
        .server
        .patch(&format!("/api/v1/collections/{collection}"))
        .json(&json!({ "default_access": access }))
        .await;
    res.assert_status_ok();
    res.json()
}

/// Everything the suite needs.
struct World {
    member_id: String,
    member: TestServer,
    open: Collection,
    private: Collection,
    gone: Collection,
    secret: ItemSummary,
    secret_board: ItemSummary,
    child: ItemSummary,
    readable: ItemSummary,
    board: ItemSummary,
    orphan: ItemSummary,
    attachment: String,
}

async fn world(app: &TestApp) -> World {
    let member_id = app.insert_user(Role::Member).await;
    let member = app.cookie_client(&app.insert_session(&member_id).await);
    let open = collection(app, "Open").await;
    let private = collection(app, &format!("{MARKER} private")).await;
    let private = set_default(app, private.id, "none").await;
    assert_eq!(private.default_access, DefaultAccess::None);
    // Private items: a document with a body, an image and a child, a board, an archived item, a draft and a
    // trashed item; and a private collection in the trash.
    let secret = item(app, private.id, None, "doc", &format!("{MARKER} plan")).await;
    let secret_board = item(app, private.id, None, "board", &format!("{MARKER} board")).await;
    let child = item(
        app,
        private.id,
        Some(secret.id),
        "doc",
        &format!("{MARKER} child"),
    )
    .await;
    let archived = item(app, private.id, None, "doc", &format!("{MARKER} archived")).await;
    app.server
        .post(&format!("/api/v1/items/{}/archive", archived.id))
        .await
        .assert_status_ok();
    let draft = item(app, private.id, None, "doc", &format!("{MARKER} draft")).await;
    app.server
        .post(&format!("/api/v1/items/{}/unpublish", draft.id))
        .await
        .assert_status_ok();
    let trashed = item(app, private.id, None, "doc", &format!("{MARKER} trashed")).await;
    app.server
        .delete(&format!("/api/v1/items/{}", trashed.id))
        .await
        .assert_status_ok();
    let gone = collection(app, &format!("{MARKER} gone")).await;
    set_default(app, gone.id, "none").await;
    item(app, gone.id, None, "doc", &format!("{MARKER} in gone")).await;
    app.server
        .delete(&format!("/api/v1/collections/{}", gone.id))
        .await
        .assert_status_ok();
    let attachment: Value = app
        .server
        .post(&format!(
            "/api/v1/attachments?filename={MARKER}.png&item_id={}",
            secret.id
        ))
        .bytes(crate::boards::PNG.to_vec().into())
        .content_type("application/octet-stream")
        .await
        .json();
    let attachment = attachment["id"].as_str().unwrap().to_owned();
    put_markdown(
        &app.server,
        secret.id,
        &format!("The {MARKER} budget.\n\n![{MARKER}](/api/v1/attachments/{attachment})\n"),
    )
    .await
    .assert_status_ok();
    // Readable items that point at the private ones: mentions, a link, an embedded board, a board card.
    let readable = item(app, open.id, None, "doc", "Readable page").await;
    put_markdown(
        &app.server,
        readable.id,
        &format!(
            "Readable page.\n\nSee [@x](wikanvas://item/{s}) and [a link](wikanvas://item/{s}) and \
             [@y](wikanvas://item/{c}).\n\n![x](wikanvas://item/{b})\n",
            s = secret.id,
            c = child.id,
            b = secret_board.id
        ),
    )
    .await
    .assert_status_ok();
    let board = item(app, open.id, None, "board", "Open board").await;
    let body = embed_fixture()
        .replace(P1, &secret.id.to_string())
        .replace(P2, &readable.id.to_string())
        .replace("First page", &format!("{MARKER} plan"))
        .replace("Second page", "Readable page");
    app.server
        .put(&format!("/api/v1/items/{}/board", board.id))
        .add_header("if-match", "*")
        .text(body)
        .content_type("application/json")
        .await
        .assert_status_ok();
    // A corrupt tree: a readable item whose parent is private.
    let orphan = item(app, open.id, None, "doc", "Orphan").await;
    let (o, s) = (orphan.id.to_string(), secret.id.to_string());
    app.state
        .db
        .write(move |tx| {
            // The trigger that keeps parents in their children's collection is set aside for the one update.
            let guard: String = tx.query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = 'items_parent_upd'",
                [],
                |r| r.get(0),
            )?;
            tx.execute_batch("DROP TRIGGER items_parent_upd")?;
            tx.execute(
                "UPDATE items SET parent_id = ?2 WHERE id = ?1",
                rusqlite::params![o, s],
            )?;
            tx.execute_batch(&guard)?;
            Ok(())
        })
        .await
        .unwrap();
    World {
        member_id,
        member,
        open,
        private,
        gone,
        secret,
        secret_board,
        child,
        readable,
        board,
        orphan,
        attachment,
    }
}

/// Every route a member could reach the marker through, without access to the private collection.
async fn assert_hidden(w: &World) {
    let m = &w.member;
    // The sidebar, the collection list, the trees.
    let sidebar: Sidebar = ok_clean(m.get("/api/v1/sidebar").await, "the sidebar").json();
    assert_eq!(
        sidebar.collections.iter().map(|c| c.id).collect::<Vec<_>>(),
        [w.open.id]
    );
    assert_eq!(sidebar.drafts_count, 0, "the one draft is private");
    assert!(
        sidebar.stars.is_empty(),
        "its star on a private item is hidden"
    );
    let list: Vec<Collection> =
        ok_clean(m.get("/api/v1/collections").await, "the collections").json();
    assert_eq!(list.len(), 1);
    ok_clean(
        m.get(&format!("/api/v1/collections/{}/tree", w.open.id))
            .await,
        "the open tree",
    );
    for path in [
        format!("/api/v1/collections/{}", w.private.id),
        format!("/api/v1/collections/{}", w.private.url_id),
        format!("/api/v1/collections/{}/tree", w.private.id),
        format!("/api/v1/collections/{}/updated", w.private.id),
        format!("/api/v1/search?q={MARKER}&collection_id={}", w.private.id),
    ] {
        missing(m.get(&path).await, &path);
    }
    // Every list.
    for list in ["recent", "updated", "drafts", "starred", "archive", "trash"] {
        ok_clean(m.get(&format!("/api/v1/{list}")).await, list);
    }
    ok_clean(
        m.get(&format!("/api/v1/collections/{}/updated", w.open.id))
            .await,
        "the open collection's updated",
    );
    // Search and title search: nothing for the marker, and no snippet of it on a readable hit.
    for q in [
        format!("q={MARKER}"),
        format!("q={MARKER}&include_archived=true"),
        format!("q={MARKER}&sort=updated"),
        "q=readable".to_owned(),
        "q=board".to_owned(),
        "q=page".to_owned(),
    ] {
        let res = ok_clean(m.get(&format!("/api/v1/search?{q}")).await, &q);
        if q.contains(MARKER) {
            assert_eq!(res.json::<Value>()["items"], json!([]), "{q}");
        }
    }
    for q in [MARKER, "", "plan", "board"] {
        let res = ok_clean(
            m.get(&format!("/api/v1/search/titles?q={q}&limit=20"))
                .await,
            &format!("titles {q}"),
        );
        if q == MARKER {
            assert_eq!(res.json::<Value>(), json!([]));
        }
    }
    // The private items themselves, by id and url_id, and everything under them.
    for id in [
        w.secret.id.to_string(),
        w.secret.url_id.clone(),
        w.child.id.to_string(),
        w.secret_board.id.to_string(),
    ] {
        missing(m.get(&format!("/api/v1/items/{id}")).await, &id);
    }
    for path in [
        format!("/api/v1/items/{}/children", w.secret.id),
        format!("/api/v1/items/{}/text", w.secret.id),
        format!("/api/v1/items/{}/doc", w.secret.id),
        format!("/api/v1/items/{}/doc/markdown", w.secret.id),
        format!("/api/v1/items/{}/doc/prosemirror", w.secret.id),
        format!("/api/v1/items/{}/board", w.secret_board.id),
        format!("/api/v1/items/{}/board/pages/0", w.secret_board.id),
        format!("/api/v1/attachments/{}", w.attachment),
    ] {
        missing(m.get(&path).await, &path);
    }
    // The readable document: its detail, children, text and exports (mentions and embeds print as missing items).
    for path in [
        format!("/api/v1/items/{}", w.readable.id),
        format!("/api/v1/items/{}/children", w.readable.id),
        format!("/api/v1/items/{}/text", w.readable.id),
        format!("/api/v1/items/{}/doc/prosemirror", w.readable.id),
    ] {
        ok_clean(m.get(&path).await, &path);
    }
    let md = ok_clean(
        m.get(&format!(
            "/api/v1/items/{}/doc/markdown?title=include",
            w.readable.id
        ))
        .await,
        "the readable markdown",
    )
    .text();
    assert!(md.contains("[@Deleted item]"), "{md}");
    assert!(md.contains("![Board]("), "{md}");
    let state: Value = ok_clean(
        m.get(&format!("/api/v1/items/{}/doc", w.readable.id)).await,
        "the readable doc",
    )
    .json();
    let update = base64::engine::general_purpose::STANDARD
        .decode(state["update"].as_str().unwrap())
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&update)
            .to_lowercase()
            .contains(MARKER),
        "the Yjs state"
    );
    // The board: its card for the private document is redacted in the GET, the download, the preview, the
    // detail's excerpt and the text.
    let board = ok_clean(
        m.get(&format!("/api/v1/items/{}/board", w.board.id)).await,
        "the board",
    );
    assert!(board.text().contains(wikanvas_core::board::REDACTED_LABEL));
    assert!(
        board.text().contains("Readable page"),
        "the readable card's label stays"
    );
    let envelope: Value = board.json();
    assert_eq!(
        envelope["document"]["pages"][0]["objects"][0]["kind"]["embed"]["item"],
        w.secret.id.to_string(),
        "the card itself stays"
    );
    ok_clean(
        m.get(&format!("/api/v1/items/{}/board.inkboard", w.board.id))
            .await,
        "the board download",
    );
    for query in ["", "?rev=1", "?rev=2"] {
        let res = m
            .get(&format!(
                "/api/v1/items/{}/board/pages/0{query}",
                w.board.id
            ))
            .await;
        if res.status_code() == StatusCode::TEMPORARY_REDIRECT {
            continue;
        }
        let res = ok_clean(res, &format!("the board preview{query}"));
        assert!(res.text().contains(wikanvas_core::board::REDACTED_LABEL));
        assert_eq!(res.header("cache-control"), "no-store");
        assert!(res.maybe_header("etag").is_none());
    }
    for path in [
        format!("/api/v1/items/{}", w.board.id),
        format!("/api/v1/items/{}/text", w.board.id),
    ] {
        ok_clean(m.get(&path).await, &path);
    }
    // Breadcrumbs stop at a parent in another collection (only a corrupt tree has one).
    let orphan: Value = ok_clean(
        m.get(&format!("/api/v1/items/{}", w.orphan.id)).await,
        "the orphan",
    )
    .json();
    assert_eq!(orphan["breadcrumbs"], json!([]));
    // The trash shows neither the private trashed item nor the private collection in the trash.
    let trash: Value = ok_clean(m.get("/api/v1/trash").await, "the trash").json();
    assert!(
        trash["collections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != w.gone.id.to_string())
    );
    // Writes to what the member may not read are 404 too, worded the same, and change nothing.
    let s = w.secret.id;
    for res in [
        m.patch(&format!("/api/v1/items/{s}"))
            .json(&json!({"title": "taken"}))
            .await,
        m.delete(&format!("/api/v1/items/{s}")).await,
        m.put(&format!("/api/v1/items/{s}/star")).await,
        m.post("/api/v1/items")
            .json(&json!({"collection_id": w.private.id, "kind": "doc"}))
            .await,
        m.post(&format!("/api/v1/items/{}/move", w.readable.id))
            .json(&json!({"collection_id": w.private.id, "parent_id": null}))
            .await,
        m.patch(&format!("/api/v1/collections/{}", w.private.id))
            .json(&json!({"default_access": "write"}))
            .await,
        m.get(&format!("/api/v1/collections/{}/members", w.private.id))
            .await,
        m.put(&format!(
            "/api/v1/collections/{}/members/{}",
            w.private.id, w.member_id
        ))
        .json(&json!({"access": "manage"}))
        .await,
    ] {
        missing(res, "a write to a private item");
    }
    // A collection position that names the private collection is as one that does not exist.
    let res = m
        .post("/api/v1/collections")
        .json(&json!({"name": "Mine", "position": {"after_id": w.private.id}}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn a_private_collection_leaks_nowhere() {
    let app = TestApp::with_owner().await;
    let w = world(&app).await;
    // The marker is really there: the owner finds it.
    let owner_hits: Value = app
        .server
        .get(&format!("/api/v1/search?q={MARKER}"))
        .await
        .json();
    assert!(
        owner_hits["items"].as_array().unwrap().len() >= 3,
        "{owner_hits}"
    );
    let owner_board = app
        .server
        .get(&format!("/api/v1/items/{}/board", w.board.id))
        .await;
    assert!(
        owner_board.text().contains(MARKER),
        "the owner's board is not redacted"
    );
    // The member starred and viewed the private document while it had access, then lost it.
    set_member(&app, w.private.id, &w.member_id, "read").await;
    for res in [
        w.member
            .put(&format!("/api/v1/items/{}/star", w.secret.id))
            .await,
        w.member
            .post(&format!("/api/v1/items/{}/view", w.secret.id))
            .await,
    ] {
        res.assert_status(StatusCode::NO_CONTENT);
    }
    let tree = w
        .member
        .get(&format!("/api/v1/collections/{}/tree", w.private.id))
        .await;
    tree.assert_status_ok();
    let etag = tree.header("etag").to_str().unwrap().to_owned();
    remove_member(&app, w.private.id, &w.member_id).await;
    // The tree it had cached is not confirmed: the collection is gone for it.
    missing(
        w.member
            .get(&format!("/api/v1/collections/{}/tree", w.private.id))
            .add_header("if-none-match", etag.clone())
            .await,
        "the lost tree",
    );
    assert_hidden(&w).await;
    // A member who writes the private attachment's id into a document it may write does not open the attachment:
    // through Markdown (the lookup does not know it) or a ProseMirror image that names it.
    put_markdown(
        &w.member,
        w.readable.id,
        &format!("Mine.\n\n![x](/api/v1/attachments/{})\n", w.attachment),
    )
    .await
    .assert_status_ok();
    let pm = json!({"doc": {"type": "doc", "content": [{"type": "image", "attrs": {
        "src": format!("/api/v1/attachments/{}", w.attachment), "attachmentId": w.attachment}}]}});
    w.member
        .put(&format!("/api/v1/items/{}/doc/prosemirror", w.readable.id))
        .add_header("if-match", "*")
        .json(&pm)
        .await
        .assert_status_ok();
    missing(
        w.member
            .get(&format!("/api/v1/attachments/{}", w.attachment))
            .await,
        "the private attachment",
    );

    // With `read`, all of it shows, read-only.
    set_member(&app, w.private.id, &w.member_id, "read").await;
    let res = w
        .member
        .get(&format!("/api/v1/collections/{}/tree", w.private.id))
        .add_header("if-none-match", etag)
        .await;
    res.assert_status_ok();
    let tree: CollectionTree = res.json();
    assert_eq!(tree.collection.my_access, CollectionAccess::Read);
    assert!(tree.items.iter().any(|i| i.id == w.secret.id));
    let sidebar: Sidebar = w.member.get("/api/v1/sidebar").await.json();
    assert_eq!(sidebar.collections.len(), 2);
    assert!(
        sidebar.stars.iter().any(|s| s.id == w.secret.id),
        "its star shows again"
    );
    let hits: Value = w
        .member
        .get(&format!("/api/v1/search?q={MARKER}"))
        .await
        .json();
    assert!(!hits["items"].as_array().unwrap().is_empty());
    for path in [
        format!("/api/v1/items/{}", w.secret.id),
        format!("/api/v1/items/{}/children", w.secret.id),
        format!("/api/v1/attachments/{}", w.attachment),
        "/api/v1/recent".to_owned(),
    ] {
        w.member.get(&path).await.assert_status_ok();
    }
    let board = w
        .member
        .get(&format!("/api/v1/items/{}/board", w.board.id))
        .await;
    assert!(board.text().contains(MARKER), "no card is redacted now");
    let md = w
        .member
        .get(&format!("/api/v1/items/{}/doc/markdown", w.secret.id))
        .await;
    assert!(md.text().contains(MARKER));
    for res in [
        w.member
            .patch(&format!("/api/v1/items/{}", w.secret.id))
            .json(&json!({"title": "changed"}))
            .await,
        put_markdown(&w.member, w.secret.id, "Changed.\n").await,
        w.member
            .post("/api/v1/items")
            .json(&json!({"collection_id": w.private.id, "kind": "doc"}))
            .await,
        w.member
            .patch(&format!("/api/v1/collections/{}", w.private.id))
            .json(&json!({"name": "changed"}))
            .await,
    ] {
        assert_error(&res, 403, ErrorCode::Forbidden);
    }

    // With `write`, the member edits, but still does not manage the collection.
    set_member(&app, w.private.id, &w.member_id, "write").await;
    w.member
        .patch(&format!("/api/v1/items/{}", w.secret.id))
        .json(&json!({"title": "changed"}))
        .await
        .assert_status_ok();
    put_markdown(&w.member, w.secret.id, "Changed.\n")
        .await
        .assert_status_ok();
    let res = w
        .member
        .get(&format!("/api/v1/collections/{}/members", w.private.id))
        .await;
    assert_error(&res, 403, ErrorCode::Forbidden);
    let c: Collection = w
        .member
        .get(&format!("/api/v1/collections/{}", w.private.id))
        .await
        .json();
    assert_eq!(c.my_access, CollectionAccess::Write);
}

#[tokio::test]
async fn the_role_caps_the_access_and_admins_manage_everything() {
    let app = TestApp::with_owner().await;
    let private = collection(&app, "Private").await;
    set_default(&app, private.id, "none").await;
    let doc = item(&app, private.id, None, "doc", "Inside").await;
    let viewer = app.insert_user(Role::Viewer).await;
    let admin = app.insert_user(Role::Admin).await;
    let (viewer_c, admin_c) = (
        app.cookie_client(&app.insert_session(&viewer).await),
        app.cookie_client(&app.insert_session(&admin).await),
    );
    // A viewer with `manage` reads, and nothing more.
    missing(
        viewer_c.get(&format!("/api/v1/items/{}", doc.id)).await,
        "no membership",
    );
    set_member(&app, private.id, &viewer, "manage").await;
    let c: Collection = viewer_c
        .get(&format!("/api/v1/collections/{}", private.id))
        .await
        .json();
    assert_eq!(c.my_access, CollectionAccess::Read);
    let res = viewer_c
        .patch(&format!("/api/v1/items/{}", doc.id))
        .json(&json!({"title": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::Forbidden);
    let members: Vec<CollectionMember> = app
        .server
        .get(&format!("/api/v1/collections/{}/members", private.id))
        .await
        .json();
    let row = members
        .iter()
        .find(|m| m.user.id.to_string() == viewer)
        .unwrap();
    assert_eq!(
        (row.access, row.effective_access),
        (MemberAccess::Manage, CollectionAccess::Read)
    );
    // The owner, the creator, has a `manage` membership; an admin manages it without one.
    assert!(
        members
            .iter()
            .any(|m| m.user.id.to_string() == app.owner() && m.access == MemberAccess::Manage)
    );
    let c: Collection = admin_c
        .get(&format!("/api/v1/collections/{}", private.id))
        .await
        .json();
    assert_eq!(c.my_access, CollectionAccess::Manage);
    admin_c
        .patch(&format!("/api/v1/collections/{}", private.id))
        .json(&json!({"default_access": "read"}))
        .await
        .assert_status_ok();
    // `read` by default: a member without a membership reads and cannot write.
    let member = app.insert_user(Role::Member).await;
    let member_c = app.cookie_client(&app.insert_session(&member).await);
    member_c
        .get(&format!("/api/v1/items/{}", doc.id))
        .await
        .assert_status_ok();
    let res = member_c
        .patch(&format!("/api/v1/items/{}", doc.id))
        .json(&json!({"title": "x"}))
        .await;
    assert_error(&res, 403, ErrorCode::Forbidden);
    // A disabled account has no access and cannot be added; its membership stays, shown without access.
    app.server
        .patch(&format!("/api/v1/users/{viewer}"))
        .json(&json!({"disabled": true}))
        .await
        .assert_status_ok();
    let members: Vec<CollectionMember> = app
        .server
        .get(&format!("/api/v1/collections/{}/members", private.id))
        .await
        .json();
    let row = members
        .iter()
        .find(|m| m.user.id.to_string() == viewer)
        .unwrap();
    assert!(row.disabled && row.effective_access == CollectionAccess::None);
    let res = app
        .server
        .put(&format!(
            "/api/v1/collections/{}/members/{viewer}",
            private.id
        ))
        .json(&json!({"access": "read"}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
    let res = app
        .server
        .put(&format!(
            "/api/v1/collections/{}/members/{}",
            private.id,
            uuid::Uuid::now_v7()
        ))
        .json(&json!({"access": "read"}))
        .await;
    assert_error(&res, 404, ErrorCode::NotFound);
    let res = app
        .server
        .put(&format!(
            "/api/v1/collections/{}/members/{member}",
            private.id
        ))
        .json(&json!({"access": "none"}))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn a_member_manages_the_collection_it_creates() {
    let app = TestApp::with_owner().await;
    let member = app.insert_user(Role::Member).await;
    let other = app.insert_user(Role::Viewer).await;
    let m = app.cookie_client(&app.insert_session(&member).await);
    let o = app.cookie_client(&app.insert_session(&other).await);
    let res = m
        .post("/api/v1/collections")
        .json(&json!({"name": "Mine", "default_access": "none"}))
        .await;
    res.assert_status(StatusCode::CREATED);
    let mine: Collection = res.json();
    assert_eq!(
        (mine.default_access, mine.my_access),
        (DefaultAccess::None, CollectionAccess::Manage)
    );
    // Others do not see it; the owner does.
    let sidebar: Sidebar = o.get("/api/v1/sidebar").await.json();
    assert!(sidebar.collections.iter().all(|c| c.id != mine.id));
    let owners: Vec<Collection> = app.server.get("/api/v1/collections").await.json();
    assert!(owners.iter().any(|c| c.id == mine.id));
    // The directory names everyone who can sign in, without emails.
    let res = m.get("/api/v1/directory").await;
    res.assert_status_ok();
    assert!(!res.text().contains('@'), "no email: {}", res.text());
    let people: Vec<DirectoryUser> = res.json();
    assert_eq!(people.len(), 3);
    // It adds a viewer, who then sees it, read-only.
    let res = m
        .put(&format!("/api/v1/collections/{}/members/{other}", mine.id))
        .json(&json!({"access": "write"}))
        .await;
    res.assert_status_ok();
    let added: CollectionMember = res.json();
    assert_eq!(added.effective_access, CollectionAccess::Read);
    let theirs: Collection = o
        .get(&format!("/api/v1/collections/{}", mine.id))
        .await
        .json();
    assert_eq!(theirs.my_access, CollectionAccess::Read);
    m.delete(&format!("/api/v1/collections/{}/members/{other}", mine.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    missing(
        o.get(&format!("/api/v1/collections/{}", mine.id)).await,
        "removed",
    );
    // Moving an item between collections needs `write` on both.
    let open = collection(&app, "Open").await;
    set_default(&app, open.id, "read").await;
    let doc: ItemSummary = m
        .post("/api/v1/items")
        .json(&json!({"collection_id": mine.id, "kind": "doc", "title": "x"}))
        .await
        .json();
    let res = m
        .post(&format!("/api/v1/items/{}/move", doc.id))
        .json(&json!({"collection_id": open.id, "parent_id": null}))
        .await;
    assert_error(&res, 403, ErrorCode::Forbidden);
    set_member(&app, open.id, &member, "write").await;
    m.post(&format!("/api/v1/items/{}/move", doc.id))
        .json(&json!({"collection_id": open.id, "parent_id": null}))
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn the_tree_etag_moves_with_the_viewers_access() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let member = app.insert_user(Role::Member).await;
    let m = app.cookie_client(&app.insert_session(&member).await);
    let path = format!("/api/v1/collections/{}/tree", c.id);
    let fetch = |etag: Option<String>| {
        let mut req = m.get(&path);
        if let Some(e) = etag {
            req = req.add_header("if-none-match", e);
        }
        req
    };
    let res = fetch(None).await;
    let etag = res.header("etag").to_str().unwrap().to_owned();
    assert_eq!(
        res.json::<CollectionTree>().collection.my_access,
        CollectionAccess::Write
    );
    fetch(Some(etag.clone()))
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
    let mut etag = etag;
    let expect = |res: TestResponse, access: CollectionAccess, etag: &mut String| {
        res.assert_status_ok();
        let next = res.header("etag").to_str().unwrap().to_owned();
        assert_ne!(next, *etag);
        assert_eq!(res.json::<CollectionTree>().collection.my_access, access);
        *etag = next;
    };
    // A new default access (the collection's tree_version).
    set_default(&app, c.id, "read").await;
    expect(
        fetch(Some(etag.clone())).await,
        CollectionAccess::Read,
        &mut etag,
    );
    // A membership (the member's access epoch).
    set_member(&app, c.id, &member, "manage").await;
    expect(
        fetch(Some(etag.clone())).await,
        CollectionAccess::Manage,
        &mut etag,
    );
    // A role change.
    app.server
        .patch(&format!("/api/v1/users/{member}"))
        .json(&json!({"role": "viewer"}))
        .await
        .assert_status_ok();
    expect(
        fetch(Some(etag.clone())).await,
        CollectionAccess::Read,
        &mut etag,
    );
    // A membership elsewhere moves the epoch too (one extra fetch, never a stale one).
    let other = collection(&app, "Other").await;
    set_member(&app, other.id, &member, "read").await;
    expect(
        fetch(Some(etag.clone())).await,
        CollectionAccess::Read,
        &mut etag,
    );
    // Nothing changed: 304 again.
    fetch(Some(etag.clone()))
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
}
