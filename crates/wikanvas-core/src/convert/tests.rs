//! The conversion tests of PLAN W4 "Acceptance" (`convert::tests::*`), and the server fixture writer.

use std::path::PathBuf;

use proptest::prelude::*;
use serde_json::{Value, json};
use yrs::updates::decoder::Decode;
use yrs::{Any, BranchID, ReadTxn, StateVector, Text, Transact, Update, XmlFragment, XmlOut};

use super::*;
use crate::schema::{content_of, type_of};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn writing() -> bool {
    std::env::var("WRITE_FIXTURES").is_ok_and(|v| v == "1")
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn web_fixtures() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root().join("ydoc")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "bin") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&path).unwrap()));
        }
    }
    out.sort();
    assert!(out.len() >= 9);
    out
}

fn doc_of(bin: &[u8]) -> yrs::Doc {
    ydiff::load(&[bin], None).unwrap()
}

fn raw_of(doc: &yrs::Doc) -> Value {
    let frag = pm::fragment(doc);
    let txn = doc.transact();
    pm::raw(&txn, &frag).unwrap()
}

#[test]
fn y_to_raw_matches_web() {
    for (name, bin) in web_fixtures() {
        let want = read_json(&root().join(format!("ydoc/{name}.y.json")));
        assert_eq!(raw_of(&doc_of(&bin)), want, "{name}.y.json");
    }
}

#[test]
fn y_to_pm_matches_web() {
    for (name, bin) in web_fixtures() {
        let want = read_json(&root().join(format!("ydoc/{name}.pm.json")));
        assert_eq!(pm::read(&doc_of(&bin)).unwrap(), want, "{name}.pm.json");
    }
}

// --------------------------------------------------------------------------------------------- generators

const UUIDS: &[&str] = &[
    "0192f0a0-1111-7000-8000-00000000000a",
    "0192f0a0-2222-7000-8000-00000000000b",
    "0192f0a0-3333-7000-8000-00000000000c",
];

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => "[a-zA-Z0-9]{1,8}",
        1 => "[a-z]{1,3}[.,!?;'\"()*_#`<>+|~=&%$-][a-z]{1,3}",
        1 => "[*_#`<>+|~=&-]{1,2}[a-z]{1,4}",
        1 => "[a-z]{1,4}[\\\\]",
        1 => Just("é😀ñ".to_owned()),
        // Punctuation (and symbols) at the start or end of a word, so of a marked run.
        1 => "[(\"'.,!?:;@)_=😀][a-z]{1,4}",
        1 => "[a-z]{1,4}[)\"'.,!?:;@(_=😀]",
        // Text comrak's autolink extension would read as a link, and highlight delimiters in text.
        1 => "[a-z]{1,4}[.+_-]?[a-z]?@[a-z]{1,4}\\.[a-z]{2,3}",
        1 => prop::sample::select(vec!["https://x.co", "www.x.co", "mailto:a@b.co", "a=b=c", "x\u{a0}y", "\u{a0}z"])
            .prop_map(str::to_owned),
    ]
}

fn words() -> impl Strategy<Value = String> {
    prop::collection::vec(word(), 1..4).prop_map(|w| w.join(" "))
}

fn mark_set() -> impl Strategy<Value = Vec<Value>> {
    let delimited = || vec!["bold", "italic", "strike", "underline", "highlight"];
    let hrefs = vec!["https://example.com/a", "/doc/x", "#frag", "mailto:a@b.c"];
    prop_oneof![
        4 => Just(vec![]),
        2 => prop::sample::subsequence(delimited(), 1..4)
            .prop_map(|m| m.into_iter().map(|n| json!({"type": n})).collect()),
        1 => Just(vec![json!({"type": "code"})]),
        1 => (prop::sample::select(hrefs), prop::sample::subsequence(delimited(), 0..2))
            .prop_map(|(h, m)| {
                let mut marks: Vec<Value> = m.into_iter().map(|n| json!({"type": n})).collect();
                marks.push(json!({"type": "link", "attrs": {"href": h, "itemId": null}}));
                marks
            }),
    ]
}

/// Inline content: marked segments, hard breaks and mentions, next to each other or separated by a plain space;
/// a segment may start or end with a space inside its marks.
fn inline(allow_breaks: bool) -> impl Strategy<Value = Vec<Value>> {
    let seg = (words(), mark_set(), 0u8..5).prop_map(|(t, m, edge)| {
        let t = match edge {
            1 => format!(" {t}"),
            2 => format!("{t} "),
            _ => t,
        };
        let mut n = json!({"type": "text", "text": t});
        if !m.is_empty() {
            n["marks"] = Value::Array(m);
        }
        n
    });
    let piece = prop_oneof![
        6 => seg.prop_map(Some),
        1 => prop::sample::select(UUIDS.to_vec()).prop_map(|id| Some(json!({"type": "mention", "attrs": {"itemId": id}}))),
        1 => Just(None),
    ];
    prop::collection::vec((piece, any::<bool>()), 1..6).prop_map(move |pieces| {
        let mut out: Vec<Value> = Vec::new();
        for (p, adjacent) in pieces {
            match p {
                Some(n) => {
                    if !adjacent
                        && let Some(last) = out.last()
                        && type_of(last) != "hardBreak"
                    {
                        out.push(json!({"type": "text", "text": " "}));
                    }
                    out.push(n);
                }
                None if allow_breaks && out.last().is_some_and(|l| type_of(l) != "hardBreak") => {
                    out.push(json!({"type": "hardBreak"}));
                }
                None => {}
            }
        }
        while out.last().is_some_and(|l| type_of(l) == "hardBreak") {
            out.pop();
        }
        if out.is_empty() {
            out.push(json!({"type": "text", "text": "x"}));
        }
        out
    })
}

fn paragraph() -> impl Strategy<Value = Value> {
    inline(true).prop_map(|c| json!({"type": "paragraph", "content": c}))
}

fn leaf_block() -> impl Strategy<Value = Value> {
    prop_oneof![
        6 => paragraph(),
        2 => (1u64..7, inline(false)).prop_map(|(l, c)| json!({"type": "heading", "attrs": {"level": l}, "content": c})),
        1 => (prop::option::of(prop::sample::select(vec!["rust", "js"])), prop::collection::vec("[a-z`~ ]{0,10}", 0..3))
            .prop_map(|(lang, lines)| {
                let text = lines.join("\n");
                let mut n = json!({"type": "codeBlock", "attrs": {"language": lang}});
                if !text.is_empty() { n["content"] = json!([{"type": "text", "text": text}]); }
                n
            }),
        1 => Just(json!({"type": "horizontalRule"})),
        1 => (words(), prop::option::of(words()), prop::sample::select(vec![
                "/api/v1/attachments/0192f0a0-4444-7000-8000-000000000001", "https://example.com/i.png"]))
            .prop_map(|(alt, caption, src)| json!({"type": "image", "attrs": {"src": src, "alt": alt, "caption": caption}})),
        1 => prop::sample::select(UUIDS.to_vec()).prop_map(|id| json!({"type": "boardEmbed", "attrs": {"itemId": id}})),
        1 => (1usize..4, 1usize..4, words()).prop_map(|(rows, cols, w)| {
            let row = |r: usize| json!({"type": "tableRow", "content": (0..cols).map(|c| json!({
                "type": if r == 0 { "tableHeader" } else { "tableCell" },
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": format!("{w}{r}{c}")}]}]
            })).collect::<Vec<_>>()});
            json!({"type": "table", "content": (0..rows).map(row).collect::<Vec<_>>()})
        }),
        1 => Just(json!({"type": "paragraph"})),
    ]
}

fn block() -> impl Strategy<Value = Value> {
    leaf_block().prop_recursive(3, 24, 3, |inner| {
        let items = |name: &'static str| {
            prop::collection::vec(
                (
                    paragraph(),
                    prop::collection::vec(inner.clone(), 0..2),
                    any::<bool>(),
                ),
                1..3,
            )
            .prop_map(move |items| {
                items
                    .into_iter()
                    .map(|(p, rest, checked)| {
                        let mut content = vec![p];
                        content.extend(rest);
                        if name == "taskItem" {
                            json!({"type": name, "attrs": {"checked": checked}, "content": content})
                        } else {
                            json!({"type": name, "content": content})
                        }
                    })
                    .collect::<Vec<_>>()
            })
        };
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..3)
                .prop_map(|c| json!({"type": "blockquote", "content": c})),
            (
                prop::sample::select(vec!["info", "tip", "warning", "danger"]),
                prop::collection::vec(inner.clone(), 1..3)
            )
                .prop_map(
                    |(v, c)| json!({"type": "callout", "attrs": {"variant": v}, "content": c})
                ),
            items("listItem").prop_map(|i| json!({"type": "bulletList", "content": i})),
            (1i64..5, items("listItem")).prop_map(
                |(s, i)| json!({"type": "orderedList", "attrs": {"start": s}, "content": i})
            ),
            items("taskItem").prop_map(|i| json!({"type": "taskList", "content": i})),
        ]
    })
}

fn document() -> impl Strategy<Value = Value> {
    prop::collection::vec(block(), 1..6)
        .prop_map(|c| schema::normalize(&json!({"type": "doc", "content": c})))
}

/// Documents with block ids on every block (as the editor writes them).
fn document_with_ids() -> impl Strategy<Value = Value> {
    document().prop_map(|mut d| {
        let mut n = 0u32;
        let mut stack = vec![&mut d];
        while let Some(x) = stack.pop() {
            if let Some(Value::Object(a)) = x.get_mut("attrs")
                && a.contains_key(schema::BLOCK_ID)
            {
                n += 1;
                a.insert(
                    schema::BLOCK_ID.into(),
                    Value::from(format!("00000000-0000-4000-8000-{n:012}")),
                );
            }
            if let Some(Value::Array(c)) = x.get_mut("content") {
                stack.extend(c.iter_mut());
            }
        }
        d
    })
}

fn config() -> ProptestConfig {
    ProptestConfig {
        // `PROPTEST_CASES` runs more (or fewer) cases.
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(128),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn pm_to_y_to_pm_identity(d in document_with_ids()) {
        let bin = pm::write_doc(&d, 7);
        let back = pm::read(&doc_of(&bin)).unwrap();
        prop_assert_eq!(back, d);
    }

    #[test]
    fn markdown_roundtrip(d in document()) {
        let printed = md_out::print(&d, &NoLookup, None).markdown;
        let (parsed, _) = md_in::parse(&printed, &NoLookup).unwrap();
        let want = md_out::md_normalize(&d, &NoLookup);
        prop_assert!(ydiff::same(&parsed, &want), "markdown:\n{}\nparsed: {}\nwant: {}", printed,
            serde_json::to_string(&parsed).unwrap(), serde_json::to_string(&want).unwrap());
    }

    #[test]
    fn markdown_print_idempotent(d in document()) {
        let once = md_out::print(&d, &NoLookup, None).markdown;
        let (parsed, _) = md_in::parse(&once, &NoLookup).unwrap();
        let twice = md_out::print(&parsed, &NoLookup, None).markdown;
        prop_assert_eq!(twice, once);
    }
}

/// Marks whose delimiters touch a word or punctuation, e-mail addresses and `=` in text: each prints to Markdown
/// that parses back to the same document, and printing that again gives the same Markdown.
#[test]
fn markdown_delimiters_next_to_words_and_punctuation() {
    let t = |text: &str, marks: &[&str]| {
        let mut n = json!({"type": "text", "text": text});
        if !marks.is_empty() {
            n["marks"] = marks.iter().map(|m| json!({"type": m})).collect();
        }
        n
    };
    let link = |text: &str| {
        json!({"type": "text", "text": text, "marks": [{"type": "bold"},
            {"type": "link", "attrs": {"href": "https://example.com/a", "itemId": null}}]})
    };
    let cases: Vec<(Vec<Value>, Option<&str>)> = vec![
        (
            vec![t("x", &[]), t("_y_", &["italic"])],
            Some("&#120;*\\_y\\_*"),
        ),
        (vec![t("a@b.co", &[])], Some("a\\@b.co")),
        // Italic inside bold keeps `*`; inside a `***` run it takes `_`.
        (
            vec![
                t("a ", &["bold"]),
                t("b", &["bold", "italic"]),
                t(" c", &["bold"]),
            ],
            Some("**a *b* c**"),
        ),
        (
            vec![
                t("a", &["bold", "italic"]),
                t("b", &["bold"]),
                t("c", &["bold", "italic"]),
            ],
            Some("***a*&#98;_c_**"),
        ),
        (vec![t("mail a.b+c@d.example now", &[])], None),
        (vec![t("a", &[]), t("(b)", &["bold"]), t("c", &[])], None),
        (
            vec![t("a", &[]), t("b", &["bold", "strike"]), t("c", &[])],
            None,
        ),
        (
            vec![t("a", &[]), t("(b)", &["bold", "strike"]), t("c", &[])],
            None,
        ),
        (
            vec![t("a", &[]), t("b", &["bold", "underline"]), t("c", &[])],
            None,
        ),
        (
            vec![t("a", &["italic"]), t("b", &["bold"]), t("c", &["italic"])],
            None,
        ),
        (
            vec![
                t("a", &["bold", "italic"]),
                t("b", &["italic"]),
                t(".", &["bold"]),
            ],
            None,
        ),
        (
            vec![
                t("x", &["highlight"]),
                t("=y", &[]),
                t("z=", &["highlight"]),
            ],
            None,
        ),
        (vec![t("a=b=c", &[])], None),
        (vec![t("\u{a0}y", &["italic"]), t("z", &[])], None),
        (
            vec![t("😀", &["bold"]), t("a", &[]), t("é", &["strike"])],
            None,
        ),
        (
            vec![
                t("a!", &[]),
                json!({"type": "mention", "attrs": {"itemId": UUIDS[0]}}),
            ],
            None,
        ),
        (
            vec![t("snake_case", &[]), t("_", &["italic"]), t("x", &[])],
            None,
        ),
        // Found by the proptests: a bold reopened after italic closes, italic reopened inside `***`, and a text
        // `_` that would close an italic written with `_`.
        (
            vec![t("0", &["italic"]), t("a ", &["bold", "italic"]), link("0")],
            None,
        ),
        (
            vec![
                t("0", &["bold", "italic"]),
                t("a", &["bold"]),
                t("0", &["bold", "italic"]),
            ],
            None,
        ),
        (
            vec![
                t("A", &["bold"]),
                t("a_", &["italic", "underline"]),
                t("0", &[]),
            ],
            None,
        ),
    ];
    for (content, want) in cases {
        let d = schema::normalize(
            &json!({"type": "doc", "content": [{"type": "paragraph", "content": content}]}),
        );
        let printed = md_out::print(&d, &NoLookup, None).markdown;
        if let Some(want) = want {
            assert_eq!(printed.trim_end(), want);
        }
        let (parsed, _) = md_in::parse(&printed, &NoLookup).unwrap();
        let normal = md_out::md_normalize(&d, &NoLookup);
        assert!(
            ydiff::same(&parsed, &normal),
            "markdown {printed:?}\nparsed {}\nwant {}",
            serde_json::to_string(&parsed).unwrap(),
            serde_json::to_string(&normal).unwrap()
        );
        assert_eq!(md_out::print(&parsed, &NoLookup, None).markdown, printed);
    }
}

// --------------------------------------------------------------------------------------------- Markdown goldens

/// A lookup for the goldens: two documents and a board, and one attachment.
struct GoldenLookup;

impl Lookup for GoldenLookup {
    fn item(&self, r: &ItemRef) -> Option<ItemInfo> {
        let items = [
            (
                "0192f0a0-1111-7000-8000-00000000000a",
                "DocA00000a",
                ItemKind::Doc,
                "Doc A",
            ),
            (
                "0192f0a0-2222-7000-8000-00000000000b",
                "DocB00000b",
                ItemKind::Doc,
                "Doc B",
            ),
            (
                "0192f0a0-3333-7000-8000-00000000000c",
                "Board0000c",
                ItemKind::Board,
                "Board C",
            ),
        ];
        items.iter().find_map(|(id, url_id, kind, title)| {
            let hit = match r {
                ItemRef::Id(x) => x == id,
                ItemRef::UrlId(x) => x == url_id,
            };
            hit.then(|| ItemInfo {
                id: (*id).to_owned(),
                kind: *kind,
                title: (*title).to_owned(),
            })
        })
    }

    fn attachment(&self, id: &str) -> bool {
        id == "0192f0a0-4444-7000-8000-000000000001"
    }

    fn public_origin(&self) -> Option<&str> {
        Some("https://wiki.example.com")
    }
}

fn golden_inputs() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root().join("markdown")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".in.json") {
            out.push((stem.to_owned(), path));
        } else if let Some(stem) = name.strip_suffix(".md")
            && !stem.ends_with(".expected")
        {
            out.push((stem.to_owned(), path));
        }
    }
    out.sort();
    assert!(!out.is_empty(), "fixtures/markdown has no inputs");
    out
}

fn golden(input: &std::path::Path) -> (String, Vec<crate::api::ConversionWarning>) {
    let mut warnings = Warnings::default();
    let doc = if input.extension().is_some_and(|e| e == "json") {
        schema::normalize(&read_json(input))
    } else {
        let md = std::fs::read_to_string(input).unwrap();
        let (doc, w) = md_in::parse(&md, &GoldenLookup).unwrap();
        warnings.extend(w);
        doc
    };
    let printed = md_out::print(&doc, &GoldenLookup, None);
    warnings.extend(printed.warnings);
    (printed.markdown, warnings.into_vec())
}

#[test]
fn markdown_goldens() {
    let mut seen = std::collections::BTreeSet::new();
    for (name, input) in golden_inputs() {
        let (md, warnings) = golden(&input);
        let expected_path = root().join(format!("markdown/{name}.expected.md"));
        let warnings_path = root().join(format!("markdown/{name}.warnings.json"));
        let warnings_json = format!("{}\n", serde_json::to_string_pretty(&warnings).unwrap());
        if writing() {
            std::fs::write(&expected_path, &md).unwrap();
            std::fs::write(&warnings_path, &warnings_json).unwrap();
        }
        assert_eq!(
            md,
            std::fs::read_to_string(&expected_path).unwrap(),
            "{name}.expected.md"
        );
        assert_eq!(
            warnings_json,
            std::fs::read_to_string(&warnings_path).unwrap(),
            "{name}.warnings.json"
        );
        // The printed form is stable: printing it again changes nothing.
        let (again, _) = md_in::parse(&md, &GoldenLookup).unwrap();
        assert_eq!(
            md_out::print(&again, &GoldenLookup, None).markdown,
            md,
            "{name}: print is idempotent"
        );
        for w in warnings {
            seen.insert(w.code.as_str());
        }
    }
    for code in [
        "image_hoisted",
        "marks_in_code_dropped",
        "hard_break_dropped",
        "empty_paragraph_dropped",
        "unsafe_link",
        "task_list_mixed",
        "alert_mapped",
        "html_as_text",
        "unknown_item",
        "embed_not_board",
        "image_width_dropped",
        "table_cell_flattened",
        "table_span_dropped",
        "table_alignment_dropped",
        "unknown_node",
        "unsupported_markdown",
    ] {
        assert!(seen.contains(code), "no golden covers {code}");
    }
}

/// `WRITE_FIXTURES=1`: converts `fixtures/markdown/*.md` into `fixtures/ydoc/server/*.bin` and normalized `*.pm.json`
/// (the web test `editor/serverWritten.test.ts` opens them in TipTap). Otherwise checks they are current.
///
/// A `.bin` is rewritten only when its content changed (same document, same state vector otherwise): yrs writes the
/// keys of a map value (an item link's `{href, itemId}`) in hash-map order, so the bytes of such an update are not
/// stable between runs even though the document is.
#[test]
fn write_server_fixtures() {
    let dir = root().join("ydoc/server");
    if writing() {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, input) in golden_inputs() {
        if input.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let md = std::fs::read_to_string(&input).unwrap();
        let (doc, _) = md_in::parse(&md, &GoldenLookup).unwrap();
        let h: [u8; 32] = sha2::Sha256::digest(name.as_bytes()).into();
        let empty: &[u8] = &[0, 0];
        let out = write(&[empty], &Target::Markdown(doc), &GoldenLookup, &h).unwrap();
        let full = {
            let d = ydiff::load(&[empty, &out.update], None).unwrap();
            d.transact()
                .encode_state_as_update_v1(&StateVector::default())
        };
        let pm_json = format!(
            "{}\n",
            serde_json::to_string_pretty(&schema::sorted(&pm::read(&doc_of(&full)).unwrap()))
                .unwrap()
        );
        let (bin_path, pm_path) = (
            dir.join(format!("{name}.bin")),
            dir.join(format!("{name}.pm.json")),
        );
        let same_doc = |stored: &[u8]| {
            let (a, b) = (doc_of(stored), doc_of(&full));
            pm::read(&a).unwrap() == pm::read(&b).unwrap()
                && a.transact().state_vector() == b.transact().state_vector()
        };
        if writing() {
            if !std::fs::read(&bin_path).is_ok_and(|old| same_doc(&old)) {
                std::fs::write(&bin_path, &full).unwrap();
            }
            std::fs::write(&pm_path, &pm_json).unwrap();
        }
        let stored = std::fs::read(&bin_path).unwrap();
        assert!(
            same_doc(&stored),
            "{name}.bin is out of date (WRITE_FIXTURES=1)"
        );
        assert_eq!(
            std::fs::read_to_string(&pm_path).unwrap(),
            pm_json,
            "{name}.pm.json"
        );
    }
}

use sha2::Digest;

// --------------------------------------------------------------------------------------------- the diff writer

fn h(s: &str) -> [u8; 32] {
    sha2::Sha256::digest(s.as_bytes()).into()
}

fn p(text: &str, id: &str) -> Value {
    json!({"type": "paragraph", "attrs": {"blockId": id}, "content": [{"type": "text", "text": text}]})
}

fn doc(content: Vec<Value>) -> Value {
    schema::normalize(&json!({"type": "doc", "content": content}))
}

/// Writes `target` (normalized) against `base` and returns the new full state, the update and the doc JSON.
fn put(base: &[u8], target: Target, seed: &str) -> (Vec<u8>, Written, Value) {
    let out = write(&[base], &target, &GoldenLookup, &h(seed)).unwrap();
    let d = ydiff::load(&[base, &out.update], None).unwrap();
    let full = d
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let json = pm::read(&d).unwrap();
    (full, out, json)
}

fn md(s: &str) -> Target {
    Target::Markdown(md_in::parse(s, &GoldenLookup).unwrap().0)
}

/// The Y ids (client, clock) of every top-level element, and the text of each.
fn identities(state: &[u8]) -> Vec<(yrs::ID, String)> {
    let d = doc_of(state);
    let frag = pm::fragment(&d);
    let txn = d.transact();
    frag.children(&txn)
        .map(|c| {
            let BranchID::Nested(id) = c.id() else {
                panic!("a nested element")
            };
            let text = match &c {
                XmlOut::Element(e) => e
                    .children(&txn)
                    .filter_map(|t| match t {
                        XmlOut::Text(t) => Some(t.get_string(&txn)),
                        _ => None,
                    })
                    .collect::<String>(),
                _ => String::new(),
            };
            (id, text)
        })
        .collect()
}

use yrs::GetString;

fn base_doc(content: Vec<Value>) -> Vec<u8> {
    pm::write_doc(&doc(content), 99)
}

#[test]
fn diff_keeps_unchanged_blocks_identity() {
    let base = base_doc(vec![
        p("The first paragraph", "b1"),
        p("The second paragraph", "b2"),
        p("The third one", "b3"),
    ]);
    let before = identities(&base);
    let target = md("The first paragraph\n\nThe second paragraph, edited\n\nThe third one\n");
    let (after, out, json) = put(&base, target, "edit-two");
    assert!(out.changed);
    let now = identities(&after);
    assert_eq!(now.len(), 3);
    assert_eq!(now[0], before[0], "P1 untouched");
    assert_eq!(now[2], before[2], "P3 untouched");
    assert_eq!(now[1].0, before[1].0, "P2 is the same Y element");
    assert_eq!(now[1].1, "The second paragraph, edited");
    let ids: Vec<&str> = content_of(&json)
        .iter()
        .map(|b| b["attrs"]["blockId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["b1", "b2", "b3"]);
    // Only P2's text changed: the update holds one client's insertion of ", edited" (8 UTF-16 units) and no deletion.
    let update = Update::decode_v1(&out.update).unwrap();
    let sv = update.state_vector();
    assert_eq!(sv.len(), 1);
    assert_eq!(sv.iter().next().map(|(_, clock)| *clock), Some(8));
}

#[test]
fn diff_insert_top_edit_bottom_keeps_middle_identity() {
    let paras: Vec<Value> = (1..=12)
        .map(|i| p(&format!("Paragraph {i}"), &format!("b{i}")))
        .collect();
    let base = base_doc(paras);
    let before = identities(&base);
    let mut md_text = String::from("Paragraph 1\n\nNew paragraph\n\n");
    for i in 2..=12 {
        if i == 11 {
            md_text.push_str("Paragraph 11, edited\n\n");
        } else {
            md_text.push_str(&format!("Paragraph {i}\n\n"));
        }
    }
    let (after, _, json) = put(&base, md(&md_text), "insert-top");
    let now = identities(&after);
    assert_eq!(now.len(), 13);
    assert_eq!(now[0], before[0]);
    for i in 2..=12 {
        if i == 11 {
            assert_eq!(now[i].0, before[i - 1].0, "P11 is updated in place");
            assert_eq!(now[i].1, "Paragraph 11, edited");
        } else {
            assert_eq!(now[i], before[i - 1], "P{i} untouched");
        }
    }
    assert_eq!(now[1].1, "New paragraph");
    let new_id = content_of(&json)[1]["attrs"]["blockId"].as_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(new_id).is_ok_and(|u| u.get_version_num() == 4),
        "{new_id}"
    );
}

#[test]
fn json_reorder_moves_by_block_id() {
    let base = base_doc(vec![p("Alpha", "a"), p("Beta", "b"), p("Gamma", "g")]);
    let before = identities(&base);
    let target = doc(vec![p("Gamma", "g"), p("Alpha", "a"), p("Beta", "b")]);
    let (after, _, json) = put(&base, Target::Json(target.clone()), "reorder");
    assert_eq!(json, target, "same content, same block ids, new order");
    let now = identities(&after);
    assert_eq!(now[1], before[0], "Alpha untouched");
    assert_eq!(now[2], before[1], "Beta untouched");
    assert_ne!(
        now[0].0, before[2].0,
        "Gamma was moved: deleted and re-inserted"
    );
}

#[test]
fn markdown_put_leaves_unchanged_blocks_untouched() {
    let image = json!({"type": "image", "attrs": {"blockId": "img", "src": "/api/v1/attachments/0192f0a0-4444-7000-8000-000000000001",
        "attachmentId": "0192f0a0-4444-7000-8000-000000000001", "width": 50, "alt": "Diagram"}});
    let cell = |t: &str, attrs: Value| json!({"type": "tableCell", "attrs": attrs, "content": [p(t, &format!("c{t}"))]});
    let table = json!({"type": "table", "attrs": {"blockId": "t"}, "content": [
        {"type": "tableRow", "attrs": {"blockId": "r0"}, "content": [
            {"type": "tableHeader", "attrs": {"blockId": "h1"}, "content": [p("A", "ha")]},
            {"type": "tableHeader", "attrs": {"blockId": "h2"}, "content": [p("B", "hb")]}]},
        {"type": "tableRow", "attrs": {"blockId": "r1"}, "content": [cell("wide", json!({"blockId": "cw", "colspan": 2}))]},
    ]});
    let base_json = doc(vec![
        p("Intro", "p1"),
        image.clone(),
        table.clone(),
        p("Outro", "p2"),
    ]);
    let base = pm::write_doc(&base_json, 5);
    let printed = md_out::print(&base_json, &GoldenLookup, None).markdown;
    let edited = printed.replace("Outro", "Outro, edited");
    assert_ne!(edited, printed);
    let (after, out, json) = put(&base, md(&edited), "edit-outro");
    assert!(out.changed);
    let c = content_of(&json);
    assert_eq!(c[1], schema::normalize(&image), "the image keeps its width");
    assert_eq!(
        c[2],
        schema::normalize(&table),
        "the table keeps its colspan"
    );
    assert_eq!(content_of(&c[3])[0]["text"], "Outro, edited");
    let (before, now) = (identities(&base), identities(&after));
    assert_eq!(now[..3], before[..3]);
}

#[test]
fn repeated_put_is_byte_identical_update() {
    let base = base_doc(vec![p("Hello", "b1"), p("World", "b2")]);
    let target = md("Hello there\n\n**World**\n\n- new\n- list\n");
    let a = write(&[&base], &target, &GoldenLookup, &h("same request")).unwrap();
    let b = write(&[&base], &target, &GoldenLookup, &h("same request")).unwrap();
    assert_eq!(a.client_id, b.client_id);
    assert_eq!(a.update, b.update);
    // Applying it twice adds nothing the second time.
    let d = ydiff::load(&[&base, &a.update], None).unwrap();
    let sv = d.transact().state_vector();
    d.transact_mut()
        .apply_update(Update::decode_v1(&b.update).unwrap())
        .unwrap();
    assert_eq!(d.transact().state_vector(), sv);
}

#[test]
fn new_blocks_get_uuid_block_ids() {
    let base = base_doc(vec![p("Keep", "k")]);
    let (_, _, json) = put(
        &base,
        md("Keep\n\n- one\n- two\n\n> quoted\n"),
        "new-blocks",
    );
    let mut ids = Vec::new();
    let mut stack = vec![&json];
    while let Some(n) = stack.pop() {
        if let Some(id) = n.get("attrs").and_then(|a| a.get("blockId")) {
            ids.push((type_of(n).to_owned(), id.clone()));
        }
        stack.extend(content_of(n).iter());
    }
    assert!(ids.len() >= 7, "{ids:?}");
    for (name, id) in &ids {
        let s = id
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no blockId"));
        if s != "k" {
            assert_eq!(
                uuid::Uuid::parse_str(s).unwrap().get_version_num(),
                4,
                "{name}: {s}"
            );
        }
    }
    let unique: std::collections::HashSet<_> = ids.iter().map(|(_, i)| i.to_string()).collect();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn utf16_offsets_with_emoji_and_combining_marks() {
    let base = base_doc(vec![p("a😀é", "b1")]);
    let (after, _, json) = put(&base, Target::Json(doc(vec![p("a😀xé", "b1")])), "emoji");
    assert_eq!(content_of(&content_of(&json)[0])[0]["text"], "a😀xé");
    // The insertion landed at UTF-16 offset 3 (after "a" and the surrogate pair): Yjs reads the same text.
    let d = doc_of(&after);
    let frag = pm::fragment(&d);
    let txn = d.transact();
    let Some(XmlOut::Element(e)) = frag.get(&txn, 0) else {
        panic!()
    };
    let Some(XmlOut::Text(t)) = e.get(&txn, 0) else {
        panic!()
    };
    assert_eq!(t.get_string(&txn), "a😀xé");
    let base2 = base_doc(vec![p("e\u{301}😀", "b1")]);
    let (_, _, json) = put(
        &base2,
        Target::Json(doc(vec![p("e\u{301}\u{301}😀!", "b1")])),
        "combining",
    );
    assert_eq!(
        content_of(&content_of(&json)[0])[0]["text"],
        "e\u{301}\u{301}😀!"
    );
    // Changing only the low surrogate of an emoji replaces the whole pair.
    let base3 = base_doc(vec![p("x😀y", "b1")]);
    let (_, _, json) = put(&base3, Target::Json(doc(vec![p("x😁y", "b1")])), "pair");
    assert_eq!(content_of(&content_of(&json)[0])[0]["text"], "x😁y");
}

#[test]
fn invalid_json_422_with_pointer() {
    let check = |d: Value| schema::validate(&d, "/doc", link_allowed, image_allowed).unwrap_err();
    let para = |c: Value| json!({"type": "doc", "content": [{"type": "paragraph", "content": c}]});
    assert_eq!(
        check(json!({"type": "doc", "content": [{"type": "marquee"}]})).path,
        "/doc/content/0/type"
    );
    assert_eq!(
        check(json!({"type": "doc", "content": [{"type": "bulletList", "content": [{"type": "paragraph"}]}]})).path,
        "/doc/content/0/content/0"
    );
    assert_eq!(
        check(json!({"type": "doc", "content": [{"type": "heading", "attrs": {"level": "one"}}]}))
            .path,
        "/doc/content/0/attrs/level"
    );
    assert_eq!(
        check(para(
            json!([{"type": "text", "text": "x", "marks": [{"type": "bold"}, {"type": "code"}]}])
        ))
        .path,
        "/doc/content/0/content/0/marks/1"
    );
    assert_eq!(
        check(para(json!([{"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]}]))).path,
        "/doc/content/0/content/0/marks/0/attrs/href"
    );
    let mut deep = json!({"type": "paragraph"});
    for _ in 0..101 {
        deep = json!({"type": "blockquote", "content": [deep]});
    }
    let e = check(json!({"type": "doc", "content": [deep]}));
    assert!(e.reason.contains("deeper"), "{e:?}");
    assert!(e.path.starts_with("/doc/content/0/content/0"));
}

#[test]
fn markdown_prescan_rejects_depth_101() {
    let line = ">".repeat(1024 * 1024 - 2);
    let e = md_in::parse(&format!("{line} x\n"), &NoLookup).unwrap_err();
    assert_eq!(e.path, "1:101");
    let ok = format!("{} x\n", "> ".repeat(100));
    assert!(md_in::parse(&ok, &NoLookup).is_ok());
    let e = md_in::parse(&format!("{} x\n", "> ".repeat(101)), &NoLookup).unwrap_err();
    assert_eq!(e.path, "1:201", "the 101st quote, found in comrak's tree");
    let lists: String = (0..101)
        .map(|i| format!("{}- x\n", "  ".repeat(i)))
        .collect();
    let e = md_in::parse(&lists, &NoLookup).unwrap_err();
    assert_eq!(e.path, "101:201");
    // comrak opens at most 100 containers on one line, so 101 takes a second line.
    let mixed = format!("- x\n  {}x\n", "- > ".repeat(50));
    assert_eq!(md_in::parse(&mixed, &NoLookup).unwrap_err().path, "2:201");
}

/// Depth 101 refused, whatever spreads it over lines; and the parse never overflows the stack on the way
/// (these inputs once reached comrak and the recursive block conversion thousands of levels deep).
#[test]
fn markdown_depth_bounded_across_lines() {
    fn refused(name: &str, md: &str) {
        let start = std::time::Instant::now();
        let e = md_in::parse(md, &NoLookup).expect_err(name);
        assert!(e.reason.contains("deeper"), "{name}: {e:?}");
        assert!(
            start.elapsed().as_secs() < 10,
            "{name}: {:?}",
            start.elapsed()
        );
    }
    // Several markers on a line, the next lines indented to its innermost item (the review's input shape).
    let mut md = String::new();
    let mut markers = 0;
    for k in 1..=30 {
        md.push_str(&" ".repeat(2 * markers));
        md.push_str(&"- ".repeat(100 - k));
        md.push_str("x\n");
        markers += 100 - k;
    }
    refused("markers per line", &md);
    // The same nesting by tabs (a tab reaches the next multiple of four columns).
    let tabs: String = (0..300)
        .map(|i| format!("{}- x\n", "\t".repeat(i)))
        .collect();
    refused("tabs", &tabs);
    // Items continued inside a quote whose prefix is narrower than the line that opened them.
    let quoted: String = (0..300)
        .map(|i| format!("> {}- x\n", "  ".repeat(i)))
        .collect();
    refused("quoted", &quoted);
    // Lazy continuation lines between the items keep them open.
    let lazy: String = (0..300)
        .map(|i| format!("{}- x\nlazy\n", "  ".repeat(i)))
        .collect();
    refused("lazy", &lazy);
    // CR line endings, alone or with LF.
    let cr: String = (0..300)
        .map(|i| format!("{}- x\r", "  ".repeat(i)))
        .collect();
    refused("cr", &cr);
    refused("crlf", &cr.replace('\r', "\r\n"));
    // Empty items nest as well.
    let empty: String = (0..300).map(|i| format!("{}-\n", "  ".repeat(i))).collect();
    refused("empty items", &empty);
    // Exactly 100 levels pass, also with lists indented by four columns per level.
    let mut md = format!("{}x\n", "- ".repeat(50));
    md.push_str(&format!("{}{}x\n", " ".repeat(100), "- ".repeat(50)));
    let (doc, _) = md_in::parse(&md, &NoLookup).unwrap();
    assert_eq!(
        serde_json::to_string(&doc)
            .unwrap()
            .matches("bulletList")
            .count(),
        100
    );
    let four: String = (0..100)
        .map(|i| format!("{}- x\n", "    ".repeat(i)))
        .collect();
    assert!(md_in::parse(&four, &NoLookup).is_ok());
    let deeper: String = (0..101)
        .map(|i| format!("{}- x\n", "    ".repeat(i)))
        .collect();
    assert_eq!(
        md_in::parse(&deeper, &NoLookup).unwrap_err().path,
        "101:401"
    );
}

#[test]
fn adversarial_markdown_bounded() {
    let start = std::time::Instant::now();
    for input in [
        format!("{}a{}", "*".repeat(50_000), "*".repeat(50_000)),
        "*a ".repeat(50_000),
        "_a **b ".repeat(30_000),
        format!("{}x{}", "[".repeat(20_000), "]".repeat(20_000)),
        "`".repeat(100_000),
        "<u>".repeat(30_000),
    ] {
        let (d, _) = md_in::parse(&input, &NoLookup).unwrap();
        let _ = md_out::print(&d, &NoLookup, None);
    }
    // Many marks whose delimiters need character references: the printer's run fixes stay linear.
    let content: Vec<Value> = (0..20_000)
        .flat_map(|i| {
            [
                json!({"type": "text", "text": format!("x{i}")}),
                json!({"type": "text", "text": "(y)", "marks": [{"type": "italic"}]}),
            ]
        })
        .collect();
    let d = schema::normalize(
        &json!({"type": "doc", "content": [{"type": "paragraph", "content": content}]}),
    );
    let printed = md_out::print(&d, &NoLookup, None).markdown;
    let (parsed, _) = md_in::parse(&printed, &NoLookup).unwrap();
    assert!(ydiff::same(&parsed, &md_out::md_normalize(&d, &NoLookup)));
    assert!(start.elapsed().as_secs() < 30, "took {:?}", start.elapsed());
}

#[test]
fn title_from_h1_and_include() {
    let mut d = md_in::parse("# The title\n\nBody\n", &NoLookup).unwrap().0;
    assert_eq!(take_h1(&mut d).as_deref(), Some("The title"));
    assert_eq!(content_of(&d).len(), 1);
    let printed = md_out::print(&d, &NoLookup, Some("A *title*"));
    assert_eq!(printed.markdown, "# A \\*title\\*\n\nBody\n");
}

#[test]
fn allow_list() {
    for ok in [
        "https://x.y/",
        "http://x",
        "mailto:a@b",
        "quire://item/x",
        "/doc/a",
        "#h-1",
    ] {
        assert!(link_allowed(ok), "{ok}");
    }
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:x",
        "java\tscript:x",
        "data:text/html,x",
        "//evil.com",
        "/\\evil.com",
        "/\t/evil.com",
        "/\n/evil.com",
        "/\r/evil.com",
        " //evil.com",
        "/ /evil.com",
        "vbscript:x",
        "",
        "\t",
        "x:y",
    ] {
        assert!(!link_allowed(bad), "{bad:?}");
        assert!(!image_allowed(bad), "{bad:?}");
    }
    assert!(!image_allowed("mailto:a@b"));
    assert!(!image_allowed("data:image/png;base64,AAAA"));
    assert_eq!(
        item_ref("/doc/some-title-AbCdEfGh12", None),
        Some(ItemRef::UrlId("AbCdEfGh12".into()))
    );
    assert_eq!(
        item_ref(
            "https://wiki.example.com/board/AbCdEfGh12",
            Some("https://wiki.example.com")
        ),
        Some(ItemRef::UrlId("AbCdEfGh12".into()))
    );
    assert_eq!(
        item_ref(
            "https://other.example.com/doc/x-AbCdEfGh12",
            Some("https://wiki.example.com")
        ),
        None
    );
    assert_eq!(
        attachment_ref("/api/v1/attachments/abc-1", None).as_deref(),
        Some("abc-1")
    );
}

#[test]
fn yjs_marks_are_read_after_y_tiptap() {
    // A mark stored under an overlapping-mark key reads as the mark.
    let d = pm::new_doc(Some(3));
    let frag = pm::fragment(&d);
    {
        let mut txn = d.transact_mut();
        let p = frag.insert(&mut txn, 0, yrs::XmlElementPrelim::empty("paragraph"));
        let t: yrs::XmlTextRef = p.insert(&mut txn, 0, yrs::XmlTextPrelim::new(""));
        t.insert(&mut txn, 0, "hi");
        let mut a = yrs::types::Attrs::new();
        a.insert("bold--AbCd1234".into(), Any::Map(Default::default()));
        t.format(&mut txn, 0, 2, a);
    }
    let json = pm::read(&d).unwrap();
    assert_eq!(
        json["content"][0]["content"][0]["marks"],
        json!([{"type": "bold"}])
    );
}

#[test]
fn empty_editor_paragraph_is_reused() {
    // The editor writes one empty paragraph into a new document; a first write fills it instead of adding another.
    let base = base_doc(vec![
        json!({"type": "paragraph", "attrs": {"blockId": "e"}}),
    ]);
    for target in [
        md("Hello there\n"),
        Target::Json(doc(vec![
            json!({"type": "paragraph", "content": [{"type": "text", "text": "Hello there"}]}),
        ])),
    ] {
        let (_, _, json) = put(&base, target, "first-write");
        let c = content_of(&json);
        assert_eq!(c.len(), 1, "{json}");
        assert_eq!(c[0]["attrs"]["blockId"], "e");
        assert_eq!(content_of(&c[0])[0]["text"], "Hello there");
    }
}
