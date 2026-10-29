//! Full-text search: the FTS5 query builder, ranked and dated search with snippets, and title search (owner: W1).
//!
//! D27: `item_fts` is an external-content FTS5 table over `item_text(title, body)` (`unicode61 remove_diacritics 2`,
//! prefixes 2 and 3), ranked with bm25, the title weighted 8×. Nothing a user types reaches FTS5 as syntax:
//! [`build_match`] quotes every term (§5 "Search query"). Snippets mark hits with U+E000 … U+E001, which
//! [`parse_snippet`] splits into text parts; clients render them as text nodes, never as HTML.

use wikanvas_core::api::page::{MAX_LIMIT, MAX_OFFSET_RESULTS};
use wikanvas_core::api::{Page, SearchHit, SearchQuery, SearchSort, SnippetPart, TitleHit};
use wikanvas_core::cursor::{Cursor, SortValue};
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use crate::error::{Error, Result};
use crate::lists::{PageRequest, page_limit};
use crate::shared::{ItemRow, Viewer, visible_items};
use crate::tree::{CHILD_COUNT, breadcrumbs};

/// The longest `q` (§7.1).
pub const MAX_QUERY_CHARS: usize = 500;
/// The default and largest `limit` of `GET /search/titles` (§7.1).
pub const TITLES_DEFAULT_LIMIT: u32 = 10;
pub const TITLES_MAX_LIMIT: u32 = 20;
/// The start and end markers of a hit in a raw snippet.
pub const HIT_START: char = '\u{E000}';
pub const HIT_END: char = '\u{E001}';
const _: () = assert!(TITLES_MAX_LIMIT <= MAX_LIMIT);
/// The most tokens a snippet holds (FTS5 allows 64).
const SNIPPET_TOKENS: u32 = 24;

/// One term of a query.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    text: String,
    phrase: bool,
    negative: bool,
}

/// Splits a query into terms: whitespace separates terms, `"…"` keeps a phrase (an unclosed quote runs to the end),
/// a leading `-` makes a term negative. Terms without a letter or digit (which the tokenizer would drop) are left out.
fn terms(q: &str) -> Vec<Term> {
    let chars: Vec<char> = q.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let negative = chars[i] == '-' && chars.get(i + 1).is_some_and(|c| !c.is_whitespace());
        if negative {
            i += 1;
        }
        let (text, phrase) = if chars[i] == '"' {
            let start = i + 1;
            let end = chars[start..]
                .iter()
                .position(|&c| c == '"')
                .map_or(chars.len(), |p| start + p);
            i = end + 1;
            (chars[start..end].iter().collect::<String>(), true)
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            (chars[start..i].iter().collect::<String>(), false)
        };
        if text.chars().any(char::is_alphanumeric) {
            out.push(Term {
                text,
                phrase,
                negative,
            });
        }
    }
    out
}

/// The FTS5 MATCH expression for a user's query (§5): every positive term or phrase double-quoted (embedded `"`
/// doubled) and joined with AND, the last positive term (when it is a bare word, not a phrase) as a prefix (`*`);
/// negative terms (`-word`, `-"phrase"`) follow as ` NOT (… OR …)`. FTS5 accepts `a NOT b` but neither
/// `a AND NOT b` nor a leading NOT. `None` when there is no positive term: such a query finds nothing, and no FTS
/// query runs.
pub fn build_match(q: &str) -> Option<String> {
    let terms = terms(q);
    let quote = |t: &Term| format!("\"{}\"", t.text.replace('"', "\"\""));
    let positives: Vec<&Term> = terms.iter().filter(|t| !t.negative).collect();
    let last = positives.last()?;
    let mut pos: Vec<String> = positives.iter().map(|t| quote(t)).collect();
    if !last.phrase {
        pos.last_mut().expect("a positive term").push('*');
    }
    let mut out = format!("({})", pos.join(" AND "));
    let negatives: Vec<String> = terms.iter().filter(|t| t.negative).map(quote).collect();
    if !negatives.is_empty() {
        out.push_str(&format!(" NOT ({})", negatives.join(" OR ")));
    }
    Some(out)
}

/// Splits a snippet marked with [`HIT_START`] / [`HIT_END`] into parts. Empty parts are dropped and neighbours of the
/// same kind merged.
pub fn parse_snippet(raw: &str) -> Vec<SnippetPart> {
    let mut parts: Vec<SnippetPart> = Vec::new();
    let mut hit = false;
    let mut cur = String::new();
    let flush = |cur: &mut String, hit: bool, parts: &mut Vec<SnippetPart>| {
        if cur.is_empty() {
            return;
        }
        match parts.last_mut() {
            Some(last) if last.hit == hit => last.text.push_str(cur),
            _ => parts.push(SnippetPart {
                text: cur.clone(),
                hit,
            }),
        }
        cur.clear();
    };
    for c in raw.chars() {
        match c {
            HIT_START => {
                flush(&mut cur, hit, &mut parts);
                hit = true;
            }
            HIT_END => {
                flush(&mut cur, hit, &mut parts);
                hit = false;
            }
            c => cur.push(c),
        }
    }
    flush(&mut cur, hit, &mut parts);
    parts
}

/// `GET /search`: items whose title or body match `q`, through `visible_items`, filtered by collection, kind and
/// `updated_after`; archived items only with `include_archived`; drafts unless `include_drafts=false`. Sorted by
/// relevance (offset cursor, at most 1,000 results) or by `updated` (keyset). 422 `validation` for a `q` over 500
/// characters (`field: "q"`), a bad `limit` or `cursor`.
pub fn search(conn: &Connection, viewer: &Viewer, q: &SearchQuery) -> Result<Page<SearchHit>> {
    if q.q.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::validation("q", "q is at most 500 characters"));
    }
    let sort = q.sort.unwrap_or_default();
    let include_drafts = q.include_drafts.unwrap_or(true);
    let req = PageRequest {
        list: "search".into(),
        params: vec![
            ("q".into(), q.q.trim().to_owned()),
            (
                "collection_id".into(),
                q.collection_id.map(|c| c.to_string()).unwrap_or_default(),
            ),
            (
                "kind".into(),
                q.kind.map(|k| k.to_string()).unwrap_or_default(),
            ),
            (
                "updated_after".into(),
                q.updated_after
                    .map(|t| t.millis().to_string())
                    .unwrap_or_default(),
            ),
            ("include_archived".into(), q.include_archived.to_string()),
            ("include_drafts".into(), include_drafts.to_string()),
            ("sort".into(), sort.to_string()),
        ],
        cursor: q.cursor.clone(),
        limit: q.limit,
    };
    let limit = req.page_size()?;
    let after = req.after()?;
    let Some(fts) = build_match(&q.q) else {
        return Ok(Page::new(Vec::new(), None));
    };
    let mut sql = format!(
        "SELECT {}, {CHILD_COUNT}, bm25(item_fts, 8.0, 1.0),
           snippet(item_fts, 1, char(57344), char(57345), '…', {SNIPPET_TOKENS})
         FROM item_fts JOIN item_text t ON t.text_id = item_fts.rowid
           JOIN items i ON i.id = t.item_id JOIN collections c ON c.id = i.collection_id
         WHERE item_fts MATCH ? AND {}",
        ItemRow::COLUMNS,
        visible_items(viewer)
    );
    let mut args = vec![Value::Text(fts)];
    if !q.include_archived {
        sql.push_str(" AND i.archived_at IS NULL");
    }
    if !include_drafts {
        sql.push_str(" AND i.published_at IS NOT NULL");
    }
    if let Some(c) = q.collection_id {
        sql.push_str(" AND i.collection_id = ?");
        args.push(Value::Text(c.to_string()));
    }
    if let Some(k) = q.kind {
        sql.push_str(" AND i.kind = ?");
        args.push(Value::Text(k.to_string()));
    }
    if let Some(t) = q.updated_after {
        sql.push_str(" AND i.updated_at > ?");
        args.push(Value::Integer(t.millis()));
    }
    let take;
    let offset;
    match (sort, after) {
        (SearchSort::Relevance, None) => {
            offset = 0;
            take = limit.min(MAX_OFFSET_RESULTS);
        }
        (SearchSort::Relevance, Some(Cursor::Offset(n))) => {
            offset = n.min(MAX_OFFSET_RESULTS);
            take = limit.min(MAX_OFFSET_RESULTS - offset);
        }
        (SearchSort::Updated, None) => {
            offset = 0;
            take = limit;
        }
        (
            SearchSort::Updated,
            Some(Cursor::Keyset {
                sort: SortValue::Int(at),
                id,
            }),
        ) => {
            offset = 0;
            take = limit;
            sql.push_str(" AND (i.updated_at, i.id) < (?, ?)");
            args.push(Value::Integer(at));
            args.push(Value::Text(id));
        }
        _ => {
            return Err(Error::validation(
                "cursor",
                "the cursor belongs to a different query",
            ));
        }
    }
    if take == 0 {
        return Ok(Page::new(Vec::new(), None));
    }
    match sort {
        SearchSort::Relevance => {
            sql.push_str(" ORDER BY bm25(item_fts, 8.0, 1.0), i.id LIMIT ? OFFSET ?")
        }
        SearchSort::Updated => {
            sql.push_str(" ORDER BY i.updated_at DESC, i.id DESC LIMIT ? OFFSET ?")
        }
    }
    args.push(Value::Integer(i64::from(take) + 1));
    args.push(Value::Integer(i64::from(offset)));
    let rows = conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args.iter()), |r| {
            Ok((
                ItemRow::from_row(r)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT)?,
                r.get::<_, f64>(ItemRow::COLUMN_COUNT + 1)?,
                r.get::<_, String>(ItemRow::COLUMN_COUNT + 2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = rows.len() > take as usize;
    let mut hits = Vec::with_capacity(rows.len().min(take as usize));
    for (row, children, rank, snippet) in rows.into_iter().take(take as usize) {
        hits.push(SearchHit {
            item: row.summary(children as u64)?,
            path: breadcrumbs(conn, &row.id)?,
            snippet: parse_snippet(&snippet),
            score: -rank,
        });
    }
    let next = match (more, sort, hits.last()) {
        (true, SearchSort::Relevance, Some(_)) => {
            let n = offset + hits.len() as u32;
            (n < MAX_OFFSET_RESULTS).then(|| req.next(&Cursor::Offset(n)))
        }
        (true, SearchSort::Updated, Some(last)) => Some(req.next(&Cursor::after_int(
            last.item.updated_at.millis(),
            last.item.id.to_string(),
        ))),
        _ => None,
    };
    Ok(Page::new(hits, next))
}

/// `GET /search/titles` (quick switcher and mentions): visible, unarchived items whose title matches `q` (prefix on
/// the last word, accents folded), best match first; for a blank `q`, the most recently updated items. `limit` 1–20
/// (default 10); 422 `validation` for `q` over 500 characters or a bad `limit`.
pub fn titles(
    conn: &Connection,
    viewer: &Viewer,
    q: &str,
    limit: Option<u32>,
) -> Result<Vec<TitleHit>> {
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::validation("q", "q is at most 500 characters"));
    }
    let limit = page_limit(limit, TITLES_MAX_LIMIT, TITLES_DEFAULT_LIMIT)?;
    let base = |from: &str, extra: &str| {
        format!(
            "SELECT {}, {CHILD_COUNT} FROM {from} JOIN collections c ON c.id = i.collection_id
             WHERE {} AND i.archived_at IS NULL {extra}",
            ItemRow::COLUMNS,
            visible_items(viewer)
        )
    };
    let (sql, args) = if q.trim().is_empty() {
        (
            base("items i", "ORDER BY i.updated_at DESC, i.id DESC LIMIT ?"),
            vec![Value::Integer(i64::from(limit))],
        )
    } else {
        let Some(fts) = build_match(q) else {
            return Ok(Vec::new());
        };
        (
            base(
                "item_fts JOIN item_text t ON t.text_id = item_fts.rowid JOIN items i ON i.id = t.item_id",
                "AND item_fts MATCH ? ORDER BY bm25(item_fts, 1.0, 0.0), i.updated_at DESC, i.id LIMIT ?",
            ),
            vec![
                Value::Text(format!("title : ({fts})")),
                Value::Integer(i64::from(limit)),
            ],
        )
    };
    let rows = conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args.iter()), |r| {
            Ok((
                ItemRow::from_row(r)?,
                r.get::<_, i64>(ItemRow::COLUMN_COUNT)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(row, n)| {
            Ok(TitleHit {
                item: row.summary(n as u64)?,
                path: breadcrumbs(conn, &row.id)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_expressions() {
        assert_eq!(build_match("plan"), Some(r#"("plan"*)"#.into()));
        assert_eq!(
            build_match("plan -x").as_deref(),
            Some(r#"("plan"*) NOT ("x")"#)
        );
        assert_eq!(
            build_match(r#"a OR b* NEAR( "x"#).as_deref(),
            Some(r#"("a" AND "OR" AND "b*" AND "NEAR(" AND "x")"#)
        );
        assert_eq!(
            build_match(r#""quoted phrase" tail -"not this" -no"#).as_deref(),
            Some(r#"("quoted phrase" AND "tail"*) NOT ("not this" OR "no")"#)
        );
        assert_eq!(build_match(r#"say"hi"#).as_deref(), Some(r#"("say""hi"*)"#));
        assert_eq!(build_match("-x"), None);
        assert_eq!(build_match("   "), None);
        assert_eq!(build_match("( ) * -"), None);
        assert_eq!(
            build_match("- x").as_deref(),
            Some(r#"("x"*)"#),
            "a lone dash is no negation"
        );
        assert_eq!(
            build_match(r#""last phrase""#).as_deref(),
            Some(r#"("last phrase")"#)
        );
    }

    #[test]
    fn snippets_split_on_markers() {
        let raw =
            format!("a {HIT_START}plan{HIT_END} for {HIT_START}x{HIT_END}{HIT_START}y{HIT_END}");
        assert_eq!(
            parse_snippet(&raw),
            [
                SnippetPart {
                    text: "a ".into(),
                    hit: false
                },
                SnippetPart {
                    text: "plan".into(),
                    hit: true
                },
                SnippetPart {
                    text: " for ".into(),
                    hit: false
                },
                SnippetPart {
                    text: "xy".into(),
                    hit: true
                },
            ]
        );
        assert!(parse_snippet("").is_empty());
    }
}
