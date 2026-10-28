//! stdout and stderr of §7.7 (owner: W8)

use std::collections::BTreeSet;
use std::time::Duration;

use clap::CommandFactory;
use quire_cli::args::Cli;
use serde_json::{Value, json};

use crate::common::{ID, Mock, Recorded, Reply, Sandbox, fake_token, last_error, run_json};

/// Every leaf command path of the clap tree (`auth login`, `tree`, …).
pub fn leaf_paths() -> BTreeSet<String> {
    fn walk(cmd: &clap::Command, prefix: &str, out: &mut BTreeSet<String>) {
        let subs: Vec<&clap::Command> = cmd.get_subcommands().collect();
        if subs.is_empty() {
            out.insert(prefix.to_owned());
            return;
        }
        for sub in subs {
            let p = if prefix.is_empty() {
                sub.get_name().to_owned()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            walk(sub, &p, out);
        }
    }
    let mut out = BTreeSet::new();
    walk(&Cli::command(), "", &mut out);
    out
}

fn summary() -> Value {
    json!({"id": ID, "url_id": "AbCdEfGhIj", "collection_id": ID, "parent_id": null, "kind": "doc", "title": "T",
           "sort_key": "a0", "child_count": 0, "published": true, "archived": false,
           "updated_at": "2026-10-28T00:00:00.000Z", "content_at": null, "created_at": "2026-10-28T00:00:00.000Z"})
}

/// A server that answers every route of §7.2 with a plausible body.
fn universal(req: &Recorded, _: usize) -> Reply {
    let p = req.path();
    let m = req.method.as_str();
    let page = json!({"items": [summary()], "next_cursor": null});
    match (m, p) {
        ("GET", "/api/v1/config") => Reply::json(
            200,
            &json!({"version": "0.1.0", "commit": "abc", "api_version": "1.0", "limits": {"max_upload_mb": 25}}),
        ),
        ("POST", "/api/v1/tokens") => Reply::json(
            201,
            &json!({"token": fake_token(42), "api_token": {"id": ID, "prefix": &fake_token(42)[..12]}}),
        ),
        ("POST", "/api/v1/items") => Reply::json(201, &summary()),
        (_, p) if p.ends_with("/doc/markdown") && m == "PUT" => Reply::json(
            200,
            &json!({"seq": 1, "etag": "\"d1\"", "updated_at": "t", "changed": true, "merged": false, "warnings": []}),
        ),
        (_, p) if p.ends_with("/doc/markdown") => {
            Reply::raw(200, "text/markdown; charset=utf-8", b"# T\n").with("ETag", "\"d1\"")
        }
        (_, p) if p.ends_with("/doc/prosemirror") => Reply::json(
            200,
            &json!({"schema_version": 1, "seq": 1, "doc": {"type": "doc"}}),
        )
        .with("ETag", "\"d1\""),
        ("PUT", p) if p.ends_with("/board") => {
            Reply::json(200, &json!({"rev": 2, "etag": "\"r2\""}))
        }
        ("GET", p) if p.ends_with("/board") => Reply::json(
            200,
            &json!({"format": "inkboard", "version": 2, "document": {}}),
        )
        .with("ETag", "\"r1\""),
        (_, p) if p.contains("/board/pages/") => Reply::raw(200, "image/svg+xml", b"<svg/>"),
        ("GET", "/api/v1/export") => Reply::raw(200, "application/x-tar", &[0u8; 1024]),
        ("GET", p) if p.starts_with("/api/v1/attachments/") => {
            Reply::raw(200, "image/png", b"\x89PNG\r\n\x1a\n")
        }
        ("POST", "/api/v1/attachments") => Reply::json(
            201,
            &json!({"id": ID, "url": format!("/api/v1/attachments/{ID}")}),
        ),
        (_, p) if p.ends_with("/star") => Reply::empty(204),
        ("DELETE", p)
            if p.starts_with("/api/v1/tokens")
                || p.starts_with("/api/v1/auth/sessions")
                || p.starts_with("/api/v1/trash") =>
        {
            Reply::empty(204)
        }
        (
            "GET",
            "/api/v1/recent" | "/api/v1/updated" | "/api/v1/drafts" | "/api/v1/starred"
            | "/api/v1/archive" | "/api/v1/search",
        ) => Reply::json(200, &page),
        ("GET", p) if p.ends_with("/updated") => Reply::json(200, &page),
        _ => Reply::json(200, &json!({"ok": "mock", "item": summary(), "id": ID})),
    }
}

#[test]
fn every_command_accepts_json() {
    let s = Sandbox::new();
    let mock = Mock::start(universal);
    let md = s.file("doc.md", b"# T\n\nBody\n", 0o600);
    let board = s.file(
        "b.inkboard",
        br#"{"format":"inkboard","version":2,"document":{}}"#,
        0o600,
    );
    let png = s.file("p.png", b"\x89PNG\r\n\x1a\nxx", 0o600);
    let stub = s.file(
        "quire-server",
        b"#!/bin/sh\necho '{\"stub\": true}'\n",
        0o755,
    );
    let out = |n: &str| s.path().join(n).to_string_lossy().into_owned();
    let (md, board, png) = (
        md.to_string_lossy().into_owned(),
        board.to_string_lossy().into_owned(),
        png.to_string_lossy().into_owned(),
    );
    let token = fake_token(1);
    let args: Vec<(&str, Vec<String>)> = vec![
        (
            "auth login",
            vec!["--url".into(), mock.url.clone(), "--token-stdin".into()],
        ),
        ("auth logout", vec![]),
        ("auth whoami", vec![]),
        ("me update", vec!["--name".into(), "N".into()]),
        ("session list", vec![]),
        ("session revoke", vec![ID.into()]),
        ("token list", vec![]),
        (
            "token create",
            vec!["--name".into(), "t".into(), "--scope".into(), "read".into()],
        ),
        ("token revoke", vec![ID.into()]),
        ("config show", vec![]),
        ("config path", vec![]),
        ("workspace get", vec![]),
        ("workspace set", vec!["--name".into(), "W".into()]),
        ("sidebar", vec![]),
        ("collection list", vec![]),
        ("collection get", vec![ID.into()]),
        ("collection create", vec!["--name".into(), "C".into()]),
        (
            "collection update",
            vec![ID.into(), "--name".into(), "C".into()],
        ),
        ("collection move", vec![ID.into()]),
        ("collection delete", vec![ID.into()]),
        ("collection restore", vec![ID.into()]),
        ("collection updated", vec![ID.into()]),
        ("item get", vec![ID.into()]),
        (
            "item create",
            vec![
                "--collection".into(),
                ID.into(),
                "--kind".into(),
                "doc".into(),
            ],
        ),
        ("item update", vec![ID.into(), "--title".into(), "T".into()]),
        ("item move", vec![ID.into(), "--top".into()]),
        ("item duplicate", vec![ID.into()]),
        ("item publish", vec![ID.into()]),
        ("item unpublish", vec![ID.into()]),
        ("item archive", vec![ID.into()]),
        ("item unarchive", vec![ID.into()]),
        ("item trash", vec![ID.into()]),
        ("item restore", vec![ID.into()]),
        ("item children", vec![ID.into()]),
        ("item text", vec![ID.into()]),
        ("tree", vec![ID.into()]),
        ("list recent", vec![]),
        ("list updated", vec![]),
        ("list drafts", vec![]),
        ("list starred", vec![]),
        ("list archive", vec![]),
        ("trash list", vec![]),
        ("trash purge", vec![ID.into(), "--yes".into()]),
        ("search", vec!["q".into()]),
        ("star add", vec![ID.into()]),
        ("star remove", vec![ID.into()]),
        ("doc get", vec![ID.into()]),
        (
            "doc put",
            vec![ID.into(), "--md".into(), md.clone(), "--force".into()],
        ),
        ("doc schema", vec![]),
        ("board get", vec![ID.into()]),
        (
            "board put",
            vec![ID.into(), board.clone(), "--force".into()],
        ),
        (
            "board preview",
            vec![ID.into(), "--out".into(), out("p0.svg")],
        ),
        ("attachment upload", vec![png.clone()]),
        (
            "attachment get",
            vec![ID.into(), "--out".into(), out("a.bin")],
        ),
        ("export", vec!["--out".into(), out("export.tar")]),
        ("backup", vec![]),
        ("api", vec!["GET".into(), "/api/v1/sidebar".into()]),
        ("admin", vec!["list-tokens".into()]),
        ("version", vec![]),
        ("exit-codes", vec![]),
        ("help", vec![]),
    ];
    let listed: BTreeSet<String> = args.iter().map(|(p, _)| (*p).to_owned()).collect();
    assert_eq!(
        listed,
        leaf_paths(),
        "every leaf command has a row here, and no other"
    );
    for (path, extra) in &args {
        let mut cmd = s.quire_at(&mock.url, &token);
        cmd.env("QUIRE_SERVER_BIN", &stub)
            .arg("--json")
            .args(path.split(' '))
            .args(extra);
        if *path == "auth login" {
            cmd.write_stdin(token.clone());
        }
        let (code, value, err) = run_json(&mut cmd);
        assert_eq!(code, 0, "quire {path} --json: {err}");
        assert!(!value.is_null(), "quire {path} --json printed nothing");
        for line in err.lines() {
            let v: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{path}: stderr line not JSON ({e}): {line}"));
            assert!(
                v.get("warning").is_some(),
                "{path}: only warnings on success: {line}"
            );
        }
    }
}

#[test]
fn json_stdout_is_the_body_unchanged() {
    let body = br#"{"user":{"id":"u","x_future":{"nested":[1,2,{"deep":null}]}},"auth":{"method":"token"},"x_new":"kept"}"#;
    let mock = Mock::always(Reply::raw(200, "application/json", body));
    let s = Sandbox::new();
    let out = s
        .quire_at(&mock.url, &fake_token(1))
        .args(["--json", "auth", "whoami"])
        .assert()
        .success()
        .stderr("");
    let mut expected = body.to_vec();
    expected.push(b'\n');
    assert_eq!(
        out.get_output().stdout,
        expected,
        "byte for byte, plus a newline"
    );
    // Wrapped content (board get, doc get --format json) and merged pages keep the server's bytes and key order.
    let envelope = r#"{"format":"inkboard","version":2,"document":{"z_last":1,"a_first":[1.50,{"x_future":true}]}}"#;
    let boards = Mock::always(
        Reply::raw(200, "application/json", envelope.as_bytes()).with("ETag", "\"r3\""),
    );
    let out = s
        .quire_at(&boards.url, &fake_token(1))
        .args(["--json", "board", "get", ID])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(text.contains(&format!("\"content\": {envelope}")), "{text}");
    assert!(
        text.find("\"item_id\"") < text.find("\"content\""),
        "{text}"
    );
    let page = r#"{"items":[{"z":1,"a":2.50}],"next_cursor":null}"#;
    let pages = Mock::always(Reply::raw(200, "application/json", page.as_bytes()));
    let out = s
        .quire_at(&pages.url, &fake_token(1))
        .args(["--json", "list", "recent", "--all"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(text.contains(r#"{"z":1,"a":2.50}"#), "{text}");
    // The same through `api`.
    let out = s
        .quire_at(&mock.url, &fake_token(1))
        .args(["--json", "api", "GET", "/api/v1/auth/me"])
        .assert()
        .success();
    assert_eq!(out.get_output().stdout, expected);
}

#[test]
fn no_content_prints_ok_object() {
    let mock = Mock::always(Reply::empty(204).with("X-Request-Id", "req-204"));
    let s = Sandbox::new();
    let (code, v, err) = run_json(
        s.quire_at(&mock.url, &fake_token(1))
            .args(["--json", "token", "revoke", ID]),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        v,
        json!({"ok": true, "status": 204, "request_id": "req-204", "id": ID})
    );
    let (_, v, _) = run_json(s.quire_at(&mock.url, &fake_token(1)).args([
        "--json",
        "api",
        "DELETE",
        "/api/v1/tokens/current",
    ]));
    assert_eq!(
        v,
        json!({"ok": true, "status": 204, "request_id": "req-204"})
    );
    assert_eq!(mock.requests()[0].method, "DELETE");
    assert_eq!(mock.requests()[0].path(), format!("/api/v1/tokens/{ID}"));
}

#[test]
fn json_error_on_stderr_and_empty_stdout() {
    let mock = Mock::always(Reply::error(404, "not_found").with("X-Request-Id", "mock-err"));
    let s = Sandbox::new();
    let out = s
        .quire_at(&mock.url, &fake_token(1))
        .args(["--json", "item", "get", ID])
        .assert()
        .code(7)
        .stdout("");
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    let e = last_error(&stderr);
    assert_eq!(e["error"]["code"], "not_found");
    assert_eq!(e["error"]["message"], "mock not_found");
    assert_eq!(e["request_id"], "mock-err");
    assert_eq!(e["status"], 404);
    assert_eq!(e["exit_code"], 7);
}

#[test]
fn json_stderr_is_json_lines() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::error(409, "cycle"));
    let readable = s.file("r.token", fake_token(2).as_bytes(), 0o644);
    let out = s
        .quire()
        .args([
            "--json",
            "--debug",
            "--url",
            &mock.url,
            "--token-file",
            readable.to_str().unwrap(),
        ])
        .args(["item", "move", ID, "--parent", ID])
        .assert()
        .code(8)
        .stdout("");
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    let lines: Vec<Value> = stderr
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    assert!(lines.len() >= 3, "{stderr}");
    assert_eq!(lines[0]["warning"]["code"], "token_file_permissions");
    assert!(lines.iter().any(|l| l.get("debug").is_some()), "{stderr}");
    let last = lines.last().unwrap();
    assert_eq!(last["error"]["code"], "cycle");
    assert_eq!(last["exit_code"], 8);
    assert_eq!(lines.iter().filter(|l| l.get("error").is_some()).count(), 1);
}

#[test]
fn usage_error_is_json_when_flag_present() {
    let sandbox = Sandbox::new();
    for args in [
        &["exit-codes", "--json", "--bogus"][..],
        &["--json", "no-such-command"][..],
        &["--json"][..],
        &["--json", "trash", "purge", ID][..],
    ] {
        let out = sandbox.quire().args(args).assert().code(2).stdout("");
        let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("UTF-8");
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(lines.len(), 1, "one JSON line: {stderr}");
        let value: Value = serde_json::from_str(lines[0]).expect("a JSON line");
        assert_eq!(value["error"]["code"], "cli_usage", "{value}");
        assert!(value["error"]["message"].is_string(), "{value}");
        assert_eq!(value["request_id"], Value::Null, "{value}");
        assert_eq!(value["status"], Value::Null, "{value}");
        assert_eq!(value["exit_code"], 2, "{value}");
    }
    // Without --json the usage error is human text.
    let out = sandbox
        .quire()
        .args(["exit-codes", "--bogus"])
        .assert()
        .code(2)
        .stdout("");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("UTF-8");
    assert!(stderr.starts_with("error: "), "{stderr}");
}

#[test]
fn bare_quire_prints_help_and_exits_2() {
    let out = Sandbox::new().quire().assert().code(2).stdout("");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("UTF-8");
    assert!(stderr.contains("Usage: quire"), "{stderr}");
    assert!(!stderr.contains('\u{1b}'), "no colour: {stderr}");
}

#[test]
fn content_write_without_if_match_or_force_exit_2() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &json!({})));
    let f = s.file("x.md", b"x", 0o600);
    let f = f.to_str().unwrap();
    for args in [
        &["doc", "put", ID, "--md", f][..],
        &["doc", "put", ID, "--json", f][..],
        &["board", "put", ID, f][..],
        &["doc", "put", ID, "--md", f, "--if-match", "d1", "--force"][..],
    ] {
        let (code, out, err) = run_json(
            s.quire_at(&mock.url, &fake_token(1))
                .arg("--json")
                .args(args),
        );
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_eq!(out, Value::Null);
        assert_eq!(last_error(&err)["error"]["code"], "cli_usage");
    }
    assert_eq!(mock.connections(), 0);
}

#[test]
fn if_match_accepts_bare_quoted_and_weak() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(
        200,
        &json!({"seq": 43, "etag": "\"d43\"", "updated_at": "t", "changed": true, "merged": false, "warnings": []}),
    ));
    let f = s.file("x.md", b"# x\n", 0o600);
    let f = f.to_str().unwrap();
    for (flag, sent) in [
        (&["--if-match", "d42"][..], "\"d42\""),
        (&["--if-match", "\"d42\""][..], "\"d42\""),
        (&["--if-match", "W/\"d42\""][..], "\"d42\""),
        (&["--force"][..], "*"),
    ] {
        let (code, _, err) = run_json(
            s.quire_at(&mock.url, &fake_token(1))
                .args(["--json", "doc", "put", ID, "--md", f])
                .args(flag),
        );
        assert_eq!(code, 0, "{flag:?}: {err}");
        let r = mock.requests().pop().unwrap();
        assert_eq!(r.header("if-match"), Some(sent), "{flag:?}");
        assert_eq!(
            r.header("content-type"),
            Some("text/markdown; charset=utf-8")
        );
        assert_eq!(r.body, b"# x\n");
    }
    // The board PUT and item update take the same forms.
    s.quire_at(&mock.url, &fake_token(1))
        .args(["--json", "board", "put", ID, f, "--if-match", "W/\"r7\""])
        .assert()
        .success();
    assert_eq!(
        mock.requests().pop().unwrap().header("if-match"),
        Some("\"r7\"")
    );
    s.quire_at(&mock.url, &fake_token(1))
        .args([
            "--json",
            "item",
            "update",
            ID,
            "--title",
            "T",
            "--if-match",
            "m3",
        ])
        .assert()
        .success();
    assert_eq!(
        mock.requests().pop().unwrap().header("if-match"),
        Some("\"m3\"")
    );
}

#[test]
fn never_prompts_with_closed_stdin() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(
        200,
        &json!({"seq": 1, "etag": "\"d1\"", "updated_at": "t", "changed": false, "merged": false, "warnings": []}),
    ));
    // assert_cmd closes stdin when no input is given; nothing may wait on a terminal.
    let quick = |cmd: &mut assert_cmd::Command| {
        cmd.timeout(Duration::from_secs(20));
        run_json(cmd)
    };
    let (code, _, err) = quick(s.quire().args([
        "--json",
        "auth",
        "login",
        "--url",
        &mock.url,
        "--token-stdin",
    ]));
    assert_eq!(code, 3, "{err}");
    let (code, _, err) = quick(s.quire().args([
        "--json",
        "--url",
        &mock.url,
        "--token-stdin",
        "auth",
        "whoami",
    ]));
    assert_eq!(code, 3, "{err}");
    let (code, _, err) = quick(
        s.quire_at(&mock.url, &fake_token(1))
            .args(["--json", "doc", "put", ID, "--md", "-", "--force"]),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        mock.requests().last().unwrap().body,
        b"",
        "an empty stdin is an empty document"
    );
    let (code, _, _) = quick(
        s.quire_at(&mock.url, &fake_token(1))
            .args(["--json", "trash", "purge", ID]),
    );
    assert_eq!(code, 2, "no --yes: no question asked either");
    let (code, _, _) = quick(
        s.quire_at(&mock.url, &fake_token(1))
            .args(["--json", "board", "put", ID, "-"]),
    );
    assert_eq!(code, 2);
}

#[test]
fn debug_never_prints_authorization() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &json!({"user": {}, "auth": {}})));
    let token = fake_token(3);
    for json_flag in [true, false] {
        let mut cmd = s.quire_at(&mock.url, &token);
        cmd.args(["--debug", "auth", "whoami"]);
        if json_flag {
            cmd.arg("--json");
        }
        let out = cmd.assert().success();
        let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
        assert!(stderr.contains("/api/v1/auth/me"), "a debug line: {stderr}");
        assert!(stderr.contains("200"), "{stderr}");
        assert!(stderr.contains("mock-"), "the request id: {stderr}");
        for secret in [token.as_str(), &token[..12], "Bearer", "uthorization"] {
            assert!(!stderr.contains(secret), "{secret} in {stderr}");
        }
        if json_flag {
            let v: Value = serde_json::from_str(stderr.lines().next().unwrap()).unwrap();
            let d = &v["debug"];
            assert_eq!(d["method"], "GET");
            assert_eq!(d["status"], 200);
            assert!(d["duration_ms"].is_u64());
            assert!(d.get("headers").is_none());
        }
    }
    assert_eq!(
        mock.requests()[0].header("authorization"),
        Some(format!("Bearer {token}").as_str())
    );
}

#[test]
fn help_json_lists_every_command() {
    let (code, v, err) = run_json(Sandbox::new().quire().args(["help", "--json"]));
    assert_eq!(code, 0, "{err}");
    fn walk(v: &Value, out: &mut BTreeSet<String>) {
        match v["commands"].as_array() {
            Some(list) => list.iter().for_each(|c| walk(c, out)),
            None => {
                out.insert(v["path"].as_str().unwrap().to_owned());
            }
        }
    }
    let mut listed = BTreeSet::new();
    for c in v["commands"].as_array().unwrap() {
        walk(c, &mut listed);
    }
    assert_eq!(listed, leaf_paths());
    assert_eq!(v["exit_codes"].as_array().unwrap().len(), 13);
    let globals: Vec<&str> = v["global_flags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    for g in [
        "--json",
        "--url",
        "--profile",
        "--token-file",
        "--token-stdin",
        "--timeout",
        "--retries",
        "--ca-file",
        "--debug",
    ] {
        assert!(globals.contains(&g), "{g} in {globals:?}");
    }
    // One command's subtree, and the human form.
    let (code, v, _) = run_json(
        Sandbox::new()
            .quire()
            .args(["help", "doc", "put", "--json"]),
    );
    assert_eq!(code, 0);
    assert_eq!(v["path"], "doc put");
    assert!(v["one_of"].to_string().contains("--force"), "{v}");
    let out = Sandbox::new()
        .quire()
        .args(["help", "doc", "put"])
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("--if-match"));
    Sandbox::new()
        .quire()
        .args(["help", "nope"])
        .assert()
        .code(2);
}

#[test]
fn all_truncation_keeps_next_cursor() {
    // An endless list: cursor "c<offset>", pages of `limit` items.
    let mock = Mock::start(|req, _| {
        let offset: usize = req
            .query("cursor")
            .and_then(|c| c.strip_prefix('c'))
            .map_or(0, |n| n.parse().unwrap());
        let limit: usize = req.query("limit").map_or(25, |n| n.parse().unwrap());
        let items: Vec<Value> = (offset..offset + limit)
            .map(|i| json!({"id": i, "title": format!("t{i}")}))
            .collect();
        Reply::json(
            200,
            &json!({"items": items, "next_cursor": format!("c{}", offset + limit)}),
        )
    });
    let s = Sandbox::new();
    let (code, v, err) = run_json(
        s.quire_at(&mock.url, &fake_token(1))
            .args(["--json", "list", "recent", "--all"]),
    );
    assert_eq!(code, 0, "{err}");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 10_000);
    assert_eq!(items[9_999]["id"], 9_999);
    assert_eq!(
        v["next_cursor"], "c10000",
        "the real cursor, to continue from"
    );
    let warning: Value = serde_json::from_str(err.lines().next().unwrap()).unwrap();
    assert_eq!(warning["warning"]["code"], "truncated");
    assert_eq!(mock.requests().len(), 100, "pages of 100");
    assert!(
        mock.requests()
            .iter()
            .all(|r| r.query("limit") == Some("100"))
    );

    // A list that ends: next_cursor null, no warning; --limit sets the page size.
    let ending = Mock::start(|req, _| {
        let offset: usize = req
            .query("cursor")
            .and_then(|c| c.strip_prefix('c'))
            .map_or(0, |n| n.parse().unwrap());
        let limit: usize = req.query("limit").unwrap().parse().unwrap();
        let end = (offset + limit).min(130);
        let items: Vec<Value> = (offset..end).map(|i| json!({"id": i})).collect();
        let next = (end < 130).then(|| format!("c{end}"));
        Reply::json(200, &json!({"items": items, "next_cursor": next}))
    });
    let (code, v, err) = run_json(
        s.quire_at(&ending.url, &fake_token(1))
            .args(["--json", "search", "x", "--all", "--limit", "50"]),
    );
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(v["items"].as_array().unwrap().len(), 130);
    assert_eq!(v["next_cursor"], Value::Null);
    assert_eq!(ending.requests().len(), 3);
    // A failing page leaves stdout empty.
    let failing = Mock::start(|_, i| {
        if i == 0 {
            Reply::json(200, &json!({"items": [{"id": 1}], "next_cursor": "c1"}))
        } else {
            Reply::error(500, "internal")
        }
    });
    let (code, v, err) = run_json(s.quire_at(&failing.url, &fake_token(1)).args([
        "--json",
        "list",
        "drafts",
        "--all",
        "--retries",
        "0",
    ]));
    assert_eq!(code, 12, "{err}");
    assert_eq!(v, Value::Null);
}
