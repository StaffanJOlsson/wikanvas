//! `wikanvas --version` against a running TestServer, and `wikanvas version` without a server (owner: W8)

use std::net::TcpListener;

use serde_json::Value;

use crate::common::{Harness, Sandbox, run_json};

#[test]
fn flag_prints_name_and_version() {
    let h = Harness::start();
    assert!(
        h.server.url.starts_with("http://127.0.0.1:"),
        "{}",
        h.server.url
    );
    let expected = format!("wikanvas {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        h.wikanvas()
            .arg(flag)
            .assert()
            .success()
            .stdout(expected.clone())
            .stderr("");
    }
    // Nothing was written to the sandbox.
    assert!(!h.sandbox.config_path().exists());
    assert_eq!(
        std::fs::read_dir(h.sandbox.home()).expect("HOME").count(),
        0
    );
}

/// `version` never fails because of the server (D-W8-6): with no URL, `server` is null and nothing is said; when the
/// server cannot be asked, `server` is null and a `server_unavailable` warning names the error.
#[test]
fn version_without_a_server_exits_0() {
    let s = Sandbox::new();
    let (code, v, err) = run_json(s.wikanvas().args(["--json", "version"]));
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(v["cli"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["cli"]["api_version"], wikanvas_api::API_VERSION);
    assert_eq!(v["server"], Value::Null);

    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (code, v, err) = run_json(
        s.wikanvas()
            .env("WIKANVAS_URL", format!("http://127.0.0.1:{port}"))
            .args(["--json", "--retries", "0", "version"]),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["server"], Value::Null);
    let lines: Vec<Value> = err
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(lines.len(), 1, "{err}");
    let w = &lines[0]["warning"];
    assert_eq!(w["code"], "server_unavailable", "{err}");
    assert_eq!(w["details"]["error"]["error"]["code"], "network", "{err}");
    assert_eq!(
        w["details"]["error"]["error"]["details"]["sent"], false,
        "{err}"
    );
}
