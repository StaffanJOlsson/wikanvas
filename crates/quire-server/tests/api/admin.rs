//! Admin command tests (owner: W2)
//!
//! W0b: `create-owner` and `create-token` run as the real binary on a temporary data directory.

use std::io::Write as _;
use std::process::{Command, Output, Stdio};

fn admin(dir: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_quire-server"))
        .arg("admin")
        .args(args)
        .env_clear()
        .env("QUIRE_DATA_DIR", dir)
        .env("QUIRE_TEST_ARGON2", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run quire-server admin");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(input) = stdin {
            pipe.write_all(input.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout {:?}, stderr {:?}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[test]
fn create_owner_once_then_exit_8() {
    let dir = tempfile::tempdir().unwrap();
    let owner = [
        "create-owner",
        "--email",
        "a@quire.invalid",
        "--name",
        "A",
        "--workspace",
        "W",
        "--generate-password",
        "--json",
    ];
    let out = admin(dir.path(), &owner, None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let created = json(&out);
    assert_eq!(created["user"]["email"], "a@quire.invalid");
    assert_eq!(created["user"]["role"], "owner");
    assert_eq!(created["workspace"]["name"], "W");
    assert_eq!(created["generated_password"].as_str().unwrap().len(), 24);

    let again = admin(dir.path(), &owner, None);
    assert_eq!(again.status.code(), Some(8));
    let err: serde_json::Value =
        serde_json::from_slice(again.stderr.trim_ascii()).expect("a JSON error line on stderr");
    assert_eq!(err["error"]["code"], "already_setup");
    assert_eq!(err["exit_code"], 8);
    assert!(again.stdout.is_empty());
}

#[test]
fn create_owner_password_stdin_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let args = [
        "create-owner",
        "--email",
        "a@quire.invalid",
        "--name",
        "A",
        "--workspace",
        "W",
        "--password-stdin",
    ];
    let short = admin(dir.path(), &args, Some("short\n"));
    assert_eq!(
        short.status.code(),
        Some(10),
        "{}",
        String::from_utf8_lossy(&short.stderr)
    );
    let ok = admin(dir.path(), &args, Some("a long enough password\n"));
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let usage = admin(dir.path(), &["create-owner", "--email", "a@x"], None);
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn create_token_json_and_token_out() {
    let dir = tempfile::tempdir().unwrap();
    let no_owner = admin(
        dir.path(),
        &["create-token", "--name", "t", "--scope", "read", "--json"],
        None,
    );
    assert_eq!(
        no_owner.status.code(),
        Some(8),
        "{}",
        String::from_utf8_lossy(&no_owner.stderr)
    );

    let owner = [
        "create-owner",
        "--email",
        "a@quire.invalid",
        "--name",
        "A",
        "--workspace",
        "W",
        "--generate-password",
    ];
    assert_eq!(admin(dir.path(), &owner, None).status.code(), Some(0));

    let out = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "write",
            "--expires",
            "1d",
            "--json",
        ],
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let created = json(&out);
    let token = created["token"].as_str().unwrap();
    assert!(quire_api::token::check(token).is_ok());
    assert_eq!(created["api_token"]["created_via"], "admin");
    assert_eq!(
        created["api_token"]["scopes"],
        serde_json::json!(["read", "write"])
    );

    let file = dir.path().join("token");
    let out = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t2",
            "--scope",
            "read",
            "--token-out",
            file.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = json(&out);
    assert!(
        written.get("token").is_none(),
        "the secret goes to the file only"
    );
    assert_eq!(written["token_file"], file.to_str().unwrap());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let secret = std::fs::read_to_string(&file).unwrap();
    assert!(quire_api::token::check(secret.trim_end()).is_ok());

    let unknown = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "read",
            "--email",
            "b@quire.invalid",
        ],
        None,
    );
    assert_eq!(unknown.status.code(), Some(7));
    let bad = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "read",
            "--expires",
            "1w",
        ],
        None,
    );
    assert_eq!(bad.status.code(), Some(10));
}

#[test]
fn unimplemented_commands_exit_1() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["list-tokens"][..],
        &["check"],
        &["backup"],
        &["revoke-token", "--all", "--yes"],
    ] {
        let out = admin(dir.path(), args, None);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("not implemented (W2)"));
    }
}
