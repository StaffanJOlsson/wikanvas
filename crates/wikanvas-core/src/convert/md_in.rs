//! Quire Markdown 1 input: comrak to normalized ProseMirror JSON (owner: W4)
//!
//! [`parse`] runs [`prescan`] (a per-line upper bound on container nesting, refused above [`PRESCAN_LIMIT`] before
//! comrak sees the input), then comrak 0.59 with `table`, `strikethrough`, `tasklist`, `autolink`, `alerts` and
//! `highlight` on and everything else off, then [`check_depth`] (containers nested deeper than 100 in comrak's tree
//! are refused, iteratively), then converts the AST to schema-v1 JSON following the "PUT Markdown" column of the
//! §6.4 table. Raw HTML is never rendered: an HTML block becomes a paragraph of its literal text, inline HTML literal
//! text, except the exact pair `<u>`…`</u>` (underline) and `<br>` in a table cell (a hard break). The result is
//! normalized (`schema::normalize`); blocks have no `blockId` (the writer assigns them).
//!
//! Block conversion recurses, within the bound [`check_depth`] enforces; inline conversion uses an explicit stack,
//! since emphasis can nest as deep as the input is long.

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{AlertType, AstNode, ListType, NodeValue, TableAlignment};
use comrak::{Arena, Options};
use serde_json::{Map, Value, json};

use crate::api::{ConversionWarningCode as W, ItemKind};
use crate::convert::{
    ItemRef, Lookup, Warnings, attachment_ref, attachment_url, image_allowed, item_ref,
    link_allowed,
};
use crate::schema::{self, Invalid, MAX_DEPTH, type_of};

/// The input limit of a Markdown PUT (D33).
pub const MAX_MARKDOWN_BYTES: usize = 1024 * 1024;

/// comrak's options for Quire Markdown 1.
pub fn options() -> Options<'static> {
    let mut o = Options::default();
    o.extension.table = true;
    o.extension.strikethrough = true;
    o.extension.tasklist = true;
    o.extension.autolink = true;
    o.extension.alerts = true;
    o.extension.highlight = true;
    o
}

fn is_list_marker(line: &[u8], at: usize) -> Option<usize> {
    let c = *line.get(at)?;
    let after = |i: usize| line.get(i).is_none_or(|b| *b == b' ' || *b == b'\t');
    if matches!(c, b'-' | b'*' | b'+') && after(at + 1) {
        return Some(1);
    }
    let digits = line[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&digits)
        && matches!(line.get(at + digits), Some(b'.') | Some(b')'))
        && after(at + digits + 1)
    {
        return Some(digits + 1);
    }
    None
}

/// The pre-scan's bound (see [`prescan`]): twice [`MAX_DEPTH`], so lists indented by four columns per level still
/// reach the exact check on comrak's tree, which refuses anything deeper than [`MAX_DEPTH`].
pub const PRESCAN_LIMIT: usize = 2 * MAX_DEPTH;

/// Refuses container nesting (`>` and list markers, including lists nested by indentation, lazy continuation lines
/// and tabs) that could be deeper than [`PRESCAN_LIMIT`], with the `line:column` where the line's bound first passes
/// [`MAX_DEPTH`] (§6.4 step 1, D33), before comrak sees the input.
///
/// The bound needs no state across lines. A line only opens containers through the markers of its prefix (the
/// leading run of whitespace, `>` and list markers); every container it continues is a quote, matched by one `>` of
/// that prefix, or a list item, matched by at least two columns of its whitespace (a marker and the space after it).
/// So `markers + whitespace columns / 2` of the prefix bounds the depth of every line that opens a container, and a
/// line that opens none (a blank or lazy line among them) is no deeper than the line before it. The bound can only
/// over-count; [`check_depth`] then applies the exact limit to comrak's tree.
pub fn prescan(md: &str) -> Result<(), Invalid> {
    for (n, line) in md.split('\n').enumerate() {
        // A lone CR ends a line as well (CommonMark); CRLF leaves an empty piece.
        for piece in line.split('\r') {
            prescan_line(piece.as_bytes(), n + 1)?;
        }
    }
    Ok(())
}

fn prescan_line(bytes: &[u8], line: usize) -> Result<(), Invalid> {
    let (mut at, mut col, mut ws, mut markers) = (0, 0, 0, 0);
    let mut over_at = None;
    loop {
        while let Some(b @ (b' ' | b'\t')) = bytes.get(at) {
            let w = if *b == b'\t' { 4 - col % 4 } else { 1 };
            col += w;
            ws += w;
            at += 1;
        }
        let width = match bytes.get(at) {
            Some(b'>') => 1,
            Some(_) => match is_list_marker(bytes, at) {
                Some(w) => w,
                None => return Ok(()),
            },
            None => return Ok(()),
        };
        markers += 1;
        let bound = markers + ws / 2;
        if bound > MAX_DEPTH && over_at.is_none() {
            over_at = Some(at);
        }
        if bound > PRESCAN_LIMIT {
            return Err(Invalid::new(
                format!("{line}:{}", over_at.unwrap_or(at) + 1),
                format!("containers (quotes and lists) are nested deeper than {MAX_DEPTH} levels"),
            ));
        }
        at += width;
        col += width;
    }
}

/// Whether a comrak node is a container the depth limit counts: a quote (alerts included) or a list item.
fn is_container(node: &AstNode<'_>) -> bool {
    matches!(
        node.data().value,
        NodeValue::BlockQuote | NodeValue::Alert(_) | NodeValue::Item(_) | NodeValue::TaskItem(_)
    )
}

/// The exact limit: containers nested deeper than [`MAX_DEPTH`] in comrak's tree are refused, with the
/// `line:column` of the first one too deep, before the (recursive) block conversion runs. Iterative.
pub fn check_depth<'a>(root: &'a AstNode<'a>) -> Result<(), Invalid> {
    let mut depth = 0usize;
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) if is_container(node) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(Invalid::new(
                        line_col(node),
                        format!(
                            "containers (quotes and lists) are nested deeper than {MAX_DEPTH} levels"
                        ),
                    ));
                }
            }
            NodeEdge::End(node) if is_container(node) => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}

/// Parses Quire Markdown 1 into a normalized document.
pub fn parse(md: &str, lookup: &dyn Lookup) -> Result<(Value, Warnings), Invalid> {
    prescan(md)?;
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, md, &options());
    check_depth(root)?;
    let mut cx = Cx {
        lookup,
        warnings: Warnings::default(),
    };
    unsupported(md, &mut cx.warnings);
    let mut content = cx.blocks(root);
    if content.is_empty() {
        content.push(json!({"type": "paragraph"}));
    }
    let doc = json!({"type": "doc", "content": content});
    Ok((schema::normalize(&doc), cx.warnings))
}

/// Footnotes, math and front matter stay literal text, with a warning.
fn unsupported(md: &str, w: &mut Warnings) {
    let mut lines = md.lines();
    let front = lines.next().is_some_and(|l| l.trim_end() == "---")
        && md
            .lines()
            .skip(1)
            .take(50)
            .any(|l| l.trim_end() == "---" || l.trim_end() == "...");
    let footnote = md.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("[^") && t.contains("]:")
    }) || md.contains("[^");
    let math = md.contains("$$");
    if front || footnote || math {
        w.add(W::UnsupportedMarkdown, None);
    }
}

struct Cx<'l> {
    lookup: &'l dyn Lookup,
    warnings: Warnings,
}

fn text(s: &str) -> Value {
    json!({"type": "text", "text": s})
}

fn line_col(node: &AstNode<'_>) -> String {
    let sp = node.data().sourcepos;
    format!("{}:{}", sp.start.line, sp.start.column)
}

fn ensure_first_paragraph(content: &mut Vec<Value>) {
    if content.first().is_none_or(|c| type_of(c) != "paragraph") {
        content.insert(0, json!({"type": "paragraph"}));
    }
}

/// An inline piece of a paragraph before hoisting: inline nodes, or a block an image became.
enum Piece {
    Inline(Value),
    Block(Value),
}

impl Cx<'_> {
    fn blocks<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Value> {
        let mut out = Vec::new();
        for child in parent.children() {
            self.block(child, &mut out);
        }
        out
    }

    fn box_blocks<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Value> {
        let mut out = self.blocks(parent);
        if out.is_empty() {
            out.push(json!({"type": "paragraph"}));
        }
        out
    }

    fn block<'a>(&mut self, node: &'a AstNode<'a>, out: &mut Vec<Value>) {
        let value = node.data().value.clone();
        match value {
            NodeValue::Paragraph => self.paragraph(node, "paragraph", None, out),
            NodeValue::Heading(h) => self.paragraph(node, "heading", Some(h.level), out),
            NodeValue::BlockQuote => {
                out.push(json!({"type": "blockquote", "content": self.box_blocks(node)}))
            }
            NodeValue::Alert(alert) => {
                let variant = match alert.alert_type {
                    AlertType::Note => "info",
                    AlertType::Tip => "tip",
                    AlertType::Warning => "warning",
                    AlertType::Caution => "danger",
                    AlertType::Important => {
                        self.warnings.add(W::AlertMapped, Some(line_col(node)));
                        "info"
                    }
                };
                let mut content = self.blocks(node);
                if let Some(title) = alert
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                {
                    content.insert(0, json!({"type": "paragraph", "content": [text(title)]}));
                }
                if content.is_empty() {
                    content.push(json!({"type": "paragraph"}));
                }
                out.push(
                    json!({"type": "callout", "attrs": {"variant": variant}, "content": content}),
                );
            }
            NodeValue::List(list) => self.list(node, &list, out),
            NodeValue::Item(_) | NodeValue::TaskItem(_) => {
                // An item outside a list (not produced by comrak): its blocks.
                out.extend(self.blocks(node));
            }
            NodeValue::CodeBlock(cb) => {
                let language = cb.info.split_whitespace().next().map(str::to_owned);
                let mut literal = cb.literal.clone();
                if literal.ends_with('\n') {
                    literal.pop();
                }
                let mut n = json!({"type": "codeBlock", "attrs": {"language": language}});
                if !literal.is_empty() {
                    n["content"] = json!([text(&literal)]);
                }
                out.push(n);
            }
            NodeValue::ThematicBreak => out.push(json!({"type": "horizontalRule"})),
            NodeValue::HtmlBlock(html) => {
                self.warnings.add(W::HtmlAsText, Some(line_col(node)));
                let mut content = Vec::new();
                for (i, line) in html.literal.trim_end_matches('\n').split('\n').enumerate() {
                    if i > 0 {
                        content.push(json!({"type": "hardBreak"}));
                    }
                    if !line.is_empty() {
                        content.push(text(line));
                    }
                }
                if !content.is_empty() {
                    out.push(json!({"type": "paragraph", "content": content}));
                }
            }
            NodeValue::Table(table) => self.table(node, &table.alignments, table.num_columns, out),
            NodeValue::FrontMatter(s) => {
                self.warnings
                    .add(W::UnsupportedMarkdown, Some(line_col(node)));
                out.push(json!({"type": "paragraph", "content": [text(s.trim())]}));
            }
            _ => {
                // Anything else comrak may produce with these options: its text, as a paragraph.
                let mut content = Vec::new();
                self.inline_into(node, &mut content, false);
                let content = flatten(content);
                if !content.is_empty() {
                    out.push(json!({"type": "paragraph", "content": content}));
                }
            }
        }
    }

    fn list<'a>(
        &mut self,
        node: &'a AstNode<'a>,
        list: &comrak::nodes::NodeList,
        out: &mut Vec<Value>,
    ) {
        let first_task = node
            .first_child()
            .is_some_and(|c| matches!(c.data().value, NodeValue::TaskItem(_)));
        let mut items = Vec::new();
        for item in node.children() {
            let value = item.data().value.clone();
            let mut content = self.blocks(item);
            match (first_task, value) {
                (true, NodeValue::TaskItem(t)) => {
                    ensure_first_paragraph(&mut content);
                    items.push(json!({"type": "taskItem", "attrs": {"checked": t.symbol.is_some()}, "content": content}));
                }
                (true, _) => {
                    self.warnings.add(W::TaskListMixed, Some(line_col(item)));
                    ensure_first_paragraph(&mut content);
                    items.push(json!({"type": "taskItem", "attrs": {"checked": false}, "content": content}));
                }
                (false, NodeValue::TaskItem(t)) => {
                    // A checkbox after the first item of a plain list: kept as text.
                    let mark = match t.symbol {
                        Some(c) => format!("[{c}] "),
                        None => "[ ] ".to_owned(),
                    };
                    match content.first_mut() {
                        Some(p) if type_of(p) == "paragraph" => {
                            let mut inline = p
                                .get("content")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default();
                            inline.insert(0, text(&mark));
                            p["content"] = Value::Array(inline);
                        }
                        _ => content.insert(
                            0,
                            json!({"type": "paragraph", "content": [text(mark.trim_end())]}),
                        ),
                    }
                    items.push(json!({"type": "listItem", "content": content}));
                }
                (false, _) => {
                    ensure_first_paragraph(&mut content);
                    items.push(json!({"type": "listItem", "content": content}));
                }
            }
        }
        let n = if first_task {
            json!({"type": "taskList", "content": items})
        } else if list.list_type == ListType::Ordered {
            json!({"type": "orderedList", "attrs": {"start": list.start}, "content": items})
        } else {
            json!({"type": "bulletList", "content": items})
        };
        out.push(n);
    }

    fn table<'a>(
        &mut self,
        node: &'a AstNode<'a>,
        align: &[TableAlignment],
        columns: usize,
        out: &mut Vec<Value>,
    ) {
        if align.iter().any(|a| *a != TableAlignment::None) {
            self.warnings
                .add(W::TableAlignmentDropped, Some(line_col(node)));
        }
        let mut rows = Vec::new();
        for (r, row) in node.children().enumerate() {
            let header = matches!(row.data().value, NodeValue::TableRow(true)) || r == 0;
            let cell_name = if header { "tableHeader" } else { "tableCell" };
            let mut cells = Vec::new();
            for cell in row.children() {
                let mut pieces = Vec::new();
                self.inline_into(cell, &mut pieces, true);
                let (inline, blocks) = split_pieces(pieces);
                let mut content = vec![paragraph_of(inline, &mut self.warnings)];
                content.extend(blocks);
                cells.push(json!({"type": cell_name, "content": content}));
            }
            while cells.len() < columns {
                cells.push(json!({"type": cell_name, "content": [{"type": "paragraph"}]}));
            }
            rows.push(json!({"type": "tableRow", "content": cells}));
        }
        out.push(json!({"type": "table", "content": rows}));
    }

    /// A paragraph or heading: its inline content, with images hoisted into blocks after it (§6.4 table).
    fn paragraph<'a>(
        &mut self,
        node: &'a AstNode<'a>,
        name: &str,
        level: Option<u8>,
        out: &mut Vec<Value>,
    ) {
        let mut pieces = Vec::new();
        self.inline_into(node, &mut pieces, false);
        let has_inline_content = pieces.iter().any(|p| match p {
            Piece::Inline(v) => {
                !(type_of(v) == "text" && v["text"].as_str().is_some_and(|t| t.trim().is_empty()))
            }
            Piece::Block(_) => false,
        });
        let has_blocks = pieces.iter().any(|p| matches!(p, Piece::Block(_)));
        let (inline, blocks) = split_pieces(pieces);
        if has_blocks && has_inline_content {
            self.warnings.add(W::ImageHoisted, Some(line_col(node)));
        }
        if has_inline_content || !has_blocks || name == "heading" {
            let mut p = paragraph_of(inline, &mut self.warnings);
            if name == "heading" {
                p["type"] = Value::from("heading");
                p["attrs"] = json!({"level": level.unwrap_or(1)});
            }
            if name == "heading" || p.get("content").is_some() {
                out.push(p);
            }
        }
        out.extend(blocks);
    }

    /// The inline content of `node` (iterative). In a table cell, `<br>` is a hard break.
    fn inline_into<'a>(&mut self, node: &'a AstNode<'a>, out: &mut Vec<Piece>, cell: bool) {
        // Pair up <u> and </u> first (in document order).
        let mut order: Vec<&'a AstNode<'a>> = Vec::new();
        {
            let mut stack: Vec<&'a AstNode<'a>> = node
                .children()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            while let Some(n) = stack.pop() {
                order.push(n);
                if !matches!(n.data().value, NodeValue::Image(_)) {
                    stack.extend(n.children().collect::<Vec<_>>().into_iter().rev());
                }
            }
        }
        let tag_of = |n: &AstNode<'_>| match &n.data().value {
            NodeValue::HtmlInline(h) => Some(h.clone()),
            _ => None,
        };
        let mut underline_pairs: Vec<bool> = vec![false; order.len()];
        let mut open: Option<usize> = None;
        for (i, n) in order.iter().enumerate() {
            match tag_of(n).as_deref() {
                Some("<u>") if open.is_none() => open = Some(i),
                Some("</u>") => {
                    if let Some(o) = open.take() {
                        underline_pairs[o] = true;
                        underline_pairs[i] = true;
                    }
                }
                _ => {}
            }
        }
        // Walk with the marks of the enclosing nodes.
        struct Frame<'a> {
            children: Vec<&'a AstNode<'a>>,
            next: usize,
            marks: Vec<Value>,
        }
        let mut index_of = std::collections::HashMap::new();
        for (i, n) in order.iter().enumerate() {
            index_of.insert(*n as *const AstNode<'a>, i);
        }
        let mut underline = false;
        let mut stack = vec![Frame {
            children: node.children().collect(),
            next: 0,
            marks: Vec::new(),
        }];
        while let Some(top) = stack.last_mut() {
            let Some(&child) = top.children.get(top.next) else {
                stack.pop();
                continue;
            };
            top.next += 1;
            let mut marks = top.marks.clone();
            if underline && !marks.iter().any(|m| type_of(m) == "underline") {
                marks.push(json!({"type": "underline"}));
            }
            let value = child.data().value.clone();
            let push_text = |out: &mut Vec<Piece>, s: &str, marks: &[Value]| {
                if s.is_empty() {
                    return;
                }
                let mut t = text(s);
                if !marks.is_empty() {
                    t["marks"] = Value::Array(marks.to_vec());
                }
                out.push(Piece::Inline(t));
            };
            let nested = |mark: Value, marks: &Vec<Value>| {
                let mut m = marks.clone();
                if !m.iter().any(|x| type_of(x) == type_of(&mark)) {
                    m.push(mark);
                }
                m
            };
            match value {
                NodeValue::Text(s) => push_text(out, &s, &marks),
                NodeValue::SoftBreak => push_text(out, " ", &marks),
                NodeValue::LineBreak => out.push(Piece::Inline(json!({"type": "hardBreak"}))),
                NodeValue::Code(c) => {
                    let had_others = marks.iter().any(|m| type_of(m) != "link");
                    let link = marks.iter().any(|m| type_of(m) == "link");
                    if had_others || link {
                        self.warnings
                            .add(W::MarksInCodeDropped, Some(line_col(child)));
                    }
                    push_text(out, &c.literal, &[json!({"type": "code"})]);
                }
                NodeValue::HtmlInline(h) => {
                    let i = index_of[&(child as *const AstNode<'a>)];
                    if underline_pairs[i] {
                        underline = h == "<u>";
                    } else if cell && is_br(&h) {
                        out.push(Piece::Inline(json!({"type": "hardBreak"})));
                    } else {
                        self.warnings.add(W::HtmlAsText, Some(line_col(child)));
                        push_text(out, &h, &marks);
                    }
                }
                NodeValue::Emph => {
                    let m = nested(json!({"type": "italic"}), &marks);
                    stack.push(Frame {
                        children: child.children().collect(),
                        next: 0,
                        marks: m,
                    });
                }
                NodeValue::Strong => {
                    let m = nested(json!({"type": "bold"}), &marks);
                    stack.push(Frame {
                        children: child.children().collect(),
                        next: 0,
                        marks: m,
                    });
                }
                NodeValue::Strikethrough => {
                    let m = nested(json!({"type": "strike"}), &marks);
                    stack.push(Frame {
                        children: child.children().collect(),
                        next: 0,
                        marks: m,
                    });
                }
                NodeValue::Highlight => {
                    let m = nested(json!({"type": "highlight"}), &marks);
                    stack.push(Frame {
                        children: child.children().collect(),
                        next: 0,
                        marks: m,
                    });
                }
                NodeValue::Link(link) => {
                    let label = plain_text(child);
                    match self.link(&link.url, &label, child) {
                        LinkTarget::Mention(id) => {
                            out.push(Piece::Inline(
                                json!({"type": "mention", "attrs": {"itemId": id}}),
                            ));
                        }
                        LinkTarget::Mark(mark) => {
                            let mut m: Vec<Value> = marks
                                .iter()
                                .filter(|x| type_of(x) != "link")
                                .cloned()
                                .collect();
                            m.push(mark);
                            stack.push(Frame {
                                children: child.children().collect(),
                                next: 0,
                                marks: m,
                            });
                        }
                        LinkTarget::Text => {
                            stack.push(Frame {
                                children: child.children().collect(),
                                next: 0,
                                marks: marks.clone(),
                            });
                        }
                    }
                }
                NodeValue::Image(link) => {
                    let alt = plain_text(child);
                    for p in self.image(&link.url, &link.title, &alt, child) {
                        out.push(p);
                    }
                }
                _ => {
                    // Other inlines (none with these options): their children.
                    stack.push(Frame {
                        children: child.children().collect(),
                        next: 0,
                        marks: marks.clone(),
                    });
                }
            }
        }
    }

    fn link(&mut self, url: &str, label: &str, node: &AstNode<'_>) -> LinkTarget {
        let origin = self.lookup.public_origin();
        if let Some(r) = item_ref(url, origin) {
            let found = self.lookup.item(&r);
            if label.starts_with('@') {
                match (&found, &r) {
                    (Some(info), _) => return LinkTarget::Mention(info.id.clone()),
                    (None, ItemRef::Id(id)) => return LinkTarget::Mention(id.clone()),
                    _ => {}
                }
            }
            return match found {
                Some(info) => LinkTarget::Mark(
                    json!({"type": "link", "attrs": {"href": url, "itemId": info.id}}),
                ),
                None => {
                    self.warnings.add(W::UnknownItem, Some(line_col(node)));
                    if link_allowed(url) {
                        LinkTarget::Mark(json!({"type": "link", "attrs": {"href": url}}))
                    } else {
                        LinkTarget::Text
                    }
                }
            };
        }
        if link_allowed(url) {
            LinkTarget::Mark(json!({"type": "link", "attrs": {"href": url}}))
        } else {
            self.warnings.add(W::UnsafeLink, Some(line_col(node)));
            LinkTarget::Text
        }
    }

    fn image(&mut self, url: &str, title: &str, alt: &str, node: &AstNode<'_>) -> Vec<Piece> {
        let origin = self.lookup.public_origin();
        if let Some(r) = item_ref(url, origin) {
            return match (self.lookup.item(&r), &r) {
                (Some(info), _) if info.kind == ItemKind::Board => {
                    vec![Piece::Block(
                        json!({"type": "boardEmbed", "attrs": {"itemId": info.id}}),
                    )]
                }
                (Some(info), _) => {
                    self.warnings.add(W::EmbedNotBoard, Some(line_col(node)));
                    vec![Piece::Inline(
                        json!({"type": "mention", "attrs": {"itemId": info.id}}),
                    )]
                }
                (None, ItemRef::Id(id)) => {
                    self.warnings.add(W::UnknownItem, Some(line_col(node)));
                    vec![Piece::Block(
                        json!({"type": "boardEmbed", "attrs": {"itemId": id}}),
                    )]
                }
                (None, ItemRef::UrlId(_)) => {
                    self.warnings.add(W::UnknownItem, Some(line_col(node)));
                    vec![Piece::Block(image_block(url, None, alt, title))]
                }
            };
        }
        if !image_allowed(url) {
            self.warnings.add(W::UnsafeLink, Some(line_col(node)));
            return if alt.is_empty() {
                vec![]
            } else {
                vec![Piece::Inline(text(alt))]
            };
        }
        let attachment = attachment_ref(url, origin).filter(|id| self.lookup.attachment(id));
        let src = attachment
            .as_deref()
            .map_or_else(|| url.to_owned(), attachment_url);
        vec![Piece::Block(image_block(
            &src,
            attachment.as_deref(),
            alt,
            title,
        ))]
    }
}

enum LinkTarget {
    Mention(String),
    Mark(Value),
    Text,
}

fn image_block(src: &str, attachment: Option<&str>, alt: &str, title: &str) -> Value {
    let clean = |s: &str| {
        let s = s.replace(['\n', '\r'], " ");
        let s = s.trim();
        if s.is_empty() {
            Value::Null
        } else {
            Value::from(s)
        }
    };
    json!({"type": "image", "attrs": {
        "src": src,
        "attachmentId": attachment,
        "alt": clean(alt),
        "caption": clean(title),
    }})
}

fn is_br(h: &str) -> bool {
    let t: String = h
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    t == "<br>" || t == "<br/>"
}

/// The plain text of a node's inline children (an image's alt, a link's label).
fn plain_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut s = String::new();
    let mut stack: Vec<&'a AstNode<'a>> = node
        .children()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    while let Some(n) = stack.pop() {
        match &n.data().value {
            NodeValue::Text(t) => s.push_str(t),
            NodeValue::Code(c) => s.push_str(&c.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => s.push(' '),
            NodeValue::HtmlInline(h) => s.push_str(h),
            _ => {}
        }
        stack.extend(n.children().collect::<Vec<_>>().into_iter().rev());
    }
    s
}

fn split_pieces(pieces: Vec<Piece>) -> (Vec<Value>, Vec<Value>) {
    let mut inline = Vec::new();
    let mut blocks = Vec::new();
    for p in pieces {
        match p {
            Piece::Inline(v) => inline.push(v),
            Piece::Block(b) => blocks.push(b),
        }
    }
    (inline, blocks)
}

fn flatten(pieces: Vec<Piece>) -> Vec<Value> {
    pieces
        .into_iter()
        .filter_map(|p| match p {
            Piece::Inline(v) => Some(v),
            Piece::Block(_) => None,
        })
        .collect()
}

/// A paragraph of inline nodes, without hard breaks at its edges (the printer cannot write those).
fn paragraph_of(inline: Vec<Value>, w: &mut Warnings) -> Value {
    let mut nodes = inline;
    while nodes.first().is_some_and(|n| type_of(n) == "hardBreak") {
        nodes.remove(0);
        w.add(W::HardBreakDropped, None);
    }
    while nodes.last().is_some_and(|n| type_of(n) == "hardBreak") {
        nodes.pop();
        w.add(W::HardBreakDropped, None);
    }
    let mut out = Map::new();
    out.insert("type".into(), Value::from("paragraph"));
    if !nodes.is_empty() {
        out.insert("content".into(), Value::Array(nodes));
    }
    Value::Object(out)
}
