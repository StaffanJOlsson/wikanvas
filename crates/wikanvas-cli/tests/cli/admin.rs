//! The `wikanvas admin …` passthrough to `wikanvas-server admin …` (owner: W8)

use std::os::unix::fs::PermissionsExt;

use serde_json::Value;

use assert_cmd::assert::OutputAssertExt;

use crate::common::{Sandbox, last_error, run_json, spawn_output};

fn stub(s: &Sandbox, name: &str, exit: u8) -> std::path::PathBuf {
    let log = s.path().join("args.log");
    s.file(
        name,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho '{{\"stub\": true}}'\necho 'stub stderr' >&2\nexit {exit}\n",
            log.display()
        )
        .as_bytes(),
        0o755,
    )
}

#[test]
fn passthrough_execs_wikanvas_server_with_args_and_exit_code() {
    let s = Sandbox::new();
    let bin = stub(&s, "fake-wikanvas-server", 7);
    let log = s.path().join("args.log");
    let out = s
        .wikanvas()
        .env("WIKANVAS_SERVER_BIN", &bin)
        .args([
            "admin",
            "create-token",
            "--name",
            "ci",
            "--scope",
            "write",
            "--expires",
            "1d",
        ])
        .assert()
        .code(7);
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "{\"stub\": true}\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stderr),
        "stub stderr\n"
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "admin\ncreate-token\n--name\nci\n--scope\nwrite\n--expires\n1d\n"
    );
    // A global --json is passed on (once), wherever it was.
    for args in [
        &["--json", "admin", "list-tokens"][..],
        &["admin", "list-tokens", "--json"][..],
    ] {
        s.wikanvas()
            .env("WIKANVAS_SERVER_BIN", &bin)
            .args(args)
            .assert()
            .code(7);
        let logged = std::fs::read_to_string(&log).unwrap();
        assert_eq!(logged.matches("--json").count(), 1, "{logged}");
        assert!(logged.starts_with("admin\n"), "{logged}");
    }
    // Success passes through as success.
    let ok = stub(&s, "ok-server", 0);
    s.wikanvas()
        .env("WIKANVAS_SERVER_BIN", &ok)
        .args(["admin", "check"])
        .assert()
        .success();
    // Next to the wikanvas binary.
    let dir = s.path().join("bin");
    std::fs::create_dir_all(&dir).unwrap();
    let wikanvas = dir.join("wikanvas");
    std::fs::copy(env!("CARGO_BIN_EXE_wikanvas"), &wikanvas).unwrap();
    std::fs::copy(&ok, dir.join("wikanvas-server")).unwrap();
    std::fs::set_permissions(
        dir.join("wikanvas-server"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    // A fresh copy: run through `spawn_output`, which retries a spawn that finds it busy (ETXTBSY).
    spawn_output(
        s.wikanvas_bin(&wikanvas)
            .env("PATH", "")
            .args(["admin", "backup"]),
    )
    .assert()
    .success();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "admin\nbackup\n");
}

#[test]
fn missing_wikanvas_server_exit_3() {
    let s = Sandbox::new();
    // A copy of wikanvas with no wikanvas-server beside it, WIKANVAS_SERVER_BIN unset and an empty PATH.
    let dir = s.path().join("alone");
    std::fs::create_dir_all(&dir).unwrap();
    let wikanvas = dir.join("wikanvas");
    std::fs::copy(env!("CARGO_BIN_EXE_wikanvas"), &wikanvas).unwrap();
    let (code, out, err) =
        run_json(
            s.wikanvas_bin(&wikanvas)
                .env("PATH", "")
                .args(["--json", "admin", "list-tokens"]),
        );
    assert_eq!(code, 3, "{err}");
    assert_eq!(out, Value::Null);
    let e = last_error(&err);
    assert_eq!(e["error"]["code"], "cli_config");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("wikanvas-server not found"),
        "{e}"
    );
}
