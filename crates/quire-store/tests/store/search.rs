//! Search tests: FTS sync, the query builder, filters, snippets and title search (owner: W1).

use quire_core::api::{ItemKind, SearchQuery, SearchSort};
use quire_store::{items, search, shared};

use crate::common::{MINUTE, NOW, TestDb, viewer};

/// Raw FTS hits for a MATCH expression (ids, sorted).
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

/// The titles `search::search` finds for `q` (default filters, relevance order).
async fn find(t: &TestDb, owner: &str, q: &str) -> Vec<String> {
    find_with(
        t,
        owner,
        SearchQuery {
            q: q.into(),
            ..Default::default()
        },
    )
    .await
}

async fn find_with(t: &TestDb, owner: &str, q: SearchQuery) -> Vec<String> {
    let v = viewer(owner);
    t.r(move |c| search::search(c, &v, &q))
        .await
        .expect("search")
        .items
        .into_iter()
        .map(|h| h.item.title)
        .collect()
}

async fn integrity(t: &TestDb) {
    t.w(|tx| {
        tx.execute(
            "INSERT INTO item_fts(item_fts) VALUES ('integrity-check')",
            [],
        )?;
        Ok(())
    })
    .await
    .expect("FTS integrity-check");
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
    assert_eq!(hits(&t, "cafe").await, std::slice::from_ref(&id));
    integrity(&t).await;
}

#[tokio::test]
async fn fts_integrity_after_rename_save_purge() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Quokka").await;
    let child = t
        .item(&owner, c.id, Some(doc.id), ItemKind::Board, "Quokka board")
        .await;
    let keep = t.item(&owner, c.id, None, ItemKind::Doc, "Keeper").await;
    integrity(&t).await;
    let (d, o) = (doc.id.to_string(), owner.clone());
    t.w(move |tx| items::rename(tx, &d, "Wombat", &o, NOW))
        .await
        .unwrap();
    integrity(&t).await;
    assert_eq!(find(&t, &owner, "wombat").await, ["Wombat"]);
    assert_eq!(find(&t, &owner, "quokka").await, ["Quokka board"]);
    let d = doc.id.to_string();
    t.w(move |tx| {
        shared::set_body_text(tx, &d, "river stones")?;
        shared::set_body_text(tx, &d, "river stones and quokka tracks")
    })
    .await
    .unwrap();
    integrity(&t).await;
    let mut both = find(&t, &owner, "quokka").await;
    both.sort();
    assert_eq!(both, ["Quokka board", "Wombat"]);
    let (d, o) = (doc.id.to_string(), owner.clone());
    t.w(move |tx| {
        items::trash(tx, &d, &o, NOW)?;
        items::purge(tx, &d)
    })
    .await
    .unwrap();
    integrity(&t).await;
    assert!(find(&t, &owner, "quokka").await.is_empty());
    assert_eq!(
        t.query(|c| c.query_row("SELECT count(*) FROM item_text", [], |r| r.get::<_, i64>(0)))
            .await,
        1
    );
    assert_eq!(find(&t, &owner, "keeper").await, ["Keeper"]);
    let _ = (child, keep);
}

#[tokio::test]
async fn query_builder_treats_fts_syntax_as_literal() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "a or b near x")
        .await;
    t.item(&owner, c.id, None, ItemKind::Doc, "a b x").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "b near").await;
    // No FTS syntax error, and OR / NEAR are words to find, not operators.
    assert_eq!(
        find(&t, &owner, r#"a OR b* NEAR( "x"#).await,
        ["a or b near x"]
    );
    for q in [
        r#"""#,
        r#"("#,
        "NOT",
        "AND OR",
        "title:x",
        "^a",
        "a + b",
        "{a b}",
        "*",
        "NEAR(a b, 2)",
    ] {
        let v = viewer(&owner);
        let q = q.to_owned();
        t.r(move |c| {
            search::search(
                c,
                &v,
                &SearchQuery {
                    q,
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    }
    assert_eq!(find(&t, &owner, "title:x").await, Vec::<String>::new());
}

#[tokio::test]
async fn negative_terms() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "plan quokka")
        .await;
    t.item(&owner, c.id, None, ItemKind::Doc, "plan x").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "planning ahead")
        .await;
    let mut got = find(&t, &owner, "plan -x").await;
    got.sort();
    assert_eq!(
        got,
        ["plan quokka", "planning ahead"],
        "the last word is a prefix"
    );
    assert_eq!(
        find(&t, &owner, r#"plan -"plan quokka" -ahead"#).await,
        ["plan x"]
    );
}

#[tokio::test]
async fn only_negative_terms_return_nothing() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "plan y").await;
    for q in ["-x", "-x -\"y z\"", "", "   ", "-"] {
        let v = viewer(&owner);
        let q2 = q.to_owned();
        let page = t
            .r(move |c| {
                search::search(
                    c,
                    &v,
                    &SearchQuery {
                        q: q2,
                        ..Default::default()
                    },
                )
            })
            .await
            .unwrap();
        assert!(page.items.is_empty() && page.next_cursor.is_none(), "{q:?}");
    }
}

#[tokio::test]
async fn remove_diacritics() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "Menu").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "Smörgåsbord")
        .await;
    let d = doc.id.to_string();
    t.w(move |tx| shared::set_body_text(tx, &d, "the café opens at nine"))
        .await
        .unwrap();
    assert_eq!(find(&t, &owner, "cafe").await, ["Menu"]);
    assert_eq!(find(&t, &owner, "CAFÉ").await, ["Menu"]);
    assert_eq!(find(&t, &owner, "smorgasbord").await, ["Smörgåsbord"]);
}

#[tokio::test]
async fn filters_snippets_and_sorts() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let other = t.collection(&owner, "Other").await;
    let title_hit = t.item(&owner, c.id, None, ItemKind::Doc, "Quokka").await;
    let body_hit = t
        .item(
            &owner,
            c.id,
            Some(title_hit.id),
            ItemKind::Doc,
            "Notes on animals",
        )
        .await;
    let board = t
        .item(&owner, other.id, None, ItemKind::Board, "Quokka board")
        .await;
    let (b, o) = (body_hit.id.to_string(), owner.clone());
    t.w(move |tx| {
        shared::set_body_text(tx, &b, "we saw a quokka near the river")?;
        shared::touch_content(tx, &b, &o, NOW + MINUTE)
    })
    .await
    .unwrap();
    // The title weighs 8×: the title match ranks first.
    let v = viewer(&owner);
    let page = t
        .r(move |c| {
            search::search(
                c,
                &v,
                &SearchQuery {
                    q: "quokka".into(),
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    let titles: Vec<&str> = page.items.iter().map(|h| h.item.title.as_str()).collect();
    assert_eq!(titles.len(), 3);
    assert_ne!(titles[2], "Quokka", "{titles:?}");
    assert!(page.items.windows(2).all(|w| w[0].score >= w[1].score));
    let hit = page
        .items
        .iter()
        .find(|h| h.item.id == body_hit.id)
        .unwrap();
    assert!(
        hit.snippet
            .iter()
            .any(|p| p.hit && p.text.eq_ignore_ascii_case("quokka")),
        "{:?}",
        hit.snippet
    );
    assert!(
        hit.snippet
            .iter()
            .all(|p| !p.text.contains(search::HIT_START))
    );
    assert_eq!(
        hit.path
            .iter()
            .map(|c| c.title.as_str())
            .collect::<Vec<_>>(),
        ["Quokka"]
    );
    // Filters.
    let base = || SearchQuery {
        q: "quokka".into(),
        ..Default::default()
    };
    assert_eq!(
        find_with(
            &t,
            &owner,
            SearchQuery {
                kind: Some(ItemKind::Board),
                ..base()
            }
        )
        .await,
        ["Quokka board"]
    );
    let mut in_c = find_with(
        &t,
        &owner,
        SearchQuery {
            collection_id: Some(c.id),
            ..base()
        },
    )
    .await;
    in_c.sort();
    assert_eq!(in_c, ["Notes on animals", "Quokka"]);
    assert_eq!(
        find_with(
            &t,
            &owner,
            SearchQuery {
                updated_after: Some(quire_core::api::Timestamp(NOW)),
                ..base()
            }
        )
        .await,
        ["Notes on animals"]
    );
    // Newest first with sort=updated.
    let by_date = find_with(
        &t,
        &owner,
        SearchQuery {
            sort: Some(SearchSort::Updated),
            ..base()
        },
    )
    .await;
    assert_eq!(by_date[0], "Notes on animals");
    // include_drafts=false leaves unpublished items out.
    let (bid, o) = (board.id.to_string(), owner.clone());
    t.w(move |tx| items::unpublish(tx, &bid, &o, NOW))
        .await
        .unwrap();
    assert_eq!(
        find(&t, &owner, "board").await,
        ["Quokka board"],
        "drafts are included by default"
    );
    assert!(
        find_with(
            &t,
            &owner,
            SearchQuery {
                q: "board".into(),
                include_drafts: Some(false),
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );
    // Too long a query is a validation error.
    let v = viewer(&owner);
    let e = t
        .r(move |c| {
            search::search(
                c,
                &v,
                &SearchQuery {
                    q: "x".repeat(501),
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap_err();
    assert_eq!(e.details, Some(serde_json::json!({"field": "q"})));
}

#[tokio::test]
async fn title_search_matches_titles_only() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let a = t
        .item(&owner, c.id, None, ItemKind::Doc, "Roadmap 2027")
        .await;
    let b = t
        .item(&owner, c.id, Some(a.id), ItemKind::Board, "Road trip")
        .await;
    let body = t.item(&owner, c.id, None, ItemKind::Doc, "Elsewhere").await;
    let d = body.id.to_string();
    t.w(move |tx| shared::set_body_text(tx, &d, "the road ahead"))
        .await
        .unwrap();
    let titles = |q: &'static str, limit: Option<u32>| {
        let v = viewer(&owner);
        t.r(move |c| search::titles(c, &v, q, limit))
    };
    let got = titles("road", None).await.unwrap();
    let mut names: Vec<&str> = got.iter().map(|h| h.item.title.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        ["Road trip", "Roadmap 2027"],
        "prefix match, titles only"
    );
    let trip = got.iter().find(|h| h.item.id == b.id).unwrap();
    assert_eq!(trip.path.len(), 1);
    assert_eq!(titles("ro", Some(1)).await.unwrap().len(), 1);
    assert!(titles("-road", None).await.unwrap().is_empty());
    // A blank query lists recently updated items.
    assert_eq!(titles(" ", None).await.unwrap().len(), 3);
    assert!(
        titles("road", Some(21))
            .await
            .unwrap_err()
            .is(quire_core::api::ErrorCode::Validation)
    );
}
