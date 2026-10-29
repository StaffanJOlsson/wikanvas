//! `quire auth` against a running TestServer (owner: W8)

use quire_api::Scope;
use serde_json::Value;

use crate::common::{Harness, fake_token, last_error, run_json};

#[test]
fn whoami_shows_token_scopes() {
    let h = Harness::start();
    for (scope, expanded) in [
        (Scope::Read, vec!["read"]),
        (Scope::Write, vec!["read", "write"]),
        (Scope::Admin, vec!["read", "write", "admin"]),
    ] {
        let (code, v, err) = h.json(scope, &["auth", "whoami"]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(v["auth"]["method"], "token");
        assert_eq!(v["user"]["email"], quire_server::testing::OWNER_EMAIL);
        assert_eq!(
            v["workspace"]["name"],
            quire_server::testing::WORKSPACE_NAME
        );
        let scopes: Vec<&str> = v["auth"]["token"]["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert_eq!(scopes, expanded, "{scope}");
        assert_eq!(v["auth"]["token"]["prefix"], &h.server.token(scope)[..12]);
        // The human form names the scopes and never the secret.
        let out = h
            .quire_as(scope)
            .args(["auth", "whoami"])
            .assert()
            .success();
        let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
        assert!(text.contains(&expanded.join(", ")), "{text}");
        assert!(!text.contains(h.server.token(scope)), "{text}");
    }
}

#[test]
fn unknown_token_exit_5() {
    let h = Harness::start();
    let (code, out, err) = run_json(
        h.sandbox
            .quire_at(&h.server.url, &fake_token(99))
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!(code, 5, "{err}");
    assert_eq!(out, Value::Null);
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "invalid_token");
    assert_eq!(e["status"], 401);
    assert!(e["request_id"].is_string());
}

#[test]
fn read_token_write_command_exit_6() {
    let h = Harness::start();
    let (code, out, err) = h.json(Scope::Read, &["collection", "create", "--name", "Nope"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(out, Value::Null);
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "insufficient_scope");
    assert_eq!(e["error"]["details"]["required"], "write");
    // The write token may.
    let (code, v, err) = h.json(Scope::Write, &["collection", "create", "--name", "Yes"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["name"], "Yes");
}

#[test]
fn login_then_profile_drives_commands() {
    let h = Harness::start();
    let s = &h.sandbox;
    let (code, v, err) = run_json(
        s.quire()
            .args([
                "--json",
                "auth",
                "login",
                "--url",
                &h.server.url,
                "--token-stdin",
            ])
            .write_stdin(h.server.read_token.clone()),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["auth"]["token"]["prefix"], &h.server.read_token[..12]);
    // No QUIRE_URL or QUIRE_TOKEN now: the default profile has both.
    let (code, v, err) = run_json(s.quire().args(["--json", "auth", "whoami"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["user"]["email"], quire_server::testing::OWNER_EMAIL);
    let (_, v, _) = run_json(s.quire().args(["--json", "config", "show"]));
    assert_eq!(v["url_source"], "profile");
    assert_eq!(v["token_source"], "profile");
}
