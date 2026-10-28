//! The exit-code contract of §7.7 (owner: W8)

use quire_api::cli::ExitCode;
use serde_json::{Value, json};

use crate::common::Sandbox;

#[test]
fn json_prints_the_table() {
    let sandbox = Sandbox::new();
    let expected = serde_json::to_value(ExitCode::table()).expect("the table serializes");
    for args in [["exit-codes", "--json"], ["--json", "exit-codes"]] {
        let out = sandbox.quire().args(args).assert().success().stderr("");
        let value: Value =
            serde_json::from_slice(&out.get_output().stdout).expect("stdout is one JSON value");
        assert_eq!(value, expected, "{args:?}");
    }
    let rows = expected.as_array().expect("an array");
    assert_eq!(rows.len(), 13);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["code"], i);
        assert_eq!(
            row.as_object().expect("an object").len(),
            3,
            "only code, name and meaning: {row}"
        );
    }
    assert_eq!(
        rows[9],
        json!({"code": 9, "name": "precondition_failed", "meaning": "Precondition failed"})
    );
}

#[test]
fn human_lists_every_code() {
    let out = Sandbox::new()
        .quire()
        .arg("exit-codes")
        .assert()
        .success()
        .stderr("");
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 13, "{stdout}");
    for (line, code) in lines.iter().zip(ExitCode::ALL) {
        let mut words = line.split_whitespace();
        assert_eq!(
            words.next(),
            Some(code.code().to_string().as_str()),
            "{line}"
        );
        assert_eq!(words.next(), Some(code.name()), "{line}");
    }
}
