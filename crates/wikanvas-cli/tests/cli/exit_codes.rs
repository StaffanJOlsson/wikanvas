//! The exit-code contract of §7.7 (owner: W8)

use serde_json::{Value, json};
use wikanvas_api::cli::ExitCode;

use crate::common::{Mock, Reply, Sandbox, fake_token, last_error, run_json};

#[test]
fn json_prints_the_table() {
    let sandbox = Sandbox::new();
    let expected = serde_json::to_value(ExitCode::table()).expect("the table serializes");
    for args in [["exit-codes", "--json"], ["--json", "exit-codes"]] {
        let out = sandbox.wikanvas().args(args).assert().success().stderr("");
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
        .wikanvas()
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

#[test]
fn every_status_maps_to_the_table() {
    let s = Sandbox::new();
    let cases: [(u16, Option<&str>, u8); 20] = [
        (302, None, 3),
        (400, Some("validation"), 10),
        (401, Some("invalid_token"), 5),
        (403, Some("insufficient_scope"), 6),
        (404, Some("not_found"), 7),
        (405, Some("method_not_allowed"), 10),
        (408, Some("request_timeout"), 12),
        (409, Some("conflict"), 8),
        (412, Some("rev_mismatch"), 9),
        (413, Some("payload_too_large"), 10),
        (415, Some("unsupported_media_type"), 10),
        (418, None, 10),
        (421, Some("host_not_allowed"), 3),
        (422, Some("validation"), 10),
        (428, Some("precondition_required"), 9),
        (429, Some("rate_limited"), 11),
        (500, Some("internal"), 12),
        (503, Some("unavailable"), 12),
        (507, Some("insufficient_storage"), 12),
        (599, None, 12),
    ];
    for (status, code, exit) in cases {
        let reply = match (status, code) {
            (302, _) => Reply::empty(302).with("Location", "https://elsewhere.invalid/"),
            (s, Some(c)) => Reply::error(s, c),
            (s, None) => Reply::raw(s, "text/html", b"<html>a proxy page</html>"),
        };
        let mock = Mock::always(reply);
        let (got, out, err) = run_json(s.wikanvas_at(&mock.url, &fake_token(1)).args([
            "--json",
            "--retries",
            "0",
            "auth",
            "whoami",
        ]));
        assert_eq!(got, i32::from(exit), "{status}: {err}");
        assert_eq!(out, Value::Null, "{status}");
        let e = last_error(&err);
        assert_eq!(e["status"], status, "{status}: {e}");
        assert_eq!(e["exit_code"], exit, "{status}: {e}");
        match code {
            Some(c) if status != 302 => assert_eq!(e["error"]["code"], c, "{e}"),
            _ => assert!(e["error"]["code"].is_string(), "{e}"),
        }
        assert_eq!(mock.requests().len(), 1, "{status}: --retries 0");
    }
}

/// The exit-code table in docs/AGENTS.md: rows `| N | Meaning | action |`.
fn agents_md_table() -> Vec<(u8, String)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/AGENTS.md");
    let text = std::fs::read_to_string(path).expect("docs/AGENTS.md");
    text.lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l
                .trim()
                .strip_prefix('|')?
                .strip_suffix('|')?
                .split('|')
                .map(str::trim)
                .collect();
            let code: u8 = cells.first()?.parse().ok()?;
            Some((code, cells.get(1)?.to_string()))
        })
        .collect()
}

#[test]
fn table_matches_agents_md() {
    let rows = agents_md_table();
    let expected: Vec<(u8, String)> = ExitCode::ALL
        .iter()
        .map(|c| (c.code(), c.meaning().to_owned()))
        .collect();
    assert_eq!(
        rows, expected,
        "docs/AGENTS.md repeats the §7.7 table (code and meaning, in order)"
    );
}
