//! Tree and collection routes: the tree ETag and 304, the sidebar, children, collection CRUD (owner: W1).

use axum::http::StatusCode;
use serde_json::{Value, json};
use wikanvas_api::{Collection, CollectionTree, ErrorCode, Sidebar};

use crate::common::{TestApp, assert_error};
use crate::items::{collection, item, tree_titles};

#[tokio::test]
async fn etag_304_until_structure_changes() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let path = format!("/api/v1/collections/{}/tree", c.id);
    let res = app.server.get(&path).await;
    res.assert_status_ok();
    let first = res.header("etag").to_str().unwrap().to_owned();
    assert!(
        first.starts_with("\"t") && first.ends_with(".0\""),
        "{first}"
    );
    assert_eq!(res.header("cache-control"), "no-cache");
    let tree: CollectionTree = res.json();
    assert_eq!((tree.collection.id, tree.items.len()), (c.id, 0));
    let not_modified = |etag: String| app.server.get(&path).add_header("if-none-match", etag);
    let res = not_modified(first.clone()).await;
    res.assert_status(StatusCode::NOT_MODIFIED);
    assert_eq!(res.header("etag"), first.as_str());
    assert!(res.text().is_empty());
    not_modified(format!("W/{first}"))
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
    not_modified(format!("\"t0.0\", {first}"))
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
    // Each structural change gives a new tag: create, move, trash, restore, archive, collection edit.
    let a = item(&app, c.id, None, "doc", "a").await;
    let b = item(&app, c.id, None, "doc", "b").await;
    let mut etag = first;
    fn changed(etag: &mut String, res: axum_test::TestResponse) {
        res.assert_status_ok();
        let next = res.header("etag").to_str().unwrap().to_owned();
        let version = |t: &str| {
            t.trim_matches('"')
                .trim_start_matches('t')
                .split('.')
                .next()
                .unwrap()
                .parse::<i64>()
                .unwrap()
        };
        assert!(version(&next) > version(etag), "{next} after {etag}");
        *etag = next;
    }
    let res = not_modified(etag.clone()).await;
    changed(&mut etag, res);
    let steps: Vec<(&str, String, Value)> = vec![
        (
            "move",
            format!("/api/v1/items/{}/move", b.id),
            json!({ "parent_id": a.id }),
        ),
        (
            "archive",
            format!("/api/v1/items/{}/archive", b.id),
            json!({}),
        ),
        (
            "unarchive",
            format!("/api/v1/items/{}/unarchive", b.id),
            json!({}),
        ),
        (
            "patch",
            format!("/api/v1/collections/{}", c.id),
            json!({ "color": "#112233" }),
        ),
        // Under the `updated` order the tree sorts by `updated_at`, which a change of `full_width` alone sets.
        (
            "patch",
            format!("/api/v1/collections/{}", c.id),
            json!({ "child_order": "updated" }),
        ),
        (
            "patch",
            format!("/api/v1/items/{}", a.id),
            json!({ "full_width": true }),
        ),
        (
            "patch",
            format!("/api/v1/collections/{}", c.id),
            json!({ "child_order": "manual" }),
        ),
    ];
    for (what, url, body) in steps {
        let res = if what == "patch" {
            app.server.patch(&url).json(&body).await
        } else {
            app.server.post(&url).json(&body).await
        };
        res.assert_status_ok();
        let res = not_modified(etag.clone()).await;
        changed(&mut etag, res);
    }
    app.server
        .delete(&format!("/api/v1/items/{}", b.id))
        .await
        .assert_status_ok();
    let res = not_modified(etag.clone()).await;
    changed(&mut etag, res);
    app.server
        .post(&format!("/api/v1/items/{}/restore", b.id))
        .await
        .assert_status_ok();
    let res = not_modified(etag.clone()).await;
    changed(&mut etag, res);
    // A read changes nothing.
    app.server
        .get(&format!("/api/v1/items/{}", a.id))
        .await
        .assert_status_ok();
    not_modified(etag.clone())
        .await
        .assert_status(StatusCode::NOT_MODIFIED);
    assert_eq!(tree_titles(&app, c.id).await, ["a", "b"]);
    // A deleted collection has no tree.
    app.server
        .delete(&format!("/api/v1/collections/{}", c.id))
        .await
        .assert_status_ok();
    assert_error(&app.server.get(&path).await, 404, ErrorCode::NotFound);
}

#[tokio::test]
async fn collections_crud_and_sidebar() {
    let app = TestApp::with_owner().await;
    let a = collection(&app, "Alpha").await;
    let b = collection(&app, "Beta").await;
    let list: Vec<Collection> = app.server.get("/api/v1/collections").await.json();
    assert_eq!(
        list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    // Reorder: Beta first.
    let res = app
        .server
        .post(&format!("/api/v1/collections/{}/move", b.id))
        .json(&json!({ "position": { "before_id": a.id } }))
        .await;
    res.assert_status_ok();
    let sidebar: Sidebar = app.server.get("/api/v1/sidebar").await.json();
    assert_eq!(
        sidebar
            .collections
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["Beta", "Alpha"]
    );
    assert_eq!((sidebar.stars.len(), sidebar.drafts_count), (0, 0));
    // Edit: icon set, then cleared with null; bad values are 422 on their field.
    let path = format!("/api/v1/collections/{}", a.id);
    let res = app
        .server
        .patch(&path)
        .json(&json!({ "icon": "rocket", "child_order": "title" }))
        .await;
    let edited: Collection = res.json();
    assert_eq!(
        (edited.icon.as_deref(), edited.child_order.as_str()),
        (Some("rocket"), "title")
    );
    let cleared: Collection = app
        .server
        .patch(&path)
        .json(&json!({ "icon": null }))
        .await
        .json();
    assert_eq!(cleared.icon, None);
    for (body, field) in [
        (json!({ "name": " " }), "name"),
        (json!({ "color": "blue" }), "color"),
        (json!({ "icon": "Not An Icon" }), "icon"),
        (json!({ "description": "d".repeat(501) }), "description"),
    ] {
        let err = assert_error(
            &app.server.patch(&path).json(&body).await,
            422,
            ErrorCode::Validation,
        );
        assert_eq!(err.error.details, Some(json!({ "field": field })));
    }
    let res = app
        .server
        .patch(&path)
        .json(&json!({ "child_order": "random" }))
        .await;
    assert_error(&res, 422, ErrorCode::Validation);
    // Delete with items, then restore.
    let doc = item(&app, a.id, None, "doc", "inside").await;
    item(&app, a.id, Some(doc.id), "board", "deeper").await;
    let res = app.server.delete(&path).await;
    res.assert_json(&json!({ "trashed_items": 2 }));
    assert_error(&app.server.get(&path).await, 404, ErrorCode::NotFound);
    assert_error(
        &app.server.patch(&path).json(&json!({ "name": "x" })).await,
        404,
        ErrorCode::NotFound,
    );
    assert_error(&app.server.delete(&path).await, 404, ErrorCode::NotFound);
    let res = app
        .server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": a.id, "kind": "doc" }))
        .await;
    assert_error(&res, 409, ErrorCode::ParentInvalid);
    assert_error(
        &app.server
            .post(&format!("/api/v1/items/{}/restore", doc.id))
            .await,
        409,
        ErrorCode::ParentInvalid,
    );
    let restored: Collection = app.server.post(&format!("{path}/restore")).await.json();
    assert_eq!(restored.item_count, 2);
    assert_eq!(tree_titles(&app, a.id).await, ["inside", "deeper"]);
    assert_error(
        &app.server.post(&format!("{path}/restore")).await,
        409,
        ErrorCode::Conflict,
    );
    // Children of an item, in the collection's order.
    item(&app, a.id, Some(doc.id), "doc", "Another").await;
    let kids: Vec<Value> = app
        .server
        .get(&format!("/api/v1/items/{}/children", doc.id))
        .await
        .json();
    let titles: Vec<&str> = kids.iter().map(|k| k["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Another", "deeper"], "child_order is title");
}
