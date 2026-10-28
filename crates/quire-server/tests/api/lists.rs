//! List routes: page shape, cursor binding, the trash (owner: W1).

use axum::http::StatusCode;
use quire_api::{ErrorCode, TrashList};
use serde_json::{Value, json};

use crate::common::{TestApp, assert_error};
use crate::items::{collection, item};

/// Every page of `path` (which has no query yet, or ends with one), `limit` at a time; checks the page shape.
async fn pages(app: &TestApp, path: &str, limit: u32) -> Vec<Value> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let mut url = format!("{path}{sep}limit={limit}");
    let mut out = Vec::new();
    for _ in 0..50 {
        let res = app.server.get(&url).await;
        res.assert_status_ok();
        let page: Value = res.json();
        let obj = page.as_object().unwrap();
        assert_eq!(
            obj.keys().map(String::as_str).collect::<Vec<_>>(),
            ["items", "next_cursor"],
            "{path}: {page}"
        );
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= limit as usize);
        out.extend(items.iter().cloned());
        match &page["next_cursor"] {
            Value::Null => return out,
            Value::String(c) => {
                assert!(!items.is_empty(), "{path}: a cursor after an empty page");
                url = format!("{path}{sep}limit={limit}&cursor={c}");
            }
            other => panic!("{path}: next_cursor {other}"),
        }
    }
    panic!("{path}: too many pages");
}

#[tokio::test]
async fn every_paginated_list_has_page_shape() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let a = item(&app, c.id, None, "doc", &format!("quokka {i}")).await;
        ids.push(a.id);
        app.server
            .post(&format!("/api/v1/items/{}/view", a.id))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        app.server
            .put(&format!("/api/v1/items/{}/star", a.id))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        app.server
            .post(&format!("/api/v1/items/{}/unpublish", a.id))
            .await
            .assert_status_ok();
    }
    for i in 0..3 {
        let a = item(&app, c.id, None, "board", &format!("archived quokka {i}")).await;
        app.server
            .post(&format!("/api/v1/items/{}/archive", a.id))
            .await
            .assert_status_ok();
    }
    let expect = [
        ("/api/v1/recent".to_owned(), 3),
        ("/api/v1/updated".to_owned(), 3),
        (format!("/api/v1/collections/{}/updated", c.id), 3),
        ("/api/v1/drafts".to_owned(), 3),
        ("/api/v1/starred".to_owned(), 3),
        ("/api/v1/archive".to_owned(), 3),
        ("/api/v1/search?q=quokka".to_owned(), 3),
        (
            "/api/v1/search?q=quokka&include_archived=true&sort=updated".to_owned(),
            6,
        ),
    ];
    for (path, n) in &expect {
        for limit in [1, 2, 100] {
            let all = pages(&app, path, limit).await;
            assert_eq!(all.len(), *n, "{path} limit {limit}");
            let mut seen: Vec<&str> = all
                .iter()
                .map(|i| i["id"].as_str().or(i["item"]["id"].as_str()).unwrap())
                .collect();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), *n, "{path} limit {limit}: no repeats");
        }
    }
    // Listed items carry the list's own field.
    let starred: Value = app.server.get("/api/v1/starred").await.json();
    assert!(starred["items"][0]["starred_at"].is_string());
    let recent: Value = app.server.get("/api/v1/recent").await.json();
    assert!(
        recent["items"][0]["last_viewed_at"].is_string() && recent["items"][0]["path"].is_array()
    );
    // An empty list is one page with a null cursor.
    let other = collection(&app, "Empty").await;
    let res = app
        .server
        .get(&format!("/api/v1/collections/{}/updated", other.id))
        .await;
    res.assert_json(&json!({ "items": [], "next_cursor": null }));
    // A limit out of range is 422 on `limit`.
    for bad in ["0", "101", "x"] {
        let res = app
            .server
            .get(&format!("/api/v1/updated?limit={bad}"))
            .await;
        let body = assert_error(&res, 422, ErrorCode::Validation);
        let field = body.error.details.unwrap()["field"].clone();
        assert!(field == "limit" || field == "query", "{bad}: {field}");
    }
}

#[tokio::test]
async fn cursor_from_other_query_422() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let other = collection(&app, "Other").await;
    for i in 0..3 {
        let a = item(&app, c.id, None, "doc", &format!("alpha {i}")).await;
        app.server
            .post(&format!("/api/v1/items/{}/view", a.id))
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }
    let cursor_of = |v: Value| v["next_cursor"].as_str().unwrap().to_owned();
    let recent = cursor_of(app.server.get("/api/v1/recent?limit=1").await.json());
    let updated = cursor_of(
        app.server
            .get(&format!("/api/v1/collections/{}/updated?limit=1", c.id))
            .await
            .json(),
    );
    let search = cursor_of(
        app.server
            .get("/api/v1/search?q=alpha&limit=1")
            .await
            .json(),
    );
    for url in [
        format!("/api/v1/updated?cursor={recent}"),
        format!("/api/v1/drafts?cursor={recent}"),
        format!("/api/v1/collections/{}/updated?cursor={updated}", other.id),
        format!("/api/v1/updated?cursor={updated}"),
        format!("/api/v1/search?q=beta&cursor={search}"),
        format!("/api/v1/search?q=alpha&sort=updated&cursor={search}"),
        format!("/api/v1/search?q=alpha&include_archived=true&cursor={search}"),
        format!("/api/v1/recent?cursor={search}"),
        "/api/v1/recent?cursor=not-a-cursor".to_owned(),
        format!("/api/v1/recent?cursor={}A", &recent[..recent.len() - 1]),
    ] {
        let body = assert_error(&app.server.get(&url).await, 422, ErrorCode::Validation);
        assert_eq!(
            body.error.details,
            Some(json!({ "field": "cursor" })),
            "{url}"
        );
    }
    // The same cursor with its own query, and another page size, works.
    app.server
        .get(&format!("/api/v1/recent?cursor={recent}&limit=50"))
        .await
        .assert_status_ok();
    app.server
        .get(&format!("/api/v1/search?q=alpha&cursor={search}"))
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn trash_lists_roots_with_purge_at() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let gone = collection(&app, "Gone").await;
    let root = item(&app, c.id, None, "doc", "root").await;
    let child = item(&app, c.id, Some(root.id), "doc", "child").await;
    let grandchild = item(&app, c.id, Some(child.id), "board", "grandchild").await;
    let earlier = item(&app, c.id, Some(root.id), "doc", "earlier").await;
    item(&app, gone.id, None, "doc", "in gone").await;
    app.server
        .delete(&format!("/api/v1/items/{}", earlier.id))
        .await
        .assert_json(&json!({ "trashed": 1 }));
    app.server
        .delete(&format!("/api/v1/items/{}", root.id))
        .await
        .assert_json(&json!({ "trashed": 3 }));
    app.server
        .delete(&format!("/api/v1/collections/{}", gone.id))
        .await
        .assert_json(&json!({ "trashed_items": 1 }));
    let res = app.server.get("/api/v1/trash").await;
    res.assert_status_ok();
    let raw: Value = res.json();
    let trash: TrashList = serde_json::from_value(raw.clone()).unwrap();
    const THIRTY_DAYS: i64 = 30 * 86_400_000;
    assert_eq!(trash.collections.len(), 1);
    let tc = &trash.collections[0];
    assert_eq!((tc.collection.id, tc.collection.item_count), (gone.id, 1));
    assert_eq!(tc.purge_at.millis() - tc.deleted_at.millis(), THIRTY_DAYS);
    assert_eq!(
        raw["collections"][0]["name"], "Gone",
        "Collection fields are flattened: {raw}"
    );
    // Roots only: root (2 nested) and earlier (its own root, with its path); not child or grandchild.
    let mut roots: Vec<(String, u64, usize)> = trash
        .items
        .iter()
        .map(|i| (i.item.title.clone(), i.descendant_count, i.path.len()))
        .collect();
    roots.sort();
    assert_eq!(
        roots,
        [("earlier".to_owned(), 0, 1), ("root".to_owned(), 2, 0)]
    );
    for i in &trash.items {
        assert_eq!(i.purge_at.millis() - i.deleted_at.millis(), THIRTY_DAYS);
    }
    assert!(
        raw["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["title"] == "root"),
        "ItemSummary is flattened"
    );
    // The item detail of a trashed descendant shows the purge date too.
    let detail: Value = app
        .server
        .get(&format!("/api/v1/items/{}", grandchild.id))
        .await
        .json();
    assert!(detail["trashed_at"].is_string() && detail["purge_at"].is_string());
    // Restoring the root brings back its unit; `earlier` stays.
    app.server
        .post(&format!("/api/v1/items/{}/restore", child.id))
        .await
        .assert_status_ok();
    let trash: TrashList = app.server.get("/api/v1/trash").await.json();
    assert_eq!(
        trash
            .items
            .iter()
            .map(|i| i.item.title.as_str())
            .collect::<Vec<_>>(),
        ["earlier"]
    );
}
