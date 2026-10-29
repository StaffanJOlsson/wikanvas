//! `quire api`: the path and header guards, and calls through it (owner: W8)

use quire_api::Scope;
use serde_json::{Value, json};

use crate::common::{Harness, Mock, Reply, Sandbox, fake_token, last_error, run_json};

#[test]
fn path_and_header_guards() {
    let s = Sandbox::new();
    let mock = Mock::always(Reply::json(200, &json!({})));
    let refused: [&[&str]; 12] = [
        &["GET", "//host/x"],
        &["GET", "//example.invalid/x"],
        &["GET", "https://host/x"],
        &["GET", "/api/../x"],
        &["GET", "/api/%2e%2e/x"],
        &["GET", "/api/v1/%2E%2E/%2e%2e/healthz"],
        &["GET", "/healthz"],
        &["GET", "/api/v1/x\\y"],
        &[
            "GET",
            "/api/v1/sidebar",
            "--header",
            "Authorization: Bearer qpat_x",
        ],
        &[
            "GET",
            "/api/v1/sidebar",
            "--header",
            "cookie:quire_session=x",
        ],
        &["GET", "/api/v1/sidebar", "--header", "Host: evil.invalid"],
        &["TRACE", "/api/v1/sidebar"],
    ];
    for args in refused {
        let (code, out, err) = run_json(
            s.quire_at(&mock.url, &fake_token(1))
                .args(["--json", "api"])
                .args(args),
        );
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_eq!(out, Value::Null);
        assert_eq!(last_error(&err)["error"]["code"], "cli_usage", "{args:?}");
    }
    assert_eq!(mock.connections(), 0, "the mock saw nothing");
    // An allowed header and a body go through; the token is the CLI's own.
    let input = s.file("in.json", br#"{"name":"C"}"#, 0o600);
    s.quire_at(&mock.url, &fake_token(1))
        .args([
            "--json",
            "api",
            "post",
            "/api/v1/collections?x=1",
            "--header",
            "X-Test: yes",
        ])
        .args(["--input", input.to_str().unwrap()])
        .assert()
        .success();
    let r = &mock.requests()[0];
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/api/v1/collections?x=1")
    );
    assert_eq!(r.header("x-test"), Some("yes"));
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {}", fake_token(1)).as_str())
    );
    assert_eq!(r.body, br#"{"name":"C"}"#);
}

#[test]
fn api_against_the_server() {
    let h = Harness::start();
    let (code, v, err) = h.json(Scope::Read, &["api", "GET", "/api/v1/openapi.json"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["openapi"], "3.1.0");
    assert!(v["components"]["securitySchemes"]["bearer"].is_object());
    let (code, _, err) = h.json(Scope::Read, &["api", "GET", "/api/v1/nope"]);
    assert_eq!(code, 7, "{err}");
    assert_eq!(last_error(&err)["error"]["code"], "not_found");
    // A public route works without any token.
    let (code, v, err) = run_json(h.sandbox.quire().env("QUIRE_URL", &h.server.url).args([
        "--json",
        "api",
        "GET",
        "/api/v1/openapi.json",
    ]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(v["info"]["title"], "Quire API");
}
