//! Coverage: every spec operation has a command (or is on the §7.6 allow-list), and every `quire …` example in
//! docs/AGENTS.md parses with the clap tree (owner: W8)

use std::collections::BTreeSet;

use quire_cli::{API_ONLY, OPERATIONS};
use serde_json::Value;

fn repo_file(rel: &str) -> String {
    let path = format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn every_spec_operation_has_command_or_api_only_allowlist() {
    let spec: Value =
        serde_json::from_str(&repo_file("docs/api/openapi.json")).expect("the spec parses");
    let paths = spec["paths"].as_object().expect("paths");
    assert!(!paths.is_empty());
    let covered: BTreeSet<(String, String)> = OPERATIONS
        .iter()
        .map(|(m, p, _)| ((*m).to_owned(), (*p).to_owned()))
        .chain(
            API_ONLY
                .iter()
                .map(|(m, p)| ((*m).to_owned(), (*p).to_owned())),
        )
        .collect();
    let mut missing = Vec::new();
    let mut count = 0;
    for (path, ops) in paths {
        for method in ops.as_object().unwrap().keys() {
            if !["get", "put", "post", "patch", "delete", "head"].contains(&method.as_str()) {
                continue;
            }
            count += 1;
            let key = (method.to_ascii_uppercase(), path.clone());
            if !covered.contains(&key) {
                missing.push(format!("{} {}", key.0, key.1));
            }
        }
    }
    assert!(count > 0);
    assert!(
        missing.is_empty(),
        "spec operations without a command or an allow-list entry: {missing:#?}"
    );
    // The allow-list is §7.6's, plus the spec itself and the board's browser download (docs/handoff/W8.md).
    let allow: BTreeSet<String> = API_ONLY.iter().map(|(m, p)| format!("{m} {p}")).collect();
    let expected: BTreeSet<String> = [
        "GET /api/v1/setup",
        "POST /api/v1/setup",
        "POST /api/v1/auth/login",
        "POST /api/v1/auth/logout",
        "POST /api/v1/auth/password",
        "POST /api/v1/items/{id}/view",
        "GET /api/v1/items/{id}/doc",
        "POST /api/v1/items/{id}/doc/updates",
        "GET /api/v1/openapi.json",
        "GET /api/v1/items/{id}/board.inkboard",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(allow, expected);
    // No operation is both a command's and allow-listed.
    for (m, p, c) in OPERATIONS {
        assert!(
            !allow.contains(&format!("{m} {p}")),
            "{m} {p} ({c}) is also allow-listed"
        );
    }
}

/// Splits a shell line into words (single and double quotes, backslash escapes). Stops at a comment.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for d in chars.by_ref() {
                    if d == '\'' {
                        break;
                    }
                    cur.push(d);
                }
            }
            '"' => {
                in_word = true;
                while let Some(d) = chars.next() {
                    match d {
                        '"' => break,
                        '\\' => {
                            if let Some(e) = chars.next() {
                                cur.push(e);
                            }
                        }
                        _ => cur.push(d),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(e) = chars.next() {
                    cur.push(e);
                }
            }
            '#' if !in_word => break,
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

/// The `quire …` invocations in the fenced code blocks of docs/AGENTS.md: each command of a pipeline or list whose
/// first word (after `VAR=value` assignments, and after `docker compose exec [opts] SERVICE`) is `quire`, up to its
/// first redirection.
fn agents_md_examples() -> Vec<Vec<String>> {
    let text = repo_file("docs/AGENTS.md");
    let mut in_code = false;
    let mut found = Vec::new();
    let mut pending = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            continue;
        }
        let line = line.trim();
        if let Some(cont) = line.strip_suffix('\\') {
            pending.push_str(cont);
            pending.push(' ');
            continue;
        }
        let full = std::mem::take(&mut pending) + line;
        let full = full.strip_prefix("$ ").unwrap_or(&full).to_owned();
        for segment in full.split(['|', ';']).flat_map(|s| s.split("&&")) {
            let mut words = shell_words(segment);
            if let Some(i) = words
                .iter()
                .position(|w| w.starts_with('>') || w.starts_with('<') || w == "2>&1")
            {
                words.truncate(i);
            }
            let mut rest: &[String] = &words;
            while rest
                .first()
                .is_some_and(|w| w.contains('=') && !w.starts_with('-'))
            {
                rest = &rest[1..];
            }
            if rest.len() >= 3 && rest[0] == "docker" && rest[1] == "compose" {
                // docker compose [-p P] [-f F] exec [-T] [-e VAR] SERVICE quire …
                let Some(exec) = rest.iter().position(|w| w == "exec") else {
                    continue;
                };
                let mut i = exec + 1;
                while i < rest.len() && rest[i].starts_with('-') {
                    i += if matches!(
                        rest[i].as_str(),
                        "-e" | "--env" | "-u" | "--user" | "-w" | "--workdir"
                    ) {
                        2
                    } else {
                        1
                    };
                }
                rest = rest.get(i + 1..).unwrap_or_default();
            }
            if rest.first().is_some_and(|w| w == "quire") {
                found.push(rest.to_vec());
            }
        }
    }
    found
}

#[test]
fn agents_md_examples_parse() {
    let examples = agents_md_examples();
    assert!(
        examples.len() >= 20,
        "docs/AGENTS.md has its examples: {examples:?}"
    );
    let mut groups = BTreeSet::new();
    for words in &examples {
        if let Err(e) = quire_cli::args::try_parse_from(words.iter().map(Into::into).collect())
            && e.use_stderr()
        {
            panic!("docs/AGENTS.md: `{}` does not parse:\n{e}", words.join(" "));
        }
        if let Some(g) = words.get(1) {
            groups.insert(g.clone());
        }
    }
    // The cheat sheet shows every command group of §7.6.
    for g in [
        "auth",
        "token",
        "config",
        "collection",
        "item",
        "tree",
        "list",
        "trash",
        "search",
        "star",
        "doc",
        "board",
        "attachment",
        "export",
        "api",
        "admin",
        "version",
        "exit-codes",
    ] {
        assert!(
            groups.contains(g),
            "no `quire {g} …` example in docs/AGENTS.md"
        );
    }
}

#[test]
fn shell_words_split_like_bash() {
    assert_eq!(
        shell_words(r#"quire search "quok -zebra" --json # comment"#),
        ["quire", "search", "quok -zebra", "--json"]
    );
    assert_eq!(shell_words("a 'b c' d\\ e"), ["a", "b c", "d e"]);
}
