//! `quire --version`, against a running TestServer (owner: W8)

use crate::common::Harness;

#[test]
fn flag_prints_name_and_version() {
    let h = Harness::start();
    assert!(
        h.server.url.starts_with("http://127.0.0.1:"),
        "{}",
        h.server.url
    );
    let expected = format!("quire {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        h.quire()
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
