//! End-to-end command flows against a TestServer (owner: W8). They need the W1, W2, W4 and W6 routes, so they pass
//! once those workstreams have merged; W7 runs them in its gate (PLAN W8, R7).

use std::os::unix::fs::symlink;

use quire_api::Scope;
use serde_json::{Value, json};

use crate::common::{Harness, last_error, run_json};

/// Runs `quire --json …` as `scope` and expects success; returns stdout.
fn ok(h: &Harness, scope: Scope, args: &[&str]) -> Value {
    let (code, v, err) = h.json(scope, args);
    assert_eq!(code, 0, "quire {}: {err}", args.join(" "));
    v
}

fn collection(h: &Harness, name: &str) -> String {
    ok(h, Scope::Write, &["collection", "create", "--name", name])["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn doc(h: &Harness, c: &str, title: &str, parent: Option<&str>) -> String {
    let mut args = vec![
        "item",
        "create",
        "--collection",
        c,
        "--kind",
        "doc",
        "--title",
        title,
    ];
    if let Some(p) = parent {
        args.extend(["--parent", p]);
    }
    ok(h, Scope::Write, &args)["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn md_doc(h: &Harness, c: &str, md: &str) -> (String, String) {
    let (code, v, err) = run_json(
        h.quire()
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                c,
                "--kind",
                "doc",
                "--md",
                "-",
            ])
            .write_stdin(md.to_owned()),
    );
    assert_eq!(code, 0, "{err}");
    (
        v["item"]["id"].as_str().unwrap().to_owned(),
        v["doc"]["etag"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn collection_tree_move_and_cycle_exit_8() {
    let h = Harness::start();
    let c = collection(&h, "Tree");
    let a = doc(&h, &c, "A", None);
    let b = doc(&h, &c, "B", Some(&a));
    let cc = doc(&h, &c, "C", Some(&b));
    let tree = ok(&h, Scope::Read, &["tree", &c]);
    let items = tree["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let parent = |id: &str| items.iter().find(|i| i["id"] == id).unwrap()["parent_id"].clone();
    assert_eq!(parent(&a), Value::Null);
    assert_eq!(parent(&b), json!(a));
    assert_eq!(parent(&cc), json!(b));
    let moved = ok(&h, Scope::Write, &["item", "move", &b, "--top"]);
    assert_eq!(moved["parent_id"], Value::Null);
    ok(&h, Scope::Write, &["item", "move", &b, "--parent", &a]);
    let (code, _, err) = h.json(Scope::Write, &["item", "move", &a, "--parent", &cc]);
    assert_eq!(code, 8, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "cycle");
    let dup = ok(&h, Scope::Write, &["item", "duplicate", &cc]);
    assert_ne!(dup["id"], json!(cc));
    // A url_id works as a reference too.
    let url_id = ok(&h, Scope::Read, &["item", "get", &a])["item"]["url_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        ok(&h, Scope::Read, &["item", "text", &url_id])["title"],
        "A"
    );
}

#[test]
fn doc_markdown_roundtrip_and_stale_etag_exit_9() {
    let h = Harness::start();
    let c = collection(&h, "Docs");
    let (id, etag) = md_doc(&h, &c, "Hello **world**.\n");
    let got = ok(&h, Scope::Read, &["doc", "get", &id, "--format", "md"]);
    assert_eq!(got["content"], "Hello **world**.\n");
    assert_eq!(got["etag"], etag.as_str());
    let f = h.sandbox.file("v2.md", b"Second version.\n", 0o600);
    let put = ok(
        &h,
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            f.to_str().unwrap(),
            "--if-match",
            &etag,
        ],
    );
    assert_eq!(put["changed"], true);
    // A stale ETag with a body that differs from the current state: 412. Re-sending the current content would be
    // 200 `changed: false` (ARCHITECTURE D50).
    let f3 = h.sandbox.file("v3.md", b"Third version.\n", 0o600);
    let (code, _, err) = h.json(
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            f3.to_str().unwrap(),
            "--if-match",
            &etag,
        ],
    );
    assert_eq!(code, 9, "a stale ETag: {err}");
    assert_eq!(last_error(&err)["error"]["code"], "rev_mismatch");
    let json = ok(&h, Scope::Read, &["doc", "get", &id, "--format", "json"]);
    assert_eq!(json["content"]["schema_version"], 1);
}

#[test]
fn doc_put_merge_after_concurrent_update() {
    let h = Harness::start();
    let c = collection(&h, "Merge");
    let (id, base) = md_doc(&h, &c, "First paragraph.\n\nSecond paragraph.\n");
    let a = h.sandbox.file(
        "a.md",
        b"First paragraph, edited by A.\n\nSecond paragraph.\n",
        0o600,
    );
    let b = h.sandbox.file(
        "b.md",
        b"First paragraph.\n\nSecond paragraph, edited by B.\n",
        0o600,
    );
    ok(
        &h,
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            a.to_str().unwrap(),
            "--if-match",
            &base,
        ],
    );
    let merged = ok(
        &h,
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            b.to_str().unwrap(),
            "--if-match",
            &base,
            "--merge",
        ],
    );
    assert_eq!(merged["merged"], true);
    let text = ok(&h, Scope::Read, &["doc", "get", &id])["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        text.contains("edited by A") && text.contains("edited by B"),
        "{text}"
    );
    let again = ok(
        &h,
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            b.to_str().unwrap(),
            "--if-match",
            &base,
            "--merge",
        ],
    );
    assert_eq!(again["changed"], false);
}

fn too_big() -> String {
    "x".repeat(1024 * 1024 + 10)
}

#[test]
fn item_create_md_discards_item_when_put_fails() {
    let h = Harness::start();
    let c = collection(&h, "Discard");
    let (code, out, err) = run_json(
        h.quire()
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                &c,
                "--kind",
                "doc",
                "--md",
                "-",
            ])
            .write_stdin(too_big()),
    );
    assert_eq!(code, 10, "the PUT's 413: {err}");
    assert_eq!(out, Value::Null);
    assert_eq!(last_error(&err)["error"]["code"], "payload_too_large");
    assert!(!err.contains("discard_failed"), "{err}");
    let tree = ok(&h, Scope::Read, &["tree", &c]);
    assert_eq!(tree["items"], json!([]), "the empty item was discarded");
    assert_eq!(
        ok(&h, Scope::Read, &["trash", "list"])["items"],
        json!([]),
        "not trashed: deleted"
    );
}

#[test]
fn item_create_md_sets_title_only_after_put() {
    let h = Harness::start();
    let c = collection(&h, "Titles");
    let (code, v, err) = run_json(
        h.quire()
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                &c,
                "--kind",
                "doc",
                "--title",
                "Set after",
                "--md",
                "-",
            ])
            .write_stdin("Body text.\n"),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["item"]["title"], "Set after");
    assert!(v["doc"]["etag"].as_str().unwrap().starts_with("\"d"), "{v}");
    // With a failing PUT the title is never set (and the item is gone).
    let (code, _, _) = run_json(
        h.quire()
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                &c,
                "--kind",
                "doc",
                "--title",
                "Never",
                "--md",
                "-",
            ])
            .write_stdin(too_big()),
    );
    assert_eq!(code, 10);
    let hits = ok(&h, Scope::Read, &["search", "--titles", "Never"]);
    assert_eq!(hits, json!([]));
    // --title-h1 takes it from the Markdown.
    let (code, v, err) = run_json(
        h.quire()
            .args([
                "--json",
                "item",
                "create",
                "--collection",
                &c,
                "--kind",
                "doc",
                "--md",
                "-",
                "--title-h1",
            ])
            .write_stdin("# From the heading\n\nText.\n"),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["item"]["title"], "From the heading");
}

#[test]
fn attach_images_refuses_paths_outside_base() {
    let h = Harness::start();
    let c = collection(&h, "Images");
    let id = doc(&h, &c, "Pictures", None);
    let s = &h.sandbox;
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\xf8\x0f\0\0\x01\x01\0\x05\x18\xd8N\0\0\0\0IEND\xaeB`\x82";
    s.file("docdir/ok.png", png, 0o644);
    s.file("docdir/fake.png", b"plain text", 0o644);
    let outside = s.file("outside.png", png, 0o644);
    symlink(&outside, s.path().join("docdir/link.png")).unwrap();
    let md = s.file(
        "docdir/page.md",
        format!(
            "![ok](ok.png)\n\n![up](../outside.png)\n\n![abs]({})\n\n![link](link.png)\n\n![fake](fake.png)\n",
            outside.display()
        )
        .as_bytes(),
        0o644,
    );
    let (code, v, err) = h.json(
        Scope::Write,
        &[
            "doc",
            "put",
            &id,
            "--md",
            md.to_str().unwrap(),
            "--force",
            "--attach-images",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["changed"], true);
    let skipped: Vec<String> = err
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|l| l["warning"]["code"] == "attach_skipped")
        .map(|l| l["warning"]["details"]["path"].as_str().unwrap().to_owned())
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
    let text = ok(&h, Scope::Read, &["doc", "get", &id])["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains("](/api/v1/attachments/"), "{text}");
    assert!(!text.contains("ok.png"), "{text}");
    // With stdin it needs --base-dir.
    let (code, _, _) = run_json(
        h.quire()
            .args([
                "--json",
                "doc",
                "put",
                &id,
                "--md",
                "-",
                "--force",
                "--attach-images",
            ])
            .write_stdin("![x](ok.png)\n"),
    );
    assert_eq!(code, 2);
}

#[test]
fn board_put_get_preserves_unknown_fields() {
    let h = Harness::start();
    let c = collection(&h, "Boards");
    let id = ok(
        &h,
        Scope::Write,
        &[
            "item",
            "create",
            "--collection",
            &c,
            "--kind",
            "board",
            "--title",
            "Plan",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../e2e/agent/fixtures/future.inkboard"
    );
    let put = ok(
        &h,
        Scope::Write,
        &["board", "put", &id, fixture, "--if-match", "r1"],
    );
    let etag = put["etag"].as_str().unwrap().to_owned();
    let got = ok(&h, Scope::Read, &["board", "get", &id]);
    assert_eq!(got["format"], "inkboard");
    assert_eq!(got["etag"], etag.as_str());
    let content = &got["content"];
    assert_eq!(content["document"]["x_future"]["note"], "kept");
    assert_eq!(content["document"]["pages"][0]["x_future"], 42);
    assert_eq!(content["document"]["name"], "Plan");
    // A stale If-Match with a changed board: 412. Re-sending the stored board would be a no-op (§6.2 step 4).
    let changed = std::fs::read_to_string(fixture)
        .unwrap()
        .replace(r#""note":"kept""#, r#""note":"changed""#);
    let changed = h
        .sandbox
        .file("changed.inkboard", changed.as_bytes(), 0o600);
    let changed = changed.to_str().unwrap();
    let (code, _, err) = h.json(
        Scope::Write,
        &["board", "put", &id, changed, "--if-match", "r1"],
    );
    assert_eq!(code, 9, "{err}");
    ok(&h, Scope::Write, &["board", "put", &id, changed, "--force"]);
    let got = ok(&h, Scope::Read, &["board", "get", &id]);
    assert_eq!(got["content"]["document"]["x_future"]["note"], "changed");
}

#[test]
fn search_all_merges_pages() {
    let h = Harness::start();
    let c = collection(&h, "Search");
    for i in 0..30 {
        doc(&h, &c, &format!("quokka {i}"), None);
    }
    let all = ok(
        &h,
        Scope::Read,
        &["search", "quokka", "--all", "--limit", "7"],
    );
    assert_eq!(all["items"].as_array().unwrap().len(), 30);
    assert_eq!(all["next_cursor"], Value::Null);
    let one = ok(&h, Scope::Read, &["search", "quokka", "--limit", "7"]);
    assert_eq!(one["items"].as_array().unwrap().len(), 7);
    assert!(one["next_cursor"].is_string());
}

#[test]
fn attachment_upload_get_same_sha256() {
    let h = Harness::start();
    let s = &h.sandbox;
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\xf8\x0f\0\0\x01\x01\0\x05\x18\xd8N\0\0\0\0IEND\xaeB`\x82";
    let f = s.file("pixel.png", png, 0o644);
    let up = ok(
        &h,
        Scope::Write,
        &["attachment", "upload", f.to_str().unwrap()],
    );
    let id = up["id"].as_str().unwrap().to_owned();
    let out = s.path().join("back.png");
    let got = ok(
        &h,
        Scope::Read,
        &["attachment", "get", &id, "--out", out.to_str().unwrap()],
    );
    assert_eq!(got["sha256"], up["sha256"]);
    assert_eq!(std::fs::read(&out).unwrap(), png);
    // Raw to stdout without --json.
    let raw = h
        .quire_as(Scope::Read)
        .args(["attachment", "get", &id])
        .assert()
        .success();
    assert_eq!(raw.get_output().stdout, png);
}

#[test]
fn trash_purge_needs_yes_and_admin() {
    let h = Harness::start();
    let c = collection(&h, "Trash");
    let id = doc(&h, &c, "Doomed", None);
    ok(&h, Scope::Write, &["item", "trash", &id]);
    let listed = ok(&h, Scope::Read, &["trash", "list"]);
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == id.as_str())
    );
    let (code, _, _) = h.json(Scope::Admin, &["trash", "purge", &id]);
    assert_eq!(code, 2, "no --yes");
    let (code, _, err) = h.json(Scope::Write, &["trash", "purge", &id, "--yes"]);
    assert_eq!(code, 6, "{err}");
    let v = ok(&h, Scope::Admin, &["trash", "purge", &id, "--yes"]);
    assert_eq!(v["ok"], true);
    let (code, _, _) = h.json(Scope::Read, &["item", "get", &id]);
    assert_eq!(code, 7, "gone for good");
}

#[test]
fn export_to_stdout_is_a_tar() {
    let h = Harness::start();
    collection(&h, "Exported");
    let out = h.quire_as(Scope::Admin).arg("export").assert().success();
    let tar = &out.get_output().stdout;
    assert!(
        tar.len() >= 1024 && tar.len().is_multiple_of(512),
        "{} bytes",
        tar.len()
    );
    assert_eq!(&tar[257..262], b"ustar");
    let names = String::from_utf8_lossy(tar);
    assert!(names.contains("manifest.json") && names.contains("quire.db"));
    let (code, _, _) = h.json(Scope::Write, &["export", "--out", "x.tar"]);
    assert_eq!(code, 6);
    let (code, _, _) = h.json(Scope::Admin, &["export"]);
    assert_eq!(code, 2, "--json needs --out for binary output");
}
