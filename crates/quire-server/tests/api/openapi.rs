//! The OpenAPI spec tests: spec_matches_committed and spec_parses_as_3_1 (owner: W2)
//!
//! `docs/api/openapi.json` is generated, never hand-edited: after a route, DTO or annotation change run
//! `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server --locked --test api -- openapi::` and commit the file
//! (PLAN R4, ARCHITECTURE §7.4).

use std::path::PathBuf;

use crate::common::TestApp;

fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/openapi.json")
}

#[test]
fn spec_matches_committed() {
    let generated = quire_server::openapi::spec_json();
    let path = committed_path();
    if std::env::var_os("UPDATE_OPENAPI").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; generate it with `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server --locked \
             --test api -- openapi::`",
            path.display()
        )
    });
    assert!(
        committed == generated,
        "docs/api/openapi.json is out of date: run `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server \
         --locked --test api -- openapi::` and commit it"
    );
}

/// No crate of the workspace's test builds turns on serde_json's `preserve_order` (oas3's default `preserve-order` did,
/// through this test binary): object keys come out sorted, as in a release build.
#[test]
fn json_maps_are_sorted_as_in_release_builds() {
    let v: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":{"d":2,"c":3}}"#).unwrap();
    assert_eq!(v.to_string(), r#"{"a":{"c":3,"d":2},"b":1}"#);
}

#[test]
fn spec_parses_as_3_1() {
    // The generated spec, so this also holds while UPDATE_OPENAPI rewrites the committed copy; spec_matches_committed
    // ties the two together.
    let json = quire_server::openapi::spec_json();
    assert!(
        json.ends_with("}\n"),
        "pretty-printed with a trailing newline"
    );
    let spec = oas3::from_json(&json).expect("the spec parses as OpenAPI 3.1 (oas3)");
    assert_eq!(spec.openapi, "3.1.0");
    let paths = spec.paths.expect("paths");
    assert!(!paths.is_empty());
    for path in paths.keys() {
        assert!(path.starts_with("/api/v1/"), "{path} is not under /api/v1/");
    }
}

#[tokio::test]
async fn served_no_cache_with_etag() {
    let app = TestApp::new().await;
    let res = app.server.get("/api/v1/openapi.json").await;
    res.assert_status_ok();
    assert_eq!(res.content_type(), "application/json");
    assert_eq!(res.header("cache-control"), "no-cache");
    assert_eq!(res.text(), quire_server::openapi::spec_json());
    let etag = res.header("etag");
    let again = app
        .server
        .get("/api/v1/openapi.json")
        .add_header("if-none-match", etag.clone())
        .await;
    assert_eq!(again.status_code(), 304);
    assert_eq!(again.header("etag"), etag);
    assert_eq!(again.header("quire-api-version"), quire_api::API_VERSION);
}
