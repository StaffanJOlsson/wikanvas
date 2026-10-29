//! Search routes: snippets, filters, the offset cap, title search (owner: W1).

use quire_api::{ErrorCode, Page, SearchHit, TitleHit};
use serde_json::Value;

use crate::common::{TestApp, assert_error};
use crate::items::{collection, item};

async fn search(app: &TestApp, query: &str) -> Page<SearchHit> {
    let res = app.server.get(&format!("/api/v1/search?{query}")).await;
    res.assert_status_ok();
    res.json()
}

#[tokio::test]
async fn snippet_markers_and_draft_filter() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Notes").await;
    let doc = item(&app, c.id, None, "doc", "Field notes").await;
    let draft = item(&app, c.id, Some(doc.id), "doc", "Quokka draft").await;
    let id = doc.id.to_string();
    app.state
        .db
        .write(move |tx| {
            quire_store::shared::set_body_text(tx, &id, "Early morning: a quokka crossed the path.")
        })
        .await
        .unwrap();
    app.server
        .post(&format!("/api/v1/items/{}/unpublish", draft.id))
        .await
        .assert_status_ok();
    let page = search(&app, "q=quokka").await;
    assert_eq!(page.items.len(), 2, "drafts are included by default");
    let hit = page.items.iter().find(|h| h.item.id == doc.id).unwrap();
    let hits: Vec<&str> = hit
        .snippet
        .iter()
        .filter(|p| p.hit)
        .map(|p| p.text.as_str())
        .collect();
    assert_eq!(hits, ["quokka"]);
    let text: String = hit.snippet.iter().map(|p| p.text.as_str()).collect();
    assert!(text.contains("Early morning: a quokka crossed"), "{text}");
    assert!(
        !text.contains('\u{E000}') && !text.contains('\u{E001}'),
        "markers never reach the client"
    );
    let raw: Value = app.server.get("/api/v1/search?q=quokka").await.json();
    let parts = raw["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|i| i["snippet"].as_array().unwrap().clone());
    assert!(
        parts
            .into_iter()
            .all(|p| p["hit"].is_boolean() && p["text"].is_string()),
        "{raw}"
    );
    let draft_hit = page.items.iter().find(|h| h.item.id == draft.id).unwrap();
    assert_eq!(
        draft_hit.path.iter().map(|c| c.id).collect::<Vec<_>>(),
        [doc.id]
    );
    let page = search(&app, "q=quokka&include_drafts=false").await;
    assert_eq!(
        page.items.iter().map(|h| h.item.id).collect::<Vec<_>>(),
        [doc.id]
    );
    // Kind and collection filters, `q` over 500 characters.
    assert!(search(&app, "q=quokka&kind=board").await.items.is_empty());
    assert_eq!(
        search(&app, &format!("q=quokka&collection_id={}", c.id))
            .await
            .items
            .len(),
        2
    );
    let res = app
        .server
        .get(&format!("/api/v1/search?q={}", "x".repeat(501)))
        .await;
    let body = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(
        body.error.details,
        Some(serde_json::json!({ "field": "q" }))
    );
    // FTS syntax is literal; only negative terms find nothing.
    search(&app, "q=a%20OR%20b*%20NEAR(%20%22x").await;
    assert!(search(&app, "q=-quokka").await.items.is_empty());
    // Title search.
    let res = app.server.get("/api/v1/search/titles?q=quok").await;
    let titles: Vec<TitleHit> = res.json();
    assert_eq!(
        titles.iter().map(|h| h.item.id).collect::<Vec<_>>(),
        [draft.id],
        "titles only"
    );
    assert_eq!(titles[0].path[0].id, doc.id);
    let res = app.server.get("/api/v1/search/titles?q=x&limit=21").await;
    assert_error(&res, 422, ErrorCode::Validation);
}

#[tokio::test]
async fn relevance_offset_capped_at_1000() {
    let app = TestApp::with_owner().await;
    let c = collection(&app, "Many").await;
    let cid = c.id.to_string();
    app.state
        .db
        .write(move |tx| {
            let mut stmt = tx.prepare(
                "INSERT INTO items (id, url_id, collection_id, kind, title, sort_key, created_at, updated_at,
                   published_at)
                 VALUES (?1, ?2, ?3, 'doc', ?4, 'a0', 1, ?5, 1)",
            )?;
            for i in 0..1050 {
                let id = uuid::Uuid::now_v7().to_string();
                stmt.execute(rusqlite::params![id, format!("m{i:09}"), cid, format!("quokka note {i}"), i])?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut url = "/api/v1/search?q=quokka&limit=100".to_owned();
    let mut pages = 0;
    loop {
        let page = search(&app, url.trim_start_matches("/api/v1/search?")).await;
        pages += 1;
        for h in &page.items {
            assert!(seen.insert(h.item.id), "repeated {}", h.item.id);
        }
        match page.next_cursor {
            Some(c) => url = format!("/api/v1/search?q=quokka&limit=100&cursor={c}"),
            None => break,
        }
        assert!(pages <= 11);
    }
    assert_eq!(
        (seen.len(), pages),
        (1000, 10),
        "relevance pages stop at 1,000 results"
    );
    // A limit that straddles the cap is cut at it.
    let mut url = "q=quokka&limit=99".to_owned();
    let mut total = 0;
    loop {
        let page = search(&app, &url).await;
        total += page.items.len();
        match page.next_cursor {
            Some(c) => url = format!("q=quokka&limit=99&cursor={c}"),
            None => break,
        }
    }
    assert_eq!(total, 1000);
    // Sorted by date there is no cap.
    let mut url = "q=quokka&limit=100&sort=updated".to_owned();
    let mut total = 0;
    loop {
        let page = search(&app, &url).await;
        total += page.items.len();
        match page.next_cursor {
            Some(c) => url = format!("q=quokka&limit=100&sort=updated&cursor={c}"),
            None => break,
        }
    }
    assert_eq!(total, 1050);
}
