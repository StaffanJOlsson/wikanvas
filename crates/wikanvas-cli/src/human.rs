//! Short human output (§7.7: not stable). Lists print as columns of id, kind, title and updated time; anything
//! else prints as indented JSON (owner: W8)
//!
//! Titles, names and messages come from the server: [`clean`] shows their control characters as escapes, so none of
//! them reaches the terminal as an escape sequence (§7.6: nothing prints colour). `output::print_text` and the human
//! stderr lines apply it to everything they print.

use std::borrow::Cow;

use serde_json::Value;

/// `text` with every control character (C0, DEL, C1) except `\n` shown as its escape (`\u{1b}`, `\t`).
pub fn clean(text: &str) -> Cow<'_, str> {
    escape_controls(text, true)
}

/// Like [`clean`], for one cell of a row: `\n` is escaped too.
fn one_line(text: &str) -> Cow<'_, str> {
    escape_controls(text, false)
}

fn escape_controls(text: &str, keep_newline: bool) -> Cow<'_, str> {
    let escaped = |c: char| c.is_control() && !(keep_newline && c == '\n');
    if !text.chars().any(escaped) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if escaped(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

fn text(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => one_line(s).into_owned(),
        other => other.to_string(),
    }
}

/// One list row: the element's `item` when it has one (search hits, title hits), else the element.
fn row(e: &Value) -> [String; 4] {
    let it = e.get("item").filter(|i| i.is_object()).unwrap_or(e);
    let kind = match it.get("kind") {
        Some(k) => text(k),
        None if it.get("prefix").is_some() => "token".into(),
        None if it.get("color").is_some() => "collection".into(),
        None => "-".into(),
    };
    let title = it
        .get("title")
        .or_else(|| it.get("name"))
        .map(text)
        .unwrap_or_else(|| "-".into());
    let when = it
        .get("updated_at")
        .or_else(|| it.get("created_at"))
        .or_else(|| it.get("last_seen_at"))
        .map(text)
        .unwrap_or_else(|| "-".into());
    let id = it.get("id").map(text).unwrap_or_else(|| "-".into());
    [id, kind, title, when]
}

/// Columns for a list of rows.
pub fn table(rows: &[Value]) -> String {
    let rows: Vec<[String; 4]> = rows.iter().map(row).collect();
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let (w0, w1, w2) = (width(0), width(1), width(2).min(60));
    let mut out = String::new();
    for r in &rows {
        let title: String = r[2].chars().take(60).collect();
        out.push_str(&format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {}\n",
            r[0], r[1], title, r[3]
        ));
    }
    out
}

/// The human form of a JSON result.
pub fn render(v: &Value) -> String {
    if let Some(list) = v.as_array()
        && list.iter().all(Value::is_object)
    {
        return table(list);
    }
    if let Some(items) = v.get("items").and_then(Value::as_array)
        && v.get("next_cursor").is_some()
    {
        let mut out = table(items);
        if let Some(c) = v["next_cursor"].as_str() {
            out.push_str(&format!("next cursor: {c}\n"));
        }
        return out;
    }
    let mut out = serde_json::to_string_pretty(v).unwrap_or_default();
    out.push('\n');
    out
}

/// `wikanvas tree`: the items indented under their parents, in the response's order.
pub fn tree(v: &Value) -> String {
    let items = v["items"].as_array().cloned().unwrap_or_default();
    let mut out = String::new();
    if let Some(name) = v["collection"]["name"].as_str() {
        out.push_str(&format!("{}\n", one_line(name)));
    }
    fn walk(out: &mut String, items: &[Value], parent: &Value, depth: usize) {
        for it in items.iter().filter(|i| &i["parent_id"] == parent) {
            out.push_str(&format!(
                "{}{}  [{} {}]\n",
                "  ".repeat(depth + 1),
                text(&it["title"]),
                text(&it["kind"]),
                text(&it["id"])
            ));
            walk(out, items, &it["id"], depth + 1);
        }
    }
    walk(&mut out, &items, &Value::Null, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_are_columns() {
        let page = json!({"items": [
            {"id": "a", "kind": "doc", "title": "One", "updated_at": "t1"},
            {"item": {"id": "b", "kind": "board", "title": "Two", "updated_at": "t2"}, "score": 1.0}
        ], "next_cursor": "c"});
        let out = render(&page);
        assert!(out.contains("a  doc    One  t1"), "{out}");
        assert!(out.contains("b  board  Two  t2"), "{out}");
        assert!(out.ends_with("next cursor: c\n"), "{out}");
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(clean("plain\nlines"), "plain\nlines");
        assert!(matches!(clean("plain"), Cow::Borrowed(_)));
        assert_eq!(
            clean("a\u{1b}[31mred\u{7}\tb\u{9b}c\u{7f}\n"),
            "a\\u{1b}[31mred\\u{7}\\tb\\u{9b}c\\u{7f}\n"
        );
        assert_eq!(one_line("x\ny"), "x\\ny");
        let page =
            json!([{"id": "a", "kind": "doc", "title": "\u{1b}]0;owned\u{7}T", "updated_at": "t"}]);
        let out = render(&page);
        assert!(!out.contains('\u{1b}') && !out.contains('\u{7}'), "{out:?}");
        let t = json!({"collection": {"name": "C\u{1b}[2J"}, "items": [
            {"id": "a", "parent_id": null, "kind": "doc", "title": "A\u{9b}1m"}
        ]});
        let out = tree(&t);
        assert!(
            !out.contains('\u{1b}') && !out.contains('\u{9b}'),
            "{out:?}"
        );
    }

    #[test]
    fn tree_indents() {
        let t = json!({"collection": {"name": "C"}, "items": [
            {"id": "a", "parent_id": null, "kind": "doc", "title": "A"},
            {"id": "b", "parent_id": "a", "kind": "doc", "title": "B"}
        ]});
        assert_eq!(tree(&t), "C\n  A  [doc a]\n    B  [doc b]\n");
    }
}
