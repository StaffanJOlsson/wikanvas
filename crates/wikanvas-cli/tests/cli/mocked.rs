//! Multi-request command logic against the mock listener, so it is checked before the real routes exist: the
//! `item create --md` sequence, `--attach-images`, and reference resolution (owner: W8)

use std::os::unix::fs::symlink;

use serde_json::{Value, json};

use crate::common::{ID, Mock, Recorded, Reply, Sandbox, fake_token, last_error, run_json};

const NEW: &str = "0192f3a4-0000-7000-8000-000000000001";

fn summary(title: &str) -> Value {
    json!({"id": NEW, "url_id": "NewItem001", "title": title, "kind": "doc"})
}

fn put_result() -> Value {
    json!({"seq": 1, "etag": "\"d1\"", "updated_at": "t", "changed": true, "merged": false, "warnings": []})
}

fn calls(mock: &Mock) -> Vec<String> {
    mock.requests()
        .iter()
        .map(|r| format!("{} {}", r.method, r.target))
        .collect()
}

#[test]
fn item_create_md_puts_then_patches_title() {
    let mock = Mock::start(|r: &Recorded, _| match (r.method.as_str(), r.path()) {
        ("POST", "/api/v1/items") => Reply::json(201, &summary("")),
        ("PUT", _) => Reply::json(200, &put_result()),
        ("PATCH", _) => Reply::json(200, &summary("T")),
        _ => Reply::error(404, "not_found"),
    });
    let s = Sandbox::new();
    let (code, v, err) = run_json(
        s.wikanvas_at(&mock.url, &fake_token(1))
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                ID,
                "--kind",
                "doc",
                "--title",
                "T",
                "--md",
                "-",
            ])
            .write_stdin("Body\n"),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v, json!({"item": summary("T"), "doc": put_result()}));
    assert_eq!(
        calls(&mock),
        [
            "POST /api/v1/items".to_owned(),
            format!("PUT /api/v1/items/{NEW}/doc/markdown"),
            format!("PATCH /api/v1/items/{NEW}"),
        ]
    );
    let reqs = mock.requests();
    let created: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        created,
        json!({"collection_id": ID, "kind": "doc"}),
        "no title before the PUT"
    );
    assert_eq!(reqs[1].header("if-match"), Some("\"d0\""));
    assert_eq!(reqs[1].body, b"Body\n");
    assert_eq!(
        serde_json::from_slice::<Value>(&reqs[2].body).unwrap(),
        json!({"title": "T"})
    );
}

#[test]
fn item_create_md_discards_on_put_failure() {
    let mock = Mock::start(|r: &Recorded, _| match (r.method.as_str(), r.path()) {
        ("POST", "/api/v1/items") => Reply::json(201, &summary("")),
        ("PUT", _) => Reply::error(413, "payload_too_large"),
        ("DELETE", _) => Reply::json(200, &json!({"trashed": 1})),
        _ => Reply::error(404, "not_found"),
    });
    let s = Sandbox::new();
    let (code, v, err) = run_json(
        s.wikanvas_at(&mock.url, &fake_token(1))
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                ID,
                "--kind",
                "doc",
                "--title",
                "T",
                "--md",
                "-",
            ])
            .write_stdin("Body\n"),
    );
    assert_eq!(code, 10, "{err}");
    assert_eq!(v, Value::Null);
    assert_eq!(
        last_error(&err)["error"]["code"],
        "payload_too_large",
        "the PUT's error"
    );
    assert_eq!(
        calls(&mock),
        [
            "POST /api/v1/items".to_owned(),
            format!("PUT /api/v1/items/{NEW}/doc/markdown"),
            format!("DELETE /api/v1/items/{NEW}?discard_empty=true"),
        ],
        "discarded, and never titled"
    );
    // A failed discard is a warning with the id; the PUT's error stays the result.
    let stuck = Mock::start(|r: &Recorded, _| match r.method.as_str() {
        "POST" => Reply::json(201, &summary("")),
        "PUT" => Reply::error(422, "validation"),
        _ => Reply::error(409, "not_empty"),
    });
    let (code, _, err) = run_json(
        s.wikanvas_at(&stuck.url, &fake_token(1))
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                ID,
                "--kind",
                "doc",
                "--md",
                "-",
            ])
            .write_stdin("x"),
    );
    assert_eq!(code, 10);
    let first: Value = serde_json::from_str(err.lines().next().unwrap()).unwrap();
    assert_eq!(first["warning"]["code"], "discard_failed");
    assert_eq!(first["warning"]["details"]["id"], NEW);
    assert_eq!(last_error(&err)["error"]["code"], "validation");
    // A board cannot take --md; an unreadable file creates nothing.
    let none = Mock::always(Reply::json(201, &summary("")));
    for args in [
        &["--kind", "board", "--md", "-"][..],
        &["--kind", "doc", "--md", "/no/such/file.md"][..],
    ] {
        let (code, _, _) = run_json(
            s.wikanvas_at(&none.url, &fake_token(1))
                .args(["--json", "item", "create", "--collection", ID])
                .args(args),
        );
        assert_eq!(code, 2, "{args:?}");
    }
    assert_eq!(none.connections(), 0);
}

/// The title PATCH after a good PUT fails: the item exists and holds the content, so it is kept (no discard), the
/// command exits with the PATCH's error, and an `item_created` warning names the item (D-W8-4).
#[test]
fn item_create_md_title_failure_keeps_item_and_warns() {
    let mock = Mock::start(|r: &Recorded, _| match (r.method.as_str(), r.path()) {
        ("POST", "/api/v1/items") => Reply::json(201, &summary("")),
        ("PUT", _) => Reply::json(200, &put_result()),
        ("PATCH", _) => Reply::error(422, "validation"),
        _ => Reply::error(404, "not_found"),
    });
    let s = Sandbox::new();
    let (code, v, err) = run_json(
        s.wikanvas_at(&mock.url, &fake_token(1))
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                ID,
                "--kind",
                "doc",
            ])
            .args(["--title", "T", "--md", "-"])
            .write_stdin("Body\n"),
    );
    assert_eq!(code, 10, "{err}");
    assert_eq!(v, Value::Null);
    let lines: Vec<Value> = err
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(lines.len(), 2, "{err}");
    assert_eq!(lines[0]["warning"]["code"], "item_created");
    assert_eq!(lines[0]["warning"]["details"]["id"], NEW);
    assert_eq!(lines[1]["error"]["code"], "validation");
    assert_eq!(lines[1]["exit_code"], 10);
    assert_eq!(
        calls(&mock),
        [
            "POST /api/v1/items".to_owned(),
            format!("PUT /api/v1/items/{NEW}/doc/markdown"),
            format!("PATCH /api/v1/items/{NEW}"),
        ],
        "never discarded"
    );
}

#[test]
fn attach_images_uploads_only_contained_images() {
    let mock = Mock::start(|r: &Recorded, i| match (r.method.as_str(), r.path()) {
        ("GET", "/api/v1/config") => Reply::error(404, "not_found"),
        ("POST", "/api/v1/attachments") => Reply::json(
            201,
            &json!({"id": format!("att{i}"), "url": format!("/api/v1/attachments/att{i}")}),
        ),
        ("PUT", _) => Reply::json(200, &put_result()),
        _ => Reply::error(404, "not_found"),
    });
    let s = Sandbox::new();
    let png = b"\x89PNG\r\n\x1a\nrest";
    s.file("docs/img/ok.png", png, 0o644);
    s.file("docs/fake.png", b"text", 0o644);
    let outside = s.file("outside.png", png, 0o644);
    symlink(&outside, s.path().join("docs/link.png")).unwrap();
    let md = format!(
        "![a](img/ok.png) ![again](img/ok.png)\n![b](../outside.png)\n![c]({})\n![d](link.png)\n![e](fake.png)\n\
         ![f](https://example.org/x.png) ![g](/api/v1/attachments/old)\n```\n![h](img/ok.png)\n```\n",
        outside.display()
    );
    let page = s.file("docs/page.md", md.as_bytes(), 0o644);
    let (code, _, err) = run_json(s.wikanvas_at(&mock.url, &fake_token(1)).args([
        "--json",
        "doc",
        "put",
        ID,
        "--md",
        page.to_str().unwrap(),
        "--force",
        "--attach-images",
    ]));
    assert_eq!(code, 0, "{err}");
    let warnings: Vec<Value> = err
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let skipped: Vec<&str> = warnings
        .iter()
        .map(|w| {
            assert_eq!(w["warning"]["code"], "attach_skipped", "{w}");
            w["warning"]["details"]["path"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        skipped,
        [
            "../outside.png",
            outside.to_str().unwrap(),
            "link.png",
            "fake.png"
        ]
    );
    let reqs = mock.requests();
    let uploads: Vec<&Recorded> = reqs
        .iter()
        .filter(|r| r.path() == "/api/v1/attachments")
        .collect();
    assert_eq!(uploads.len(), 1, "one upload for the same file twice");
    assert_eq!(uploads[0].body, png);
    assert_eq!(
        uploads[0].header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(uploads[0].query("item_id"), Some(ID));
    assert_eq!(uploads[0].query("filename"), Some("ok.png"));
    let put = reqs.iter().find(|r| r.method == "PUT").unwrap();
    let body = String::from_utf8(put.body.clone()).unwrap();
    let url = format!(
        "/api/v1/attachments/att{}",
        reqs.iter()
            .position(|r| r.path() == "/api/v1/attachments")
            .unwrap()
    );
    assert!(
        body.starts_with(&format!("![a]({url}) ![again]({url})\n")),
        "{body}"
    );
    assert!(
        body.contains("![b](../outside.png)") && body.contains("![e](fake.png)"),
        "{body}"
    );
    assert!(
        body.contains("![f](https://example.org/x.png) ![g](/api/v1/attachments/old)"),
        "{body}"
    );
    assert!(
        body.contains("```\n![h](img/ok.png)\n```"),
        "code blocks stay: {body}"
    );
    // --base-dir moves the containment root.
    let (code, _, err) = run_json(
        s.wikanvas_at(&mock.url, &fake_token(1))
            .args([
                "--json",
                "doc",
                "put",
                ID,
                "--md",
                "-",
                "--force",
                "--attach-images",
                "--base-dir",
            ])
            .arg(s.path().join("docs"))
            .write_stdin("![x](img/ok.png)\n"),
    );
    assert_eq!((code, err.as_str()), (0, ""));
}

#[test]
fn url_ids_and_urls_resolve_to_uuids() {
    let mock = Mock::start(|r: &Recorded, _| match r.path() {
        "/api/v1/items/AbCdEfGhIj" => Reply::json(200, &json!({"item": {"id": ID}})),
        "/api/v1/collections/CoLlEcTiOn" => Reply::json(200, &json!({"id": NEW})),
        _ => Reply::json(200, &json!({"ok": true})),
    });
    let s = Sandbox::new();
    for (args, last) in [
        (
            &["item", "text", "AbCdEfGhIj"][..],
            format!("GET /api/v1/items/{ID}/text"),
        ),
        (
            &[
                "item",
                "text",
                "https://wiki.example.org/doc/my-page-AbCdEfGhIj",
            ][..],
            format!("GET /api/v1/items/{ID}/text"),
        ),
        (
            &["tree", "CoLlEcTiOn"][..],
            format!("GET /api/v1/collections/{NEW}/tree"),
        ),
        (
            &["item", "get", "AbCdEfGhIj"][..],
            "GET /api/v1/items/AbCdEfGhIj".to_owned(),
        ),
    ] {
        let (code, _, err) = run_json(
            s.wikanvas_at(&mock.url, &fake_token(1))
                .arg("--json")
                .args(args),
        );
        assert_eq!(code, 0, "{args:?}: {err}");
        assert_eq!(calls(&mock).last().unwrap(), &last, "{args:?}");
    }
    let (code, _, _) = run_json(s.wikanvas_at(&mock.url, &fake_token(1)).args([
        "--json",
        "item",
        "text",
        "not-a-ref",
    ]));
    assert_eq!(code, 2);
}

/// `item move` always sends `parent_id` (§7.2, W1 D23). Without `--parent` or `--top` (a reorder) it is the item's
/// current parent, read first; `--top` sends `null`; `--collection` needs one of the two.
#[test]
fn item_move_without_parent_keeps_the_current_parent() {
    const PARENT: &str = "0192f3a4-0000-7000-8000-0000000000aa";
    const SIBLING: &str = "0192f3a4-0000-7000-8000-0000000000bb";
    let mock = Mock::start(|r: &Recorded, _| match (r.method.as_str(), r.path()) {
        ("GET", p) if p == format!("/api/v1/items/{ID}") => Reply::json(
            200,
            &json!({"item": {"id": ID, "url_id": "Item000001", "parent_id": PARENT, "kind": "doc"}}),
        ),
        ("POST", p) if p == format!("/api/v1/items/{ID}/move") => {
            Reply::json(200, &json!({"id": ID, "parent_id": PARENT}))
        }
        _ => Reply::error(404, "not_found"),
    });
    let s = Sandbox::new();
    let run = |extra: &[&str]| {
        let mut args = vec!["--json", "item", "move", ID];
        args.extend_from_slice(extra);
        run_json(s.wikanvas_at(&mock.url, &fake_token(1)).args(args))
    };
    let (code, _, err) = run(&["--after", SIBLING]);
    assert_eq!(code, 0, "{err}");
    let bodies = |mock: &Mock| -> Vec<Value> {
        mock.requests()
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    };
    assert_eq!(
        calls(&mock),
        [
            format!("GET /api/v1/items/{ID}"),
            format!("POST /api/v1/items/{ID}/move"),
        ]
    );
    assert_eq!(
        bodies(&mock),
        [json!({"parent_id": PARENT, "position": {"after_id": SIBLING}})]
    );
    // --top: null, without a GET.
    let (code, _, err) = run(&["--top"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(bodies(&mock)[1], json!({"parent_id": null}));
    assert_eq!(calls(&mock).len(), 3);
    // --collection alone is a usage error, and nothing is sent.
    let (code, _, err) = run(&["--collection", ID]);
    assert_eq!(code, 2, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "cli_usage");
    assert_eq!(calls(&mock).len(), 3);
}
