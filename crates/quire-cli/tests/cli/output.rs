//! stdout and stderr of §7.7 (owner: W8)

use serde_json::Value;

use crate::common::Sandbox;

#[test]
fn usage_error_is_json_when_flag_present() {
    let sandbox = Sandbox::new();
    for args in [
        &["exit-codes", "--json", "--bogus"][..],
        &["--json", "no-such-command"][..],
        &["--json"][..],
    ] {
        let out = sandbox.quire().args(args).assert().code(2).stdout("");
        let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("UTF-8");
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(lines.len(), 1, "one JSON line: {stderr}");
        let value: Value = serde_json::from_str(lines[0]).expect("a JSON line");
        assert_eq!(value["error"]["code"], "cli_usage", "{value}");
        assert!(value["error"]["message"].is_string(), "{value}");
        assert_eq!(value["request_id"], Value::Null, "{value}");
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
