//! `wikanvas token` and `wikanvas auth logout --revoke` against a running TestServer (owner: W8)

use std::os::unix::fs::PermissionsExt;

use wikanvas_api::Scope;
use serde_json::Value;

use crate::common::{Harness, last_error, run_json};

#[test]
fn create_prints_secret_once_and_revoke_works() {
    let h = Harness::start();
    let (code, v, err) = h.json(
        Scope::Admin,
        &[
            "token",
            "create",
            "--name",
            "agent",
            "--scope",
            "read",
            "--expires",
            "1d",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let secret = v["token"].as_str().unwrap().to_owned();
    assert!(wikanvas_api::token::check(&secret).is_ok(), "{secret}");
    let id = v["api_token"]["id"].as_str().unwrap().to_owned();
    assert_eq!(v["api_token"]["scopes"], serde_json::json!(["read"]));
    assert!(v["api_token"]["expires_at"].is_string());
    // The list shows the prefix and never the secret.
    let out = h
        .wikanvas_as(Scope::Admin)
        .args(["--json", "token", "list"])
        .assert()
        .success();
    let listed = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(!listed.contains(&secret), "the secret is never listed");
    let list: Value = serde_json::from_str(&listed).unwrap();
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .expect("listed");
    assert_eq!(row["prefix"], &secret[..12]);
    // The new token works, then is revoked.
    let (code, _, err) = run_json(
        h.sandbox
            .wikanvas_at(&h.server.url, &secret)
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!(code, 0, "{err}");
    let (code, v, err) = h.json(Scope::Admin, &["token", "revoke", &id]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], 204);
    assert_eq!(v["id"], id.as_str());
    let (code, _, err) = run_json(
        h.sandbox
            .wikanvas_at(&h.server.url, &secret)
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!(code, 5, "{err}");
    // A write token cannot list or create tokens; a token can never mint admin.
    let (code, _, err) = h.json(Scope::Write, &["token", "list"]);
    assert_eq!(code, 6, "{err}");
    let (code, _, err) = h.json(
        Scope::Admin,
        &["token", "create", "--name", "x", "--scope", "admin"],
    );
    assert_eq!(code, 6, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "session_required");
    // A bad --expires is a usage error before any request.
    let (code, _, _) = h.json(
        Scope::Admin,
        &[
            "token",
            "create",
            "--name",
            "x",
            "--scope",
            "read",
            "--expires",
            "1w",
        ],
    );
    assert_eq!(code, 2);
}

#[test]
fn token_out_writes_0600_and_keeps_secret_off_stdout() {
    let h = Harness::start();
    let path = h.sandbox.path().join("agent.token");
    let p = path.to_str().unwrap();
    let out = h
        .wikanvas_as(Scope::Admin)
        .args([
            "--json",
            "token",
            "create",
            "--name",
            "file",
            "--scope",
            "write",
            "--expires",
            "1d",
            "--token-out",
            p,
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    let secret = std::fs::read_to_string(&path).unwrap();
    assert!(secret.ends_with('\n'));
    let secret = secret.trim_end();
    assert!(wikanvas_api::token::check(secret).is_ok());
    assert!(
        !stdout.contains(secret) && !stderr.contains(secret),
        "the secret is only in the file"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["token_file"], p);
    assert_eq!(v["api_token"]["prefix"], &secret[..12]);
    assert!(v.get("token").is_none());
    // The file works as --token-file; the human form keeps the secret off stdout too.
    let (code, _, err) = run_json(h.sandbox.wikanvas().args([
        "--json",
        "--url",
        &h.server.url,
        "--token-file",
        p,
        "auth",
        "whoami",
    ]));
    assert_eq!(code, 0, "{err}");
    let out = h
        .wikanvas_as(Scope::Admin)
        .args([
            "token",
            "create",
            "--name",
            "f2",
            "--scope",
            "read",
            "--expires",
            "1d",
            "--token-out",
            p,
        ])
        .assert()
        .success();
    let secret2 = std::fs::read_to_string(&path).unwrap();
    let human = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(!human.contains(secret2.trim_end()), "{human}");
}

#[test]
fn logout_revoke_revokes_current_token() {
    let h = Harness::start();
    let (_, v, _) = h.json(
        Scope::Admin,
        &[
            "token",
            "create",
            "--name",
            "session",
            "--scope",
            "read",
            "--expires",
            "1d",
        ],
    );
    let secret = v["token"].as_str().unwrap().to_owned();
    let s = &h.sandbox;
    s.wikanvas()
        .args([
            "--json",
            "auth",
            "login",
            "--url",
            &h.server.url,
            "--token-stdin",
        ])
        .write_stdin(secret.clone())
        .assert()
        .success();
    assert!(
        std::fs::read_to_string(s.config_path())
            .unwrap()
            .contains(&secret)
    );
    let (code, v, err) = run_json(s.wikanvas().args(["--json", "auth", "logout", "--revoke"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], 204);
    assert_eq!(v["profile"], "default");
    assert_eq!(v["removed"], true);
    assert!(
        !std::fs::read_to_string(s.config_path())
            .unwrap()
            .contains(&secret),
        "removed from the profile"
    );
    let (code, _, err) = run_json(
        s.wikanvas_at(&h.server.url, &secret)
            .args(["--json", "auth", "whoami"]),
    );
    assert_eq!(code, 5, "revoked on the server: {err}");
    // The other tokens still work.
    let (code, _, _) = h.json(Scope::Read, &["auth", "whoami"]);
    assert_eq!(code, 0);
}

/// `auth logout --revoke` revokes the token it removes, the profile's, at the profile's URL, even when the shell
/// exports another token (WIKANVAS_TOKEN) that the resolved pair would otherwise use.
#[test]
fn logout_revoke_revokes_the_profiles_token_not_the_environments() {
    let h = Harness::start();
    let mint = |name: &str| {
        let (code, v, err) = h.json(
            Scope::Admin,
            &[
                "token",
                "create",
                "--name",
                name,
                "--scope",
                "read",
                "--expires",
                "1d",
            ],
        );
        assert_eq!(code, 0, "{err}");
        v["token"].as_str().unwrap().to_owned()
    };
    let (profile_token, shell_token) = (mint("profile"), mint("shell"));
    let s = &h.sandbox;
    s.wikanvas()
        .args([
            "--json",
            "auth",
            "login",
            "--url",
            &h.server.url,
            "--token-stdin",
        ])
        .write_stdin(profile_token.clone())
        .assert()
        .success();
    let whoami = |token: &str| {
        run_json(
            s.wikanvas_at(&h.server.url, token)
                .args(["--json", "auth", "whoami"]),
        )
        .0
    };

    // The URL comes from the profile, the token in play from WIKANVAS_TOKEN.
    let (code, v, err) = run_json(
        s.wikanvas()
            .env("WIKANVAS_TOKEN", &shell_token)
            .args(["--json", "auth", "logout", "--revoke"]),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["status"], 204);
    assert_eq!(v["removed"], true);
    assert_eq!(whoami(&profile_token), 5, "the removed token is revoked");
    assert_eq!(whoami(&shell_token), 0, "the shell's token still works");
    assert!(
        !std::fs::read_to_string(s.config_path())
            .unwrap()
            .contains(&profile_token)
    );

    // The profile holds no token now: --revoke has nothing to revoke (exit 3), and the shell's token is untouched.
    let (code, _, err) = run_json(
        s.wikanvas()
            .env("WIKANVAS_TOKEN", &shell_token)
            .args(["--json", "auth", "logout", "--revoke"]),
    );
    assert_eq!(code, 3, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "cli_config");
    assert_eq!(whoami(&shell_token), 0);
}
