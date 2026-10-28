//! Quire Markdown 1 output: Quire's deterministic printer and its block map (owner: W4)
//!
//! Printing is two steps:
//! 1. [`lower`] applies exactly the lossy rules of the §6.4 table to a normalized document, giving the tree the
//!    parser will read back ([`md_normalize`] is that tree without the block map). Every lowered node that stands
//!    for one base node carries the base child index under [`ORIGIN`]; a node the printer made up (a table filler
//!    cell, an added header row, the paragraph of a flattened cell) has none, and lists the base children it stands
//!    for under [`CONSUMED`]. This is the block map the write path's Markdown comparison uses (§6.4 step 3).
//! 2. [`render`] writes a lowered tree as Markdown: ATX headings; `-` bullets (`*` after another bullet list); `1.`
//!    ordered lists from `start` (`1)` after another ordered list); fences longer than any backtick run inside;
//!    `---` rules; backslash hard breaks; one blank line between blocks; LF endings; one trailing newline;
//!    backslash escapes for punctuation that would start a construct.
//!
//! `convert::tests::markdown_roundtrip` checks `parse(print(d)) == md_normalize(d)` (ignoring `blockId`).
//! The printer recurses over containers (at most [`MAX_PRINT_DEPTH`] levels; deeper content is printed as the text of
//! its blocks).

use serde_json::{Map, Value, json};

use crate::api::{ConversionWarningCode as W, ItemKind};
use crate::convert::{
    ItemRef, Lookup, Warnings, attachment_url, image_allowed, item_ref, link_allowed,
    quire_item_id, quire_item_url,
};
use crate::schema::{self, BLOCK_ID, content_of, push_inline, type_of, v1};

/// The block map: the base child index a lowered node stands for.
pub const ORIGIN: &str = "@o";
/// The block map: the base child indexes a made-up node stands for.
pub const CONSUMED: &str = "@c";
/// Marks a table cell the printer added to expand a span (it stands for nothing).
pub const FILLER: &str = "@f";

/// Containers deeper than this are printed flat.
pub const MAX_PRINT_DEPTH: usize = 200;

/// Delimiter marks, in the order they open (they are closed in reverse).
const DELIMITED: &[&str] = &["bold", "italic", "strike", "underline", "highlight"];

/// The callout variants and their GitHub alert names.
pub const ALERTS: &[(&str, &str)] = &[
    ("info", "NOTE"),
    ("tip", "TIP"),
    ("warning", "WARNING"),
    ("danger", "CAUTION"),
];

/// The printed Markdown and the warnings of the lossy steps.
#[derive(Clone, Debug)]
pub struct Printed {
    pub markdown: String,
    pub warnings: Warnings,
}

/// Prints a normalized document (`title`: `?title=include`, prepended as `# <title>` and a blank line).
pub fn print(doc: &Value, lookup: &dyn Lookup, title: Option<&str>) -> Printed {
    let mut warnings = Warnings::default();
    let lowered = lower(doc, lookup, &mut warnings);
    let mut markdown = String::new();
    if let Some(title) = title {
        let heading = json!({"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": title}]});
        let heading = lower_inline_node(&heading, lookup, &mut Warnings::default());
        markdown.push_str(&render_blocks(&[heading], lookup, 0).join("\n"));
        markdown.push_str("\n\n");
    }
    let body = render(&lowered, lookup);
    if title.is_some() && body == "\n" {
        markdown.truncate(markdown.len() - 1);
    } else {
        markdown.push_str(&body);
    }
    Printed { markdown, warnings }
}

/// The lossy rules of §6.4 applied in JSON space: what `parse(print(doc))` reads (ignoring `blockId`).
pub fn md_normalize(doc: &Value, lookup: &dyn Lookup) -> Value {
    let mut lowered = lower(doc, lookup, &mut Warnings::default());
    strip_map(&mut lowered);
    lowered
}

/// Removes the block-map keys from a lowered tree.
pub fn strip_map(node: &mut Value) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let Some(o) = n.as_object_mut() {
            o.remove(ORIGIN);
            o.remove(CONSUMED);
            o.remove(FILLER);
            if let Some(Value::Array(c)) = o.get_mut("content") {
                stack.extend(c.iter_mut());
            }
        }
    }
}

fn attrs_of(node: &Value) -> Option<&Map<String, Value>> {
    node.get("attrs").and_then(Value::as_object)
}

fn str_attr<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    attrs_of(node)?.get(name)?.as_str()
}

/// A node `{type, attrs: defaults + given, content}`.
fn make(name: &str, attrs: &[(&str, Value)], content: Vec<Value>) -> Value {
    let nt = v1().node(name).expect("a v1 node");
    let mut a = schema::normal_attrs(nt.attrs, None);
    for (k, v) in attrs {
        a.insert((*k).to_owned(), v.clone());
    }
    let mut o = Map::new();
    o.insert("type".into(), Value::from(name));
    if !nt.attrs.is_empty() {
        o.insert("attrs".into(), Value::Object(a));
    }
    if !content.is_empty() {
        o.insert("content".into(), Value::Array(content));
    }
    Value::Object(o)
}

fn with_origin(mut node: Value, origin: usize, base: &Value) -> Value {
    if let Some(o) = node.as_object_mut() {
        o.insert(ORIGIN.into(), Value::from(origin));
        if let Some(id) = str_attr(base, BLOCK_ID)
            && let Some(Value::Object(a)) = o.get_mut("attrs")
        {
            a.insert(BLOCK_ID.into(), Value::from(id));
        }
    }
    node
}

fn empty_paragraph() -> Value {
    make("paragraph", &[], vec![])
}

fn is_empty_paragraph(n: &Value) -> bool {
    type_of(n) == "paragraph" && content_of(n).is_empty()
}

/// Lowers a normalized document (module docs).
pub fn lower(doc: &Value, lookup: &dyn Lookup, warnings: &mut Warnings) -> Value {
    // A document of empty paragraphs (a new one the editor opened) prints as nothing and reads back as one empty
    // paragraph: the first stands for it.
    let children = content_of(doc);
    let container = if !children.is_empty() && children.iter().all(is_empty_paragraph) {
        Container::Box
    } else {
        Container::Doc
    };
    let mut content = lower_blocks(children, lookup, warnings, container, 0);
    if content.is_empty() {
        content.push(empty_paragraph());
    }
    let mut o = Map::new();
    o.insert("type".into(), Value::from("doc"));
    o.insert("content".into(), Value::Array(content));
    Value::Object(o)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Doc,
    /// blockquote, callout, table cell: needs at least one block.
    Box,
    /// listItem, taskItem: the first paragraph is the marker line and always stays.
    Item,
}

fn lower_blocks(
    children: &[Value],
    lookup: &dyn Lookup,
    w: &mut Warnings,
    container: Container,
    depth: usize,
) -> Vec<Value> {
    let mut out = Vec::new();
    // A box holding only empty paragraphs prints as an empty box, which reads back as one empty paragraph.
    let empty_box = container == Container::Box
        && !children.is_empty()
        && children.iter().all(is_empty_paragraph);
    for (i, child) in children.iter().enumerate() {
        let first_item_paragraph =
            (container == Container::Item || empty_box) && i == 0 && type_of(child) == "paragraph";
        let mut produced = lower_block(child, lookup, w, depth);
        if !first_item_paragraph {
            let before = produced.len();
            produced.retain(|n| !is_empty_paragraph(n));
            if produced.len() < before && type_of(child) == "paragraph" && !empty_box {
                w.add(W::EmptyParagraphDropped, None);
            }
        } else if produced.is_empty() {
            produced.push(empty_paragraph());
        }
        let one_to_one = produced.len() == 1
            && type_of(&produced[0]) == type_of(child)
            && produced[0].get(CONSUMED).is_none();
        for n in produced {
            if one_to_one {
                out.push(with_origin(n, i, child));
            } else {
                let mut n = n;
                let o = n.as_object_mut().expect("object");
                o.remove(ORIGIN);
                o.entry(CONSUMED).or_insert_with(|| json!([i]));
                out.push(n);
            }
        }
    }
    if container == Container::Box && out.is_empty() {
        out.push(empty_paragraph());
    }
    out
}

/// One base block → its lowered blocks (none, one, or several).
fn lower_block(node: &Value, lookup: &dyn Lookup, w: &mut Warnings, depth: usize) -> Vec<Value> {
    let name = type_of(node);
    if v1().node(name).is_none() {
        w.add(W::UnknownNode, None);
        return vec![];
    }
    if depth >= MAX_PRINT_DEPTH && !v1().node(name).is_some_and(|n| n.is_textblock()) {
        // Too deep: the text of its blocks, as paragraphs.
        return flat_paragraphs(node, lookup, w);
    }
    match name {
        "paragraph" | "heading" => vec![lower_inline_node(node, lookup, w)],
        "codeBlock" => {
            let lang = str_attr(node, "language")
                .and_then(|l| l.split_whitespace().next())
                .filter(|l| !l.is_empty())
                .map(|l| Value::from(l.to_owned()))
                .unwrap_or(Value::Null);
            let text: String = content_of(node)
                .iter()
                .filter_map(|t| t.get("text").and_then(Value::as_str))
                .collect::<String>()
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let content = if text.is_empty() {
                vec![]
            } else {
                vec![json!({"type": "text", "text": text})]
            };
            vec![make("codeBlock", &[("language", lang)], content)]
        }
        "horizontalRule" => vec![make("horizontalRule", &[], vec![])],
        "image" => lower_image(node, lookup, w),
        "boardEmbed" => {
            let Some(id) = str_attr(node, "itemId") else {
                return vec![];
            };
            match lookup.item(&ItemRef::Id(id.to_owned())) {
                Some(info) if info.kind == ItemKind::Doc => {
                    w.add(W::EmbedNotBoard, None);
                    vec![make(
                        "paragraph",
                        &[],
                        vec![json!({"type": "mention", "attrs": {"itemId": info.id}})],
                    )]
                }
                _ => vec![make("boardEmbed", &[("itemId", Value::from(id))], vec![])],
            }
        }
        "blockquote" => {
            let content = lower_children(node, lookup, w, Container::Box, depth);
            vec![make("blockquote", &[], content)]
        }
        "callout" => {
            let variant = str_attr(node, "variant")
                .filter(|v| ALERTS.iter().any(|(n, _)| n == v))
                .unwrap_or("info");
            let content = lower_children(node, lookup, w, Container::Box, depth);
            vec![make(
                "callout",
                &[("variant", Value::from(variant))],
                content,
            )]
        }
        "bulletList" | "orderedList" | "taskList" => {
            let item_name = if name == "taskList" {
                "taskItem"
            } else {
                "listItem"
            };
            let mut items = Vec::new();
            for (i, item) in content_of(node).iter().enumerate() {
                let mut content = lower_children(item, lookup, w, Container::Item, depth + 1);
                if content.is_empty() || type_of(&content[0]) != "paragraph" {
                    content.insert(0, empty_paragraph());
                }
                let attrs: Vec<(&str, Value)> = if item_name == "taskItem" {
                    let checked = attrs_of(item)
                        .and_then(|a| a.get("checked"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    vec![("checked", Value::Bool(checked))]
                } else {
                    vec![]
                };
                items.push(with_origin(make(item_name, &attrs, content), i, item));
            }
            let attrs: Vec<(&str, Value)> = if name == "orderedList" {
                let start = attrs_of(node)
                    .and_then(|a| a.get("start"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1)
                    .clamp(0, 999_999_999);
                vec![("start", Value::from(start))]
            } else {
                vec![]
            };
            if items.is_empty() {
                return vec![];
            }
            vec![make(name, &attrs, items)]
        }
        "table" => lower_table(node, lookup, w),
        // listItem, taskItem, tableRow and cells outside their parents are not schema-valid; print their blocks.
        _ => lower_blocks(content_of(node), lookup, w, Container::Doc, depth + 1),
    }
}

fn lower_children(
    node: &Value,
    lookup: &dyn Lookup,
    w: &mut Warnings,
    container: Container,
    depth: usize,
) -> Vec<Value> {
    lower_blocks(content_of(node), lookup, w, container, depth + 1)
}

fn flat_paragraphs(node: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Vec<Value> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        let nt = v1().node(type_of(n));
        if nt.is_some_and(|t| t.is_textblock()) {
            let p = make("paragraph", &[], content_of(n).to_vec());
            let p = lower_inline_node(&p, lookup, w);
            if !content_of(&p).is_empty() {
                out.push(p);
            }
        } else {
            stack.extend(content_of(n).iter().rev());
        }
    }
    out
}

fn lower_image(node: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Vec<Value> {
    if attrs_of(node)
        .and_then(|a| a.get("width"))
        .is_some_and(|v| !v.is_null())
    {
        w.add(W::ImageWidthDropped, None);
    }
    let clean = |s: Option<&str>| {
        s.map(|s| s.replace(['\n', '\r'], " ").trim().to_owned())
            .filter(|s| !s.is_empty())
            .map(Value::from)
            .unwrap_or(Value::Null)
    };
    let alt = clean(str_attr(node, "alt"));
    let caption = clean(str_attr(node, "caption"));
    let (src, attachment) = match str_attr(node, "attachmentId") {
        Some(id) => {
            let known = lookup.attachment(id);
            (attachment_url(id), known.then(|| id.to_owned()))
        }
        None => match str_attr(node, "src") {
            Some(src) if image_allowed(src) => {
                // A src that names an item or an attachment is read back as one.
                if let Some(r) = item_ref(src, lookup.public_origin()) {
                    return lower_image_item(src, &r, &alt, lookup, w);
                }
                let att = crate::convert::attachment_ref(src, lookup.public_origin())
                    .filter(|a| lookup.attachment(a));
                (src.to_owned(), att)
            }
            Some(_) => {
                w.add(W::UnsafeLink, None);
                return match alt.as_str() {
                    Some(text) => {
                        let p = make(
                            "paragraph",
                            &[],
                            vec![json!({"type": "text", "text": text})],
                        );
                        vec![lower_inline_node(&p, lookup, w)]
                    }
                    None => vec![],
                };
            }
            None => return vec![],
        },
    };
    let src = match &attachment {
        Some(id) => attachment_url(id),
        None => src,
    };
    vec![make(
        "image",
        &[
            ("src", Value::from(src)),
            ("attachmentId", attachment.map_or(Value::Null, Value::from)),
            ("alt", alt),
            ("caption", caption),
        ],
        vec![],
    )]
}

/// An external image whose URL names an item: a board embed, or a mention of a document.
fn lower_image_item(
    src: &str,
    r: &ItemRef,
    alt: &Value,
    lookup: &dyn Lookup,
    w: &mut Warnings,
) -> Vec<Value> {
    match (lookup.item(r), r) {
        (Some(info), _) if info.kind == ItemKind::Board => {
            vec![make(
                "boardEmbed",
                &[("itemId", Value::from(info.id))],
                vec![],
            )]
        }
        (Some(info), _) => {
            w.add(W::EmbedNotBoard, None);
            vec![make(
                "paragraph",
                &[],
                vec![json!({"type": "mention", "attrs": {"itemId": info.id}})],
            )]
        }
        (None, ItemRef::Id(id)) => vec![make(
            "boardEmbed",
            &[("itemId", Value::from(id.clone()))],
            vec![],
        )],
        (None, ItemRef::UrlId(_)) => vec![make(
            "image",
            &[("src", Value::from(src)), ("alt", alt.clone())],
            vec![],
        )],
    }
}

/// One table → a rectangular GFM table: a header row of `tableHeader` cells (an empty one added when the first row
/// has none), body rows of `tableCell`s, spans expanded into filler cells, every cell one paragraph.
fn lower_table(node: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Vec<Value> {
    let rows = content_of(node);
    if rows.is_empty() {
        return vec![];
    }
    // The grid of base cells: (row, base cell index) or a filler.
    let mut grid: Vec<Vec<Option<(usize, usize)>>> = vec![Vec::new(); rows.len()];
    let mut span_dropped = false;
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0;
        for (c, cell) in content_of(row).iter().enumerate() {
            while grid[r].get(col).is_some_and(|x| x.is_none()) {
                col += 1;
            }
            let span = |name: &str| {
                attrs_of(cell)
                    .and_then(|a| a.get(name))
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 1000) as usize
            };
            let (colspan, rowspan) = (span("colspan"), span("rowspan"));
            if colspan > 1 || rowspan > 1 {
                span_dropped = true;
            }
            for dr in 0..rowspan.min(rows.len() - r) {
                let line = &mut grid[r + dr];
                for dc in 0..colspan {
                    let at = col + dc;
                    if line.len() <= at {
                        line.resize(at + 1, None);
                    }
                    if dr == 0 && dc == 0 {
                        line[at] = Some((r, c));
                    }
                }
            }
            col += colspan;
        }
    }
    if span_dropped {
        w.add(W::TableSpanDropped, None);
    }
    let width = grid.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let first_has_header = content_of(&rows[0])
        .iter()
        .any(|c| type_of(c) == "tableHeader");
    let mut out_rows = Vec::new();
    if !first_has_header {
        let cells = (0..width).map(|_| filler_cell("tableHeader")).collect();
        let mut row = make("tableRow", &[], cells);
        row.as_object_mut()
            .expect("object")
            .insert(CONSUMED.into(), json!([]));
        out_rows.push(row);
    }
    for (r, line) in grid.iter().enumerate() {
        let header = r == 0 && first_has_header;
        let cell_name = if header { "tableHeader" } else { "tableCell" };
        let base_row = &rows[r];
        let mut cells = Vec::new();
        for col in 0..width {
            match line.get(col).copied().flatten() {
                Some((_, c)) => {
                    let cell = &content_of(base_row)[c];
                    let para = lower_cell_content(cell, lookup, w);
                    let lowered = make(cell_name, &[], vec![para]);
                    cells.push(with_origin(lowered, c, cell));
                }
                None => cells.push(filler_cell(cell_name)),
            }
        }
        out_rows.push(with_origin(make("tableRow", &[], cells), r, base_row));
    }
    vec![make("table", &[], out_rows)]
}

fn filler_cell(name: &str) -> Value {
    let mut cell = make(name, &[], vec![empty_paragraph()]);
    cell.as_object_mut()
        .expect("object")
        .insert(FILLER.into(), Value::Bool(true));
    cell
}

/// A cell's content as one paragraph: its single paragraph, or its blocks' inline text joined by hard breaks.
fn lower_cell_content(cell: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Value {
    let blocks = content_of(cell);
    if let [only] = blocks
        && type_of(only) == "paragraph"
    {
        let p = lower_inline_cell(only, lookup, w);
        return with_origin(p, 0, only);
    }
    if !(blocks.is_empty() || blocks.iter().all(is_empty_paragraph)) {
        w.add(W::TableCellFlattened, None);
    }
    let mut inline: Vec<Value> = Vec::new();
    let mut stack: Vec<&Value> = blocks.iter().rev().collect();
    while let Some(n) = stack.pop() {
        if v1().node(type_of(n)).is_some_and(|t| t.is_textblock()) {
            let content = content_of(n);
            if content.is_empty() {
                continue;
            }
            if !inline.is_empty() {
                inline.push(json!({"type": "hardBreak"}));
            }
            if type_of(n) == "codeBlock" {
                for t in content {
                    let text = t.get("text").and_then(Value::as_str).unwrap_or("");
                    for (i, line) in text.split('\n').enumerate() {
                        if i > 0 {
                            inline.push(json!({"type": "hardBreak"}));
                        }
                        if !line.is_empty() {
                            inline.push(json!({"type": "text", "text": line}));
                        }
                    }
                }
            } else {
                inline.extend(content.iter().cloned());
            }
        } else {
            stack.extend(content_of(n).iter().rev());
        }
    }
    let p = make("paragraph", &[], inline);
    let mut p = lower_inline_cell(&p, lookup, w);
    p.as_object_mut().expect("object").insert(
        CONSUMED.into(),
        Value::Array((0..blocks.len()).map(Value::from).collect()),
    );
    p
}

fn lower_inline_cell(p: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Value {
    let content = lower_inline(content_of(p), lookup, w, InlineCtx::Cell);
    make("paragraph", &[], content)
}

/// A paragraph or heading with its inline content lowered.
fn lower_inline_node(node: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Value {
    let name = type_of(node);
    let ctx = if name == "heading" {
        InlineCtx::Heading
    } else {
        InlineCtx::Block
    };
    let content = lower_inline(content_of(node), lookup, w, ctx);
    let attrs: Vec<(&str, Value)> = if name == "heading" {
        let level = attrs_of(node)
            .and_then(|a| a.get("level"))
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .clamp(1, 6);
        vec![("level", Value::from(level))]
    } else {
        vec![]
    };
    make(name, &attrs, content)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineCtx {
    Block,
    Heading,
    Cell,
}

fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn has_mark(node: &Value, name: &str) -> bool {
    crate::convert::pm::marks_of(node)
        .iter()
        .any(|m| type_of(m) == name)
}

fn without_mark(node: &Value, name: &str) -> Value {
    let mut n = node.clone();
    if let Some(Value::Array(marks)) = n.get_mut("marks") {
        marks.retain(|m| type_of(m) != name);
        if marks.is_empty() {
            n.as_object_mut().expect("object").remove("marks");
        }
    }
    n
}

fn text_of(node: &Value) -> &str {
    node.get("text").and_then(Value::as_str).unwrap_or("")
}

fn set_text(node: &mut Value, text: String) {
    node.as_object_mut()
        .expect("object")
        .insert("text".into(), Value::from(text));
}

/// The lossy rules for inline content (§6.4): unknown nodes and marks dropped; links outside the allow-list become
/// text; item links canonical; hard breaks at the edges dropped (inside a heading: a space); whitespace at the edges
/// of a delimited mark moved outside it; leading whitespace of each line and trailing whitespace of the content
/// dropped.
fn lower_inline(
    content: &[Value],
    lookup: &dyn Lookup,
    w: &mut Warnings,
    ctx: InlineCtx,
) -> Vec<Value> {
    let schema = v1();
    // 1. Nodes and marks.
    let mut nodes: Vec<Value> = Vec::new();
    for n in content {
        match type_of(n) {
            "text" => {
                let mut t = n.clone();
                let text = text_of(n).replace(['\n', '\r'], " ");
                set_text(&mut t, text);
                let marks: Vec<Value> = crate::convert::pm::marks_of(n)
                    .iter()
                    .filter_map(|m| {
                        let name = type_of(m);
                        if schema.mark(name).is_none() {
                            w.add(W::UnknownNode, None);
                            return None;
                        }
                        if name == "link" {
                            return lower_link(m, lookup, w);
                        }
                        Some(m.clone())
                    })
                    .collect();
                let marks = if marks.iter().any(|m| type_of(m) == "code") {
                    if marks.len() > 1 {
                        w.add(W::MarksInCodeDropped, None);
                    }
                    vec![json!({"type": "code"})]
                } else {
                    schema::normal_marks(&marks)
                };
                let o = t.as_object_mut().expect("object");
                if marks.is_empty() {
                    o.remove("marks");
                } else {
                    o.insert("marks".into(), Value::Array(marks));
                }
                nodes.push(t);
            }
            "hardBreak" => {
                if ctx == InlineCtx::Heading {
                    w.add(W::HardBreakDropped, None);
                    nodes.push(json!({"type": "text", "text": " "}));
                } else {
                    nodes.push(json!({"type": "hardBreak"}));
                }
            }
            "mention" => {
                if let Some(id) = str_attr(n, "itemId") {
                    nodes.push(json!({"type": "mention", "attrs": {"itemId": id}}));
                }
            }
            _ => w.add(W::UnknownNode, None),
        }
    }
    // 2. Whitespace at the edges of delimited marks moves outside them.
    for mark in DELIMITED {
        let mut i = 0;
        while i < nodes.len() {
            if !(type_of(&nodes[i]) == "text" && has_mark(&nodes[i], mark)) {
                i += 1;
                continue;
            }
            let start = i;
            while i < nodes.len() && type_of(&nodes[i]) == "text" && has_mark(&nodes[i], mark) {
                i += 1;
            }
            let end = i; // exclusive
            let mut inserted = 0;
            // Leading whitespace.
            let lead: String = text_of(&nodes[start])
                .chars()
                .take_while(|c| is_ws(*c))
                .collect();
            if !lead.is_empty() {
                let rest = text_of(&nodes[start])[lead.len()..].to_owned();
                let mut outside = without_mark(&nodes[start], mark);
                set_text(&mut outside, lead);
                set_text(&mut nodes[start], rest);
                nodes.insert(start, outside);
                inserted += 1;
            }
            let last = end - 1 + inserted;
            let trail_len = text_of(&nodes[last])
                .chars()
                .rev()
                .take_while(|c| is_ws(*c))
                .count();
            if trail_len > 0 {
                let text = text_of(&nodes[last]).to_owned();
                let cut = text.len() - trail_len;
                let mut outside = without_mark(&nodes[last], mark);
                set_text(&mut outside, text[cut..].to_owned());
                set_text(&mut nodes[last], text[..cut].to_owned());
                nodes.insert(last + 1, outside);
                inserted += 1;
            }
            nodes.retain(|n| type_of(n) != "text" || !text_of(n).is_empty());
            i = end + inserted;
            // `retain` may have shifted the run; restart the scan from its start to be safe.
            i = i.min(nodes.len());
            if inserted > 0 {
                i = start;
                // Skip past the (now whitespace-free) run.
                while i < nodes.len()
                    && !(type_of(&nodes[i]) == "text" && has_mark(&nodes[i], mark))
                {
                    i += 1;
                }
                while i < nodes.len() && type_of(&nodes[i]) == "text" && has_mark(&nodes[i], mark) {
                    i += 1;
                }
            }
        }
    }
    // 3. Hard breaks at the edges, and edge whitespace of lines.
    loop {
        match nodes.first() {
            Some(n) if type_of(n) == "hardBreak" => {
                w.add(W::HardBreakDropped, None);
                nodes.remove(0);
            }
            _ => break,
        }
    }
    loop {
        match nodes.last() {
            Some(n) if type_of(n) == "hardBreak" => {
                w.add(W::HardBreakDropped, None);
                nodes.pop();
            }
            _ => break,
        }
    }
    let mut out: Vec<Value> = Vec::new();
    let mut line_start = true;
    for mut n in nodes {
        match type_of(&n) {
            "hardBreak" => {
                // Trailing whitespace before a hard break.
                trim_end_plain(&mut out);
                out.push(n);
                line_start = true;
            }
            "text" => {
                if line_start && !has_mark(&n, "code") && !has_mark(&n, "link") {
                    let t = text_of(&n).trim_start_matches(is_ws).to_owned();
                    set_text(&mut n, t);
                }
                if !text_of(&n).is_empty() {
                    line_start = false;
                    push_inline(&mut out, n);
                }
            }
            _ => {
                line_start = false;
                out.push(n);
            }
        }
    }
    trim_end_plain(&mut out);
    // Edge whitespace stripping can leave hard breaks at the edges again.
    while out.last().is_some_and(|n| type_of(n) == "hardBreak") {
        out.pop();
        w.add(W::HardBreakDropped, None);
    }
    let _ = ctx;
    out
}

/// Drops trailing whitespace of the last text node when it is plain (not code or link) text.
fn trim_end_plain(out: &mut Vec<Value>) {
    while let Some(last) = out.last_mut() {
        if type_of(last) != "text" || has_mark(last, "code") || has_mark(last, "link") {
            return;
        }
        let t = text_of(last).trim_end_matches(is_ws).to_owned();
        if t.is_empty() {
            out.pop();
            continue;
        }
        set_text(last, t);
        return;
    }
}

/// A link mark after the rules: outside the allow-list it is dropped; an item link is `quire://item/<id>`.
fn lower_link(mark: &Value, lookup: &dyn Lookup, w: &mut Warnings) -> Option<Value> {
    let href = str_attr(mark, "href").unwrap_or("");
    let item = str_attr(mark, "itemId");
    if let Some(id) = item {
        let url = quire_item_url(id);
        let known = lookup.item(&ItemRef::Id(id.to_owned())).map(|i| i.id);
        return Some(json!({"type": "link", "attrs": {"href": url, "itemId": known}}));
    }
    if !link_allowed(href) {
        w.add(W::UnsafeLink, None);
        return None;
    }
    let known = item_ref(href, lookup.public_origin())
        .and_then(|r| lookup.item(&r))
        .map(|i| i.id);
    let href = match (&known, quire_item_id(href)) {
        (Some(id), Some(_)) => quire_item_url(id),
        _ => href.to_owned(),
    };
    Some(json!({"type": "link", "attrs": {"href": href, "itemId": known}}))
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering a lowered tree.

/// Renders a lowered document as Markdown (one trailing newline).
pub fn render(lowered: &Value, lookup: &dyn Lookup) -> String {
    let lines = render_blocks(content_of(lowered), lookup, 0);
    let mut s = lines.join("\n");
    while s.ends_with('\n') {
        s.pop();
    }
    s.push('\n');
    s
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListMarker {
    Dash,
    Star,
    Dot,
    Paren,
}

/// The lines of sibling blocks, separated by one blank line.
fn render_blocks(blocks: &[Value], lookup: &dyn Lookup, depth: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut prev_marker: Option<ListMarker> = None;
    for b in blocks {
        let name = type_of(b);
        if name == "paragraph" && content_of(b).is_empty() {
            continue;
        }
        let marker = match name {
            "bulletList" | "taskList" => Some(if prev_marker == Some(ListMarker::Dash) {
                ListMarker::Star
            } else {
                ListMarker::Dash
            }),
            "orderedList" => Some(if prev_marker == Some(ListMarker::Dot) {
                ListMarker::Paren
            } else {
                ListMarker::Dot
            }),
            _ => None,
        };
        let lines = render_block(b, lookup, depth, marker);
        if lines.is_empty() {
            continue;
        }
        prev_marker = marker;
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend(lines);
    }
    out
}

fn prefixed(lines: Vec<String>, first: &str, rest: &str, blank: &str) -> Vec<String> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let p = if i == 0 { first } else { rest };
            if l.is_empty() {
                if i == 0 {
                    first.trim_end().to_owned()
                } else {
                    blank.to_owned()
                }
            } else {
                format!("{p}{l}")
            }
        })
        .collect()
}

fn render_block(
    b: &Value,
    lookup: &dyn Lookup,
    depth: usize,
    marker: Option<ListMarker>,
) -> Vec<String> {
    match type_of(b) {
        "paragraph" => {
            let text = render_inline(content_of(b), lookup, InlineCtx::Block);
            text.split('\n').map(str::to_owned).collect()
        }
        "heading" => {
            let level = attrs_of(b)
                .and_then(|a| a.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let hashes = "#".repeat(level.clamp(1, 6) as usize);
            let text = render_inline(content_of(b), lookup, InlineCtx::Heading);
            if text.is_empty() {
                vec![hashes]
            } else {
                vec![format!("{hashes} {text}")]
            }
        }
        "codeBlock" => {
            let text = content_of(b).first().map(text_of).unwrap_or("");
            let lang = str_attr(b, "language").unwrap_or("");
            let fence_char = if lang.contains('`') { '~' } else { '`' };
            let longest = longest_run(text, fence_char);
            let fence: String = std::iter::repeat_n(fence_char, (longest + 1).max(3)).collect();
            let mut lines = vec![format!("{fence}{lang}")];
            if !text.is_empty() {
                lines.extend(text.split('\n').map(str::to_owned));
            }
            lines.push(fence);
            lines
        }
        "horizontalRule" => vec!["---".into()],
        "image" => vec![render_image(b)],
        "boardEmbed" => {
            let id = str_attr(b, "itemId").unwrap_or("");
            let title = lookup
                .item(&ItemRef::Id(id.to_owned()))
                .map(|i| i.title)
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Board".into());
            vec![format!(
                "![{}]({})",
                escape_label(&title),
                quire_item_url(id)
            )]
        }
        "blockquote" => {
            let inner = render_blocks(content_of(b), lookup, depth + 1);
            if inner.is_empty() {
                return vec![">".into()];
            }
            prefixed(inner, "> ", "> ", ">")
        }
        "callout" => {
            let variant = str_attr(b, "variant").unwrap_or("info");
            let alert = ALERTS
                .iter()
                .find(|(v, _)| *v == variant)
                .map_or("NOTE", |(_, a)| a);
            let mut lines = vec![format!("> [!{alert}]")];
            let inner = render_blocks(content_of(b), lookup, depth + 1);
            lines.extend(prefixed(inner, "> ", "> ", ">"));
            lines
        }
        "bulletList" | "orderedList" | "taskList" => {
            let marker = marker.unwrap_or(ListMarker::Dash);
            let start = attrs_of(b)
                .and_then(|a| a.get("start"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let mut lines = Vec::new();
            for (i, item) in content_of(b).iter().enumerate() {
                let m = match marker {
                    ListMarker::Dash => "-".to_owned(),
                    ListMarker::Star => "*".to_owned(),
                    ListMarker::Dot => format!("{}.", start + i as i64),
                    ListMarker::Paren => format!("{})", start + i as i64),
                };
                let indent = " ".repeat(m.len() + 1);
                let task = (type_of(item) == "taskItem").then(|| {
                    let checked = attrs_of(item)
                        .and_then(|a| a.get("checked"))
                        .and_then(Value::as_bool)
                        == Some(true);
                    if checked { "[x]" } else { "[ ]" }
                });
                let head = match task {
                    Some(t) => format!("{m} {t}"),
                    None => m.clone(),
                };
                let children = content_of(item);
                let first_text = children
                    .first()
                    .filter(|c| type_of(c) == "paragraph" && !content_of(c).is_empty());
                let mut item_lines: Vec<String> = Vec::new();
                let rest: &[Value] = if children.is_empty() {
                    &[]
                } else {
                    &children[1..]
                };
                match first_text {
                    Some(p) => {
                        let plines = render_block(p, lookup, depth + 1, None);
                        item_lines.extend(prefixed(plines, &format!("{head} "), &indent, ""));
                        let more = render_blocks(rest, lookup, depth + 1);
                        if !more.is_empty() {
                            item_lines.push(String::new());
                            item_lines.extend(prefixed(more, &indent, &indent, ""));
                        }
                    }
                    None => {
                        item_lines.push(head.clone());
                        let more = render_blocks(rest, lookup, depth + 1);
                        item_lines.extend(prefixed(more, &indent, &indent, ""));
                    }
                }
                lines.extend(item_lines);
            }
            lines
        }
        "table" => render_table(b, lookup),
        _ => vec![],
    }
}

fn longest_run(text: &str, c: char) -> usize {
    let (mut best, mut cur) = (0, 0);
    for ch in text.chars() {
        if ch == c {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

fn render_image(b: &Value) -> String {
    let src = str_attr(b, "src").unwrap_or("");
    let alt = str_attr(b, "alt").unwrap_or("");
    let url = escape_url(src);
    match str_attr(b, "caption") {
        Some(c) => format!(
            "![{}]({url} \"{}\")",
            escape_label(alt),
            c.replace('\\', "\\\\").replace('"', "\\\"")
        ),
        None => format!("![{}]({url})", escape_label(alt)),
    }
}

fn render_table(b: &Value, lookup: &dyn Lookup) -> Vec<String> {
    let rows = content_of(b);
    let mut lines = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let cells: Vec<String> = content_of(row)
            .iter()
            .map(|cell| {
                let para = content_of(cell).first();
                let text = para.map_or(String::new(), |p| {
                    render_inline(content_of(p), lookup, InlineCtx::Cell)
                });
                if text.is_empty() {
                    " ".to_owned()
                } else {
                    format!(" {text} ")
                }
            })
            .collect();
        lines.push(format!("|{}|", cells.join("|")));
        if r == 0 {
            let delim: Vec<&str> = cells.iter().map(|_| " --- ").collect();
            lines.push(format!("|{}|", delim.join("|")));
        }
    }
    lines
}

/// Link destination: `<…>` when it holds a space; parentheses and backslashes escaped otherwise.
fn escape_url(url: &str) -> String {
    let url = url.replace(['\n', '\r'], "");
    if url.contains(' ') && !url.contains(['<', '>']) {
        return format!("<{url}>");
    }
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            '(' | ')' | '\\' | '<' | '>' => {
                out.push('\\');
                out.push(c);
            }
            ' ' => out.push_str("%20"),
            _ => out.push(c),
        }
    }
    escape_entities(&out)
}

/// Escapes `&` that would start a character reference.
fn escape_entities(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        if c == '&'
            && bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'#')
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Link text and image alt: every character Markdown could read as syntax is escaped.
fn escape_label(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i == 0 { None } else { Some(chars[i - 1]) };
        let next = chars.get(i + 1).copied();
        push_escaped(&mut out, c, prev, next, i == 0, false);
    }
    escape_autolinks(&out)
}

/// Breaks text that comrak's autolink extension would turn into a link (`http://`, `www.`).
fn escape_autolinks(s: &str) -> String {
    let mut out = s.replace("://", "\\://");
    let mut i = 0;
    while let Some(pos) = out[i..].find("www.") {
        let at = i + pos;
        let boundary = out[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        if boundary {
            out.replace_range(at..at + 1, "&#119;");
            i = at + 6;
        } else {
            i = at + 4;
        }
    }
    out
}

fn push_escaped(
    out: &mut String,
    c: char,
    prev: Option<char>,
    next: Option<char>,
    line_start: bool,
    heading: bool,
) {
    let escape = match c {
        '\\' | '`' | '*' | '[' | ']' | '<' | '|' | '~' => true,
        '_' => {
            !(prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric))
        }
        '=' => line_start || prev == Some('=') || next == Some('='),
        '&' => next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#'),
        '#' => line_start || heading,
        '>' | '+' | '-' => line_start,
        '!' => next == Some('['),
        '.' | ')' => {
            // "1." or "1)" at the start of a line would start an ordered list.
            line_start && false
        }
        _ => false,
    };
    if escape {
        out.push('\\');
    }
    out.push(c);
}

/// Renders lowered inline content (marks opened in rank order, closed in reverse).
fn render_inline(nodes: &[Value], lookup: &dyn Lookup, ctx: InlineCtx) -> String {
    let mut out = String::new();
    let mut open: Vec<&'static str> = Vec::new();
    let mut link: Option<String> = None;
    let mut line_start = true;
    let mut i = 0;
    let close_all = |out: &mut String, open: &mut Vec<&'static str>| {
        while let Some(m) = open.pop() {
            out.push_str(closer(m));
        }
    };
    while i < nodes.len() {
        let n = &nodes[i];
        let node_link = if type_of(n) == "text" {
            crate::convert::pm::marks_of(n)
                .iter()
                .find(|m| type_of(m) == "link")
                .map(|m| str_attr(m, "href").unwrap_or("").to_owned())
        } else {
            None
        };
        if node_link != link {
            close_all(&mut out, &mut open);
            if let Some(href) = link.take() {
                out.push_str(&format!("]({})", escape_url(&href)));
            }
            if let Some(href) = &node_link {
                // A "!" right before the link would make it an image.
                if out.ends_with('!') && !out.ends_with("\\!") {
                    out.pop();
                    out.push_str("\\!");
                }
                out.push('[');
                link = Some(href.clone());
            }
        }
        match type_of(n) {
            "hardBreak" => {
                close_all(&mut out, &mut open);
                out.push_str(if ctx == InlineCtx::Cell {
                    "<br>"
                } else {
                    "\\\n"
                });
                line_start = true;
            }
            "mention" => {
                close_all(&mut out, &mut open);
                let id = str_attr(n, "itemId").unwrap_or("");
                let title = lookup
                    .item(&ItemRef::Id(id.to_owned()))
                    .map(|i| i.title)
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| "Deleted item".into());
                out.push_str(&format!(
                    "[@{}]({})",
                    escape_label(&title),
                    quire_item_url(id)
                ));
                line_start = false;
            }
            "text" => {
                let text = text_of(n);
                if has_mark(n, "code") {
                    close_all(&mut out, &mut open);
                    let ticks: String = "`".repeat(longest_run(text, '`') + 1);
                    let pad = text.starts_with('`')
                        || text.ends_with('`')
                        || (text.starts_with(' ')
                            && text.ends_with(' ')
                            && text.chars().any(|c| c != ' '));
                    let pad = if pad { " " } else { "" };
                    out.push_str(&format!("{ticks}{pad}{text}{pad}{ticks}"));
                } else {
                    let want: Vec<&'static str> = DELIMITED
                        .iter()
                        .copied()
                        .filter(|m| has_mark(n, m))
                        .collect();
                    let keep = open.iter().take_while(|m| want.contains(m)).count();
                    while open.len() > keep {
                        out.push_str(closer(open.pop().expect("open")));
                    }
                    for m in &want {
                        if !open.contains(m) {
                            out.push_str(opener(m));
                            open.push(m);
                        }
                    }
                    let chars: Vec<char> = text.chars().collect();
                    let mut buf = String::new();
                    for (j, &c) in chars.iter().enumerate() {
                        let prev = if j == 0 {
                            out.chars().next_back()
                        } else {
                            Some(chars[j - 1])
                        };
                        let next = chars.get(j + 1).copied().or_else(|| next_char(nodes, i));
                        let at_start = line_start && j == 0 && open.is_empty() && link.is_none();
                        push_escaped(&mut buf, c, prev, next, at_start, ctx == InlineCtx::Heading);
                        if at_start && c.is_ascii_digit() {
                            // "1." / "1)" at a line start: escape the delimiter after the digits.
                            let digits =
                                chars[j..].iter().take_while(|d| d.is_ascii_digit()).count();
                            if matches!(chars.get(j + digits), Some('.') | Some(')')) {
                                for d in &chars[j + 1..j + digits] {
                                    buf.push(*d);
                                }
                                buf.push('\\');
                                buf.push(chars[j + digits]);
                                let rest: String = chars[j + digits + 1..].iter().collect();
                                buf.push_str(&escape_rest(
                                    &rest,
                                    chars.get(j + digits).copied(),
                                    next_char(nodes, i),
                                    ctx,
                                ));
                                break;
                            }
                        }
                    }
                    // Inside a link label comrak finds no autolinks.
                    if link.is_some() {
                        out.push_str(&buf);
                    } else {
                        out.push_str(&escape_autolinks(&buf));
                    }
                }
                line_start = false;
            }
            _ => {}
        }
        i += 1;
    }
    close_all(&mut out, &mut open);
    if let Some(href) = link.take() {
        out.push_str(&format!("]({})", escape_url(&href)));
    }
    out
}

fn escape_rest(rest: &str, prev: Option<char>, after: Option<char>, ctx: InlineCtx) -> String {
    let chars: Vec<char> = rest.chars().collect();
    let mut buf = String::new();
    for (j, &c) in chars.iter().enumerate() {
        let p = if j == 0 { prev } else { Some(chars[j - 1]) };
        let n = chars.get(j + 1).copied().or(after);
        push_escaped(&mut buf, c, p, n, false, ctx == InlineCtx::Heading);
    }
    buf
}

/// The first character after node `i` (for escaping decisions).
fn next_char(nodes: &[Value], i: usize) -> Option<char> {
    nodes.get(i + 1).and_then(|n| match type_of(n) {
        "text" => text_of(n).chars().next(),
        _ => None,
    })
}

fn opener(mark: &str) -> &'static str {
    match mark {
        "bold" => "**",
        "italic" => "*",
        "strike" => "~~",
        "underline" => "<u>",
        "highlight" => "==",
        _ => "",
    }
}

fn closer(mark: &str) -> &'static str {
    match mark {
        "bold" => "**",
        "italic" => "*",
        "strike" => "~~",
        "underline" => "</u>",
        "highlight" => "==",
        _ => "",
    }
}
