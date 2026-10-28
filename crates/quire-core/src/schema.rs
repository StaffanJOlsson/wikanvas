//! Document schema v1 as data, and normalization of ProseMirror JSON (ARCHITECTURE §6.4) (owner: W4)
//!
//! [`v1`] is the TipTap schema of `web/src/editor/schema.ts` as a table: every node (name, groups, content expression,
//! allowed marks, inline, atom, attrs with type and default) and every mark (name, rank, `excludes`, attrs), in
//! ProseMirror's order. `fixtures/schema/v1.json`, dumped from the live editor, must equal [`Schema::doc_schema`]
//! (`schema::tests::matches_tiptap_fixture`), and `GET /api/v1/schema/doc` serves the same JSON.
//!
//! - [`normalize`] turns raw JSON (y-tiptap's `yXmlFragmentToProsemirrorJSON`) into the shape of `editor.getJSON()`:
//!   every attr present with its default, unknown attrs dropped, marks with their attrs filled and sorted by rank,
//!   adjacent text nodes with equal marks joined. Nodes and marks unknown to v1 pass through unchanged.
//! - [`validate`] checks a ProseMirror PUT body against the schema (§6.4 step 1) and returns it normalized.
//!
//! Both walk with an explicit stack, so a deep document cannot overflow a worker thread's stack.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::api::{AttrSpec, DocSchema, MarkSpec, NodeSpec, SCHEMA_VERSION};

/// The block id attribute (TipTap UniqueID); heading anchors are `#h-<blockId>`.
pub const BLOCK_ID: &str = "blockId";
/// The deepest nesting a written document may have (D33).
pub const MAX_DEPTH: usize = 100;

/// The JSON type of an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrType {
    /// `string | null`.
    Str,
    /// `integer` (a JSON number without a fraction).
    Int,
    /// `boolean`.
    Bool,
    /// `number | null`.
    Num,
    /// `integer[] | null`.
    IntArray,
    /// One of the strings; `true` when null is allowed too.
    Enum(&'static [&'static str], bool),
}

impl AttrType {
    /// The type in the TypeScript notation of `AttrSpec.type`.
    pub fn describe(self) -> String {
        match self {
            AttrType::Str => "string | null".into(),
            AttrType::Int => "integer".into(),
            AttrType::Bool => "boolean".into(),
            AttrType::Num => "number | null".into(),
            AttrType::IntArray => "integer[] | null".into(),
            AttrType::Enum(values, nullable) => {
                let mut parts: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
                if nullable {
                    parts.push("null".into());
                }
                parts.join(" | ")
            }
        }
    }

    /// Whether `v` has this type; returns it in canonical form (integers as integers).
    pub fn check(self, v: &Value) -> Option<Value> {
        match (self, v) {
            (AttrType::Str | AttrType::Num | AttrType::IntArray, Value::Null) => Some(Value::Null),
            (AttrType::Enum(_, true), Value::Null) => Some(Value::Null),
            (AttrType::Str, Value::String(_)) => Some(v.clone()),
            (AttrType::Bool, Value::Bool(_)) => Some(v.clone()),
            (AttrType::Int, Value::Number(_)) => as_int(v).map(Value::from),
            (AttrType::Num, Value::Number(n)) => Some(canonical_number(n.as_f64()?)),
            (AttrType::IntArray, Value::Array(items)) => items
                .iter()
                .map(|i| as_int(i).map(Value::from))
                .collect::<Option<Vec<_>>>()
                .map(Value::Array),
            (AttrType::Enum(values, _), Value::String(s)) => {
                values.contains(&s.as_str()).then(|| v.clone())
            }
            _ => None,
        }
    }
}

/// A JSON number without a fraction, as i64.
fn as_int(v: &Value) -> Option<i64> {
    if let Some(i) = v.as_i64() {
        return Some(i);
    }
    let f = v.as_f64()?;
    (f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15).then_some(f as i64)
}

/// A number as JavaScript's JSON.stringify writes it: integral values without a fraction.
pub fn canonical_number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 {
        Value::from(f as i64)
    } else {
        serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

/// An attribute's default value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Default {
    Null,
    Int(i64),
    Bool(bool),
    Str(&'static str),
}

impl Default {
    pub fn to_json(self) -> Value {
        match self {
            Default::Null => Value::Null,
            Default::Int(i) => Value::from(i),
            Default::Bool(b) => Value::Bool(b),
            Default::Str(s) => Value::from(s),
        }
    }
}

/// An attribute of a node or mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: &'static str,
    pub ty: AttrType,
    pub default: Default,
}

/// A node type.
#[derive(Debug)]
pub struct NodeType {
    pub name: &'static str,
    /// Space-separated groups, e.g. `block list`.
    pub group: Option<&'static str>,
    /// The content expression; `None` for a leaf.
    pub content: Option<&'static str>,
    /// The marks allowed in the content (ProseMirror's `marks`); `None` is the default.
    pub marks: Option<&'static str>,
    pub inline: bool,
    pub atom: bool,
    pub attrs: &'static [Attr],
}

impl NodeType {
    /// Whether the node is in `group`.
    pub fn in_group(&self, group: &str) -> bool {
        self.group.is_some_and(|g| g.split(' ').any(|x| x == group))
    }

    /// Whether the content is inline (a textblock: paragraph, heading, code block).
    pub fn is_textblock(&self) -> bool {
        matches!(self.content, Some("inline*") | Some("text*"))
    }

    /// A block node: neither inline nor `doc`.
    pub fn is_block(&self) -> bool {
        !self.inline && self.name != "doc"
    }

    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }
}

/// A mark type.
#[derive(Debug)]
pub struct MarkType {
    pub name: &'static str,
    pub rank: u32,
    pub excludes: Option<&'static str>,
    pub inclusive: Option<bool>,
    pub attrs: &'static [Attr],
}

impl MarkType {
    /// Whether this mark excludes `other` (ProseMirror: `excludes` unset means only itself).
    pub fn excludes(&self, other: &MarkType) -> bool {
        match self.excludes {
            None => self.name == other.name,
            Some(spec) => spec.split(' ').any(|x| x == "_" || x == other.name),
        }
    }
}

const BID: Attr = Attr {
    name: BLOCK_ID,
    ty: AttrType::Str,
    default: Default::Null,
};
const fn str_attr(name: &'static str) -> Attr {
    Attr {
        name,
        ty: AttrType::Str,
        default: Default::Null,
    }
}
const ALIGN: &[&str] = &["left", "center", "right"];
/// The callout variants (§6.1).
pub const CALLOUT_VARIANTS: &[&str] = &["info", "tip", "warning", "danger"];
const CELL_ATTRS: &[Attr] = &[
    BID,
    Attr {
        name: "colspan",
        ty: AttrType::Int,
        default: Default::Int(1),
    },
    Attr {
        name: "rowspan",
        ty: AttrType::Int,
        default: Default::Int(1),
    },
    Attr {
        name: "colwidth",
        ty: AttrType::IntArray,
        default: Default::Null,
    },
    Attr {
        name: "align",
        ty: AttrType::Enum(ALIGN, true),
        default: Default::Null,
    },
];

const fn node(
    name: &'static str,
    group: Option<&'static str>,
    content: Option<&'static str>,
    inline: bool,
    atom: bool,
    attrs: &'static [Attr],
) -> NodeType {
    NodeType {
        name,
        group,
        content,
        marks: None,
        inline,
        atom,
        attrs,
    }
}

/// Schema v1's nodes, in ProseMirror's order (the order TipTap resolves the extensions in).
static NODES: &[NodeType] = &[
    node(
        "paragraph",
        Some("block"),
        Some("inline*"),
        false,
        false,
        &[BID],
    ),
    node(
        "mention",
        Some("inline"),
        None,
        true,
        true,
        &[str_attr("itemId")],
    ),
    node(
        "blockquote",
        Some("block"),
        Some("block+"),
        false,
        false,
        &[BID],
    ),
    node(
        "bulletList",
        Some("block list"),
        Some("listItem+"),
        false,
        false,
        &[BID],
    ),
    node("doc", None, Some("block+"), false, false, &[]),
    node("hardBreak", Some("inline"), None, true, true, &[]),
    node(
        "heading",
        Some("block"),
        Some("inline*"),
        false,
        false,
        &[
            BID,
            Attr {
                name: "level",
                ty: AttrType::Int,
                default: Default::Int(1),
            },
        ],
    ),
    node("horizontalRule", Some("block"), None, false, true, &[BID]),
    node(
        "listItem",
        None,
        Some("paragraph block*"),
        false,
        false,
        &[BID],
    ),
    node(
        "orderedList",
        Some("block list"),
        Some("listItem+"),
        false,
        false,
        &[
            BID,
            Attr {
                name: "start",
                ty: AttrType::Int,
                default: Default::Int(1),
            },
            str_attr("type"),
        ],
    ),
    node("text", Some("inline"), None, true, true, &[]),
    NodeType {
        name: "codeBlock",
        group: Some("block"),
        content: Some("text*"),
        marks: Some(""),
        inline: false,
        atom: false,
        attrs: &[BID, str_attr("language")],
    },
    node(
        "taskList",
        Some("block list"),
        Some("taskItem+"),
        false,
        false,
        &[BID],
    ),
    node(
        "taskItem",
        None,
        Some("paragraph block*"),
        false,
        false,
        &[
            BID,
            Attr {
                name: "checked",
                ty: AttrType::Bool,
                default: Default::Bool(false),
            },
        ],
    ),
    node(
        "table",
        Some("block"),
        Some("tableRow+"),
        false,
        false,
        &[BID],
    ),
    node(
        "tableRow",
        None,
        Some("(tableCell | tableHeader)*"),
        false,
        false,
        &[BID],
    ),
    node(
        "tableHeader",
        None,
        Some("block+"),
        false,
        false,
        CELL_ATTRS,
    ),
    node("tableCell", None, Some("block+"), false, false, CELL_ATTRS),
    node(
        "image",
        Some("block"),
        None,
        false,
        true,
        &[
            BID,
            str_attr("src"),
            str_attr("attachmentId"),
            Attr {
                name: "width",
                ty: AttrType::Num,
                default: Default::Null,
            },
            str_attr("alt"),
            str_attr("caption"),
        ],
    ),
    node(
        "callout",
        Some("block"),
        Some("block+"),
        false,
        false,
        &[
            BID,
            Attr {
                name: "variant",
                ty: AttrType::Enum(CALLOUT_VARIANTS, false),
                default: Default::Str("info"),
            },
        ],
    ),
    node(
        "boardEmbed",
        Some("block"),
        None,
        false,
        true,
        &[BID, str_attr("itemId")],
    ),
];

const fn mark(name: &'static str, rank: u32) -> MarkType {
    MarkType {
        name,
        rank,
        excludes: None,
        inclusive: None,
        attrs: &[],
    }
}

/// Schema v1's marks, in rank order.
static MARKS: &[MarkType] = &[
    MarkType {
        name: "link",
        rank: 0,
        excludes: None,
        inclusive: Some(true),
        attrs: &[str_attr("href"), str_attr("itemId")],
    },
    mark("bold", 1),
    MarkType {
        name: "code",
        rank: 2,
        excludes: Some("_"),
        inclusive: None,
        attrs: &[],
    },
    mark("italic", 3),
    mark("strike", 4),
    mark("underline", 5),
    mark("highlight", 6),
];

/// Schema v1 with its compiled content expressions.
pub struct Schema {
    pub nodes: &'static [NodeType],
    pub marks: &'static [MarkType],
    node_index: HashMap<&'static str, usize>,
    mark_index: HashMap<&'static str, usize>,
    content: Vec<Option<Nfa>>,
}

/// Schema v1.
pub fn v1() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| Schema::new(NODES, MARKS))
}

impl Schema {
    fn new(nodes: &'static [NodeType], marks: &'static [MarkType]) -> Schema {
        let node_index = nodes.iter().enumerate().map(|(i, n)| (n.name, i)).collect();
        let mark_index = marks.iter().enumerate().map(|(i, m)| (m.name, i)).collect();
        let mut schema = Schema {
            nodes,
            marks,
            node_index,
            mark_index,
            content: Vec::new(),
        };
        schema.content = nodes
            .iter()
            .map(|n| {
                n.content.map(|expr| {
                    Nfa::compile(expr, &schema)
                        .unwrap_or_else(|e| panic!("content expression of {}: {e}", n.name))
                })
            })
            .collect();
        schema
    }

    pub fn node(&self, name: &str) -> Option<&NodeType> {
        self.node_index.get(name).map(|&i| &self.nodes[i])
    }

    pub fn mark(&self, name: &str) -> Option<&MarkType> {
        self.mark_index.get(name).map(|&i| &self.marks[i])
    }

    fn node_id(&self, name: &str) -> Option<usize> {
        self.node_index.get(name).copied()
    }

    /// The node ids a content-expression name stands for: the node itself, or every node of the group.
    fn resolve(&self, name: &str) -> Vec<usize> {
        if let Some(i) = self.node_id(name) {
            return vec![i];
        }
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.in_group(name))
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether `children` (node names) is valid content for `parent`. Unknown names never are.
    pub fn content_valid(&self, parent: &str, children: &[&str]) -> bool {
        let Some(pi) = self.node_id(parent) else {
            return false;
        };
        let ids: Option<Vec<usize>> = children.iter().map(|c| self.node_id(c)).collect();
        let Some(ids) = ids else {
            return false;
        };
        match &self.content[pi] {
            None => ids.is_empty(),
            Some(nfa) => nfa.matches(&ids),
        }
    }

    /// The index of the first child at which `children` stops matching `parent`'s content, or `children.len()`
    /// when the sequence is a valid prefix but incomplete; `None` when it is valid.
    pub fn content_error(&self, parent: &str, children: &[&str]) -> Option<usize> {
        let pi = self.node_id(parent)?;
        let Some(nfa) = &self.content[pi] else {
            return if children.is_empty() { None } else { Some(0) };
        };
        let mut states = nfa.start();
        for (i, c) in children.iter().enumerate() {
            let Some(id) = self.node_id(c) else {
                return Some(i);
            };
            states = nfa.step(&states, id);
            if states.is_empty() {
                return Some(i);
            }
        }
        (!nfa.accepts(&states)).then_some(children.len())
    }

    /// Whether `mark` may appear in the content of `parent`.
    pub fn allows_mark(&self, parent: &NodeType, mark: &MarkType) -> bool {
        match parent.marks {
            None => parent.is_textblock() || parent.content.is_some_and(|c| c.contains("inline")),
            Some("_") => true,
            Some("") => false,
            Some(spec) => spec.split(' ').any(|m| m == mark.name),
        }
    }

    /// The schema as the API's `DocSchema` (`GET /api/v1/schema/doc`, `fixtures/schema/v1.json`).
    pub fn doc_schema(&self) -> DocSchema {
        let attrs = |attrs: &[Attr]| {
            attrs
                .iter()
                .map(|a| AttrSpec {
                    name: a.name.into(),
                    value_type: a.ty.describe(),
                    default: a.default.to_json(),
                })
                .collect()
        };
        DocSchema {
            schema_version: SCHEMA_VERSION,
            top_node: "doc".into(),
            nodes: self
                .nodes
                .iter()
                .map(|n| NodeSpec {
                    name: n.name.into(),
                    group: n.group.map(Into::into),
                    content: n.content.map(Into::into),
                    marks: n.marks.map(Into::into),
                    inline: n.inline,
                    atom: n.atom,
                    attrs: attrs(n.attrs),
                })
                .collect(),
            marks: self
                .marks
                .iter()
                .map(|m| MarkSpec {
                    name: m.name.into(),
                    rank: m.rank,
                    excludes: m.excludes.map(Into::into),
                    inclusive: m.inclusive,
                    attrs: attrs(m.attrs),
                })
                .collect(),
        }
    }
}

/// A compiled content expression: a Thompson NFA over node ids.
struct Nfa {
    /// Per state: edges labelled with a term (an index into `terms`) or epsilon (`None`).
    edges: Vec<Vec<(Option<usize>, usize)>>,
    terms: Vec<Vec<usize>>,
    accept: usize,
}

enum Expr {
    Name(Vec<usize>),
    Seq(Vec<Expr>),
    Choice(Vec<Expr>),
    Repeat(Box<Expr>, usize, Option<usize>),
}

impl Nfa {
    fn compile(src: &str, schema: &Schema) -> Result<Nfa, String> {
        let tokens = tokenize(src);
        let mut pos = 0;
        let expr = parse_choice(&tokens, &mut pos, schema)?;
        if pos != tokens.len() {
            return Err(format!("unexpected {:?}", tokens[pos]));
        }
        let mut nfa = Nfa {
            edges: vec![Vec::new()],
            terms: Vec::new(),
            accept: 0,
        };
        nfa.accept = nfa.build(&expr, 0);
        Ok(nfa)
    }

    fn state(&mut self) -> usize {
        self.edges.push(Vec::new());
        self.edges.len() - 1
    }

    fn build(&mut self, e: &Expr, from: usize) -> usize {
        match e {
            Expr::Name(ids) => {
                let to = self.state();
                self.terms.push(ids.clone());
                let t = self.terms.len() - 1;
                self.edges[from].push((Some(t), to));
                to
            }
            Expr::Seq(items) => items.iter().fold(from, |at, item| self.build(item, at)),
            Expr::Choice(alts) => {
                let end = self.state();
                for alt in alts {
                    let e = self.build(alt, from);
                    self.edges[e].push((None, end));
                }
                end
            }
            Expr::Repeat(inner, min, max) => {
                let mut at = from;
                for _ in 0..*min {
                    at = self.build(inner, at);
                }
                match max {
                    None => {
                        let lp = self.state();
                        self.edges[at].push((None, lp));
                        let e = self.build(inner, lp);
                        self.edges[e].push((None, lp));
                        lp
                    }
                    Some(max) => {
                        let end = self.state();
                        self.edges[at].push((None, end));
                        for _ in *min..*max {
                            at = self.build(inner, at);
                            self.edges[at].push((None, end));
                        }
                        end
                    }
                }
            }
        }
    }

    fn closure(&self, mut states: Vec<usize>) -> Vec<usize> {
        let mut seen = vec![false; self.edges.len()];
        for &s in &states {
            seen[s] = true;
        }
        let mut i = 0;
        while i < states.len() {
            let s = states[i];
            for &(label, to) in &self.edges[s] {
                if label.is_none() && !seen[to] {
                    seen[to] = true;
                    states.push(to);
                }
            }
            i += 1;
        }
        states
    }

    fn start(&self) -> Vec<usize> {
        self.closure(vec![0])
    }

    fn step(&self, states: &[usize], id: usize) -> Vec<usize> {
        let mut next = Vec::new();
        for &s in states {
            for &(label, to) in &self.edges[s] {
                if let Some(t) = label
                    && self.terms[t].contains(&id)
                    && !next.contains(&to)
                {
                    next.push(to);
                }
            }
        }
        self.closure(next)
    }

    fn accepts(&self, states: &[usize]) -> bool {
        states.contains(&self.accept)
    }

    fn matches(&self, ids: &[usize]) -> bool {
        let mut states = self.start();
        for &id in ids {
            states = self.step(&states, id);
            if states.is_empty() {
                return false;
            }
        }
        self.accepts(&states)
    }
}

fn tokenize(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in src.chars() {
        if c.is_alphanumeric() || c == '_' || c == ',' && cur.starts_with('{') {
            cur.push(c);
        } else if c == '{' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        } else if c == '}' {
            cur.push(c);
            out.push(std::mem::take(&mut cur));
        } else {
            if !cur.is_empty() && !cur.starts_with('{') {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_choice(t: &[String], pos: &mut usize, schema: &Schema) -> Result<Expr, String> {
    let mut alts = vec![parse_seq(t, pos, schema)?];
    while t.get(*pos).is_some_and(|x| x == "|") {
        *pos += 1;
        alts.push(parse_seq(t, pos, schema)?);
    }
    Ok(if alts.len() == 1 {
        alts.pop().expect("one")
    } else {
        Expr::Choice(alts)
    })
}

fn parse_seq(t: &[String], pos: &mut usize, schema: &Schema) -> Result<Expr, String> {
    let mut items = Vec::new();
    while let Some(tok) = t.get(*pos) {
        if tok == ")" || tok == "|" {
            break;
        }
        let mut atom = if tok == "(" {
            *pos += 1;
            let e = parse_choice(t, pos, schema)?;
            if t.get(*pos).map(String::as_str) != Some(")") {
                return Err("missing )".into());
            }
            *pos += 1;
            e
        } else {
            *pos += 1;
            let ids = schema.resolve(tok);
            if ids.is_empty() {
                return Err(format!("no node or group named {tok}"));
            }
            Expr::Name(ids)
        };
        while let Some(op) = t.get(*pos) {
            let (min, max) = match op.as_str() {
                "*" => (0, None),
                "+" => (1, None),
                "?" => (0, Some(1)),
                s if s.starts_with('{') => {
                    let inner = &s[1..s.len() - 1];
                    let mut parts = inner.split(',');
                    let min = parts
                        .next()
                        .and_then(|x| x.trim().parse().ok())
                        .ok_or("bad range")?;
                    let max = match parts.next() {
                        None => Some(min),
                        Some(x) if x.trim().is_empty() => None,
                        Some(x) => Some(x.trim().parse().map_err(|_| "bad range")?),
                    };
                    (min, max)
                }
                _ => break,
            };
            *pos += 1;
            atom = Expr::Repeat(Box::new(atom), min, max);
        }
        items.push(atom);
    }
    if items.is_empty() {
        return Err("empty expression".into());
    }
    Ok(if items.len() == 1 {
        items.pop().expect("one")
    } else {
        Expr::Seq(items)
    })
}

/// The type name of a JSON node (`""` when it has none).
pub fn type_of(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

/// The children of a JSON node (empty when it has none).
pub fn content_of(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A node's normalized attrs: every schema attr, from `given` or its default; unknown keys dropped.
pub fn normal_attrs(attrs: &[Attr], given: Option<&Map<String, Value>>) -> Map<String, Value> {
    attrs
        .iter()
        .map(|a| {
            let v = given
                .and_then(|g| g.get(a.name))
                .filter(|v| !v.is_null())
                .map(|v| match v {
                    Value::Number(n) => n.as_f64().map_or(v.clone(), canonical_number),
                    _ => v.clone(),
                })
                .unwrap_or_else(|| a.default.to_json());
            (a.name.to_owned(), v)
        })
        .collect()
}

/// A normalized mark: `{type, attrs?}` with every attr filled; unknown marks unchanged.
pub fn normal_mark(mark: &Value) -> Value {
    let schema = v1();
    let Some(mt) = schema.mark(type_of(mark)) else {
        return mark.clone();
    };
    let mut out = Map::new();
    out.insert("type".into(), Value::from(mt.name));
    if !mt.attrs.is_empty() {
        let given = mark.get("attrs").and_then(Value::as_object);
        out.insert("attrs".into(), Value::Object(normal_attrs(mt.attrs, given)));
    }
    Value::Object(out)
}

/// Sorts marks by rank (unknown marks after the known ones, in their order) and normalizes them.
pub fn normal_marks(marks: &[Value]) -> Vec<Value> {
    let schema = v1();
    let mut out: Vec<Value> = marks.iter().map(normal_mark).collect();
    out.sort_by_key(|m| schema.mark(type_of(m)).map_or(u32::MAX, |t| t.rank));
    out
}

/// Appends `node` to `out`, joining it with a preceding text node that has the same marks.
pub fn push_inline(out: &mut Vec<Value>, node: Value) {
    if type_of(&node) == "text"
        && let Some(last) = out.last_mut()
        && type_of(last) == "text"
        && last.get("marks") == node.get("marks")
        && let (Some(Value::String(a)), Some(Value::String(b))) = (
            last.as_object_mut().and_then(|o| o.get_mut("text")),
            node.get("text"),
        )
    {
        a.push_str(b);
        return;
    }
    out.push(node);
}

/// Builds a normalized element from its source node and its already-normalized children.
fn normal_element(src: &Value, children: Vec<Value>) -> Value {
    let schema = v1();
    let name = type_of(src);
    let Some(nt) = schema.node(name) else {
        // Unknown to v1: unchanged, except the children (which may be known).
        let mut out = src.as_object().cloned().unwrap_or_default();
        if !children.is_empty() {
            out.insert("content".into(), Value::Array(children));
        }
        return Value::Object(out);
    };
    let mut out = Map::new();
    out.insert("type".into(), Value::from(nt.name));
    if !nt.attrs.is_empty() {
        let given = src.get("attrs").and_then(Value::as_object);
        out.insert("attrs".into(), Value::Object(normal_attrs(nt.attrs, given)));
    }
    if !children.is_empty() {
        out.insert("content".into(), Value::Array(children));
    }
    Value::Object(out)
}

/// A normalized text node, or `None` for an empty one.
fn normal_text(src: &Value) -> Option<Value> {
    let text = src.get("text").and_then(Value::as_str).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert("type".into(), Value::from("text"));
    out.insert("text".into(), Value::from(text));
    let marks = src
        .get("marks")
        .and_then(Value::as_array)
        .map(|m| normal_marks(m))
        .unwrap_or_default();
    if !marks.is_empty() {
        out.insert("marks".into(), Value::Array(marks));
    }
    Some(Value::Object(out))
}

/// Raw y-tiptap JSON → the normalized shape of `editor.getJSON()` (module docs). A raw `content` entry that is an
/// array (a top-level Y text, which y-tiptap does not flatten) is flattened.
pub fn normalize(raw: &Value) -> Value {
    struct Frame<'a> {
        src: &'a Value,
        pending: Vec<&'a Value>,
        next: usize,
        out: Vec<Value>,
    }
    fn flat(content: &[Value]) -> Vec<&Value> {
        let mut v = Vec::with_capacity(content.len());
        for c in content {
            match c {
                Value::Array(items) => v.extend(items.iter()),
                other => v.push(other),
            }
        }
        v
    }
    let mut stack = vec![Frame {
        src: raw,
        pending: flat(content_of(raw)),
        next: 0,
        out: Vec::new(),
    }];
    loop {
        let top = stack.last_mut().expect("non-empty");
        if let Some(&child) = top.pending.get(top.next) {
            top.next += 1;
            if type_of(child) == "text" {
                if let Some(t) = normal_text(child) {
                    push_inline(&mut top.out, t);
                }
            } else {
                stack.push(Frame {
                    src: child,
                    pending: flat(content_of(child)),
                    next: 0,
                    out: Vec::new(),
                });
            }
            continue;
        }
        let frame = stack.pop().expect("non-empty");
        let node = normal_element(frame.src, frame.out);
        match stack.last_mut() {
            Some(parent) => push_inline(&mut parent.out, node),
            None => return node,
        }
    }
}

/// Whether a JSON document holds a node or mark that schema v1 does not know (written by a newer client).
pub fn has_unknown(doc: &Value) -> bool {
    let schema = v1();
    let mut stack = vec![doc];
    while let Some(n) = stack.pop() {
        let t = type_of(n);
        if t != "text" && schema.node(t).is_none() {
            return true;
        }
        if let Some(marks) = n.get("marks").and_then(Value::as_array)
            && marks.iter().any(|m| schema.mark(type_of(m)).is_none())
        {
            return true;
        }
        for c in content_of(n) {
            match c {
                Value::Array(items) => stack.extend(items.iter()),
                other => stack.push(other),
            }
        }
    }
    false
}

/// Why a ProseMirror PUT body is not schema v1: `details {path, reason}` of 422 `invalid_document`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    /// A JSON pointer into the request body.
    pub path: String,
    pub reason: String,
}

impl Invalid {
    pub fn new(path: impl Into<String>, reason: impl Into<String>) -> Invalid {
        Invalid {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

/// A link or image URL check (the §6.4 scheme allow-list), used by [`validate`].
pub type UrlCheck = fn(&str) -> bool;

/// Validates `doc` (the `doc` field of a ProseMirror PUT, at JSON pointer `pointer`) against schema v1 and returns
/// it normalized (§6.4 step 1): an unknown node, mark or attr, a child the content expression forbids, a mark the
/// parent does not allow, a wrong attr type, a mark combination an `excludes` rule forbids, an empty text node, a
/// link (`link_ok`) or external image (`image_ok`) outside the scheme allow-list, or nesting deeper than
/// [`MAX_DEPTH`] is an [`Invalid`].
pub fn validate(
    doc: &Value,
    pointer: &str,
    link_ok: UrlCheck,
    image_ok: UrlCheck,
) -> Result<Value, Invalid> {
    let schema = v1();
    struct Frame<'a> {
        src: &'a Value,
        nt: &'static NodeType,
        path: String,
        children: &'a [Value],
        next: usize,
        out: Vec<Value>,
        names: Vec<&'static str>,
    }
    let root = check_element(schema, doc, pointer, true)?;
    if root.name != "doc" {
        return Err(Invalid::new(
            format!("{pointer}/type"),
            "the root node must be doc",
        ));
    }
    let mut stack = vec![Frame {
        src: doc,
        nt: root,
        path: pointer.to_owned(),
        children: children_of(doc, pointer)?,
        next: 0,
        out: Vec::new(),
        names: Vec::new(),
    }];
    loop {
        let depth = stack.len();
        let top = stack.last_mut().expect("non-empty");
        if let Some(child) = top.children.get(top.next) {
            let path = format!("{}/content/{}", top.path, top.next);
            top.next += 1;
            if !child.is_object() {
                return Err(Invalid::new(path, "a node is a JSON object"));
            }
            if type_of(child) == "text" {
                let text = check_text(schema, child, top.nt, &path, link_ok)?;
                top.names.push("text");
                push_inline(&mut top.out, text);
                continue;
            }
            let nt = check_element(schema, child, &path, false)?;
            if depth > MAX_DEPTH {
                return Err(Invalid::new(
                    path,
                    format!("the document is nested deeper than {MAX_DEPTH} levels"),
                ));
            }
            top.names.push(nt.name);
            if nt.name == "image" {
                check_image(child, &path, image_ok)?;
            }
            let children = children_of(child, &path)?;
            stack.push(Frame {
                src: child,
                nt,
                path,
                children,
                next: 0,
                out: Vec::new(),
                names: Vec::new(),
            });
            continue;
        }
        let frame = stack.pop().expect("non-empty");
        if let Some(i) = schema.content_error(frame.nt.name, &frame.names) {
            let reason = match frame.names.get(i) {
                Some(child) => format!("{} cannot contain {child} here", frame.nt.name),
                None if frame.names.is_empty() => format!("{} cannot be empty", frame.nt.name),
                None => format!("the content of {} is incomplete", frame.nt.name),
            };
            let path = if i < frame.children.len() {
                format!("{}/content/{i}", frame.path)
            } else {
                format!("{}/content", frame.path)
            };
            return Err(Invalid::new(path, reason));
        }
        let node = normal_element(frame.src, frame.out);
        match stack.last_mut() {
            Some(parent) => push_inline(&mut parent.out, node),
            None => return Ok(node),
        }
    }
}

fn children_of<'a>(node: &'a Value, path: &str) -> Result<&'a [Value], Invalid> {
    match node.get("content") {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(Invalid::new(
            format!("{path}/content"),
            "content is an array",
        )),
    }
}

fn check_element(
    schema: &'static Schema,
    node: &Value,
    path: &str,
    root: bool,
) -> Result<&'static NodeType, Invalid> {
    let Some(obj) = node.as_object() else {
        return Err(Invalid::new(path, "a node is a JSON object"));
    };
    let name = match obj.get("type") {
        Some(Value::String(s)) => s.as_str(),
        _ => return Err(Invalid::new(format!("{path}/type"), "a node needs a type")),
    };
    let Some(nt) = schema.node(name) else {
        return Err(Invalid::new(
            format!("{path}/type"),
            format!("unknown node type {name}"),
        ));
    };
    if !root && nt.name == "doc" {
        return Err(Invalid::new(format!("{path}/type"), "doc is only the root"));
    }
    for key in obj.keys() {
        if !matches!(key.as_str(), "type" | "attrs" | "content") {
            return Err(Invalid::new(
                format!("{path}/{key}"),
                format!("unexpected field {key}"),
            ));
        }
    }
    check_attrs(
        nt.attrs,
        obj.get("attrs"),
        &format!("{path}/attrs"),
        nt.name,
    )?;
    if nt.content.is_none()
        && obj
            .get("content")
            .is_some_and(|c| c.as_array().is_some_and(|a| !a.is_empty()))
    {
        return Err(Invalid::new(
            format!("{path}/content"),
            format!("{} has no content", nt.name),
        ));
    }
    Ok(nt)
}

fn check_attrs(
    attrs: &[Attr],
    given: Option<&Value>,
    path: &str,
    owner: &str,
) -> Result<(), Invalid> {
    let given = match given {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err(Invalid::new(path, "attrs is an object")),
    };
    for (key, v) in given {
        let Some(a) = attrs.iter().find(|a| a.name == key) else {
            return Err(Invalid::new(
                format!("{path}/{key}"),
                format!("{owner} has no attribute {key}"),
            ));
        };
        if a.ty.check(v).is_none() {
            return Err(Invalid::new(
                format!("{path}/{key}"),
                format!("{owner}.{key} must be {}", a.ty.describe()),
            ));
        }
    }
    Ok(())
}

fn check_text(
    schema: &'static Schema,
    node: &Value,
    parent: &NodeType,
    path: &str,
    link_ok: UrlCheck,
) -> Result<Value, Invalid> {
    let obj = node.as_object().expect("checked by the caller");
    for key in obj.keys() {
        if !matches!(key.as_str(), "type" | "text" | "marks") {
            return Err(Invalid::new(
                format!("{path}/{key}"),
                format!("unexpected field {key}"),
            ));
        }
    }
    match obj.get("text") {
        Some(Value::String(s)) if !s.is_empty() => {}
        Some(Value::String(_)) => {
            return Err(Invalid::new(
                format!("{path}/text"),
                "empty text nodes are not allowed",
            ));
        }
        _ => {
            return Err(Invalid::new(
                format!("{path}/text"),
                "a text node needs text",
            ));
        }
    }
    let marks = match obj.get("marks") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(m)) => m.as_slice(),
        Some(_) => return Err(Invalid::new(format!("{path}/marks"), "marks is an array")),
    };
    let mut seen: Vec<&MarkType> = Vec::new();
    for (i, m) in marks.iter().enumerate() {
        let mpath = format!("{path}/marks/{i}");
        let Some(mobj) = m.as_object() else {
            return Err(Invalid::new(mpath, "a mark is a JSON object"));
        };
        for key in mobj.keys() {
            if !matches!(key.as_str(), "type" | "attrs") {
                return Err(Invalid::new(
                    format!("{mpath}/{key}"),
                    format!("unexpected field {key}"),
                ));
            }
        }
        let name = type_of(m);
        let Some(mt) = schema.mark(name) else {
            return Err(Invalid::new(
                format!("{mpath}/type"),
                format!("unknown mark type {name}"),
            ));
        };
        if !schema.allows_mark(parent, mt) {
            return Err(Invalid::new(
                format!("{mpath}/type"),
                format!("{} allows no {name} mark", parent.name),
            ));
        }
        check_attrs(mt.attrs, m.get("attrs"), &format!("{mpath}/attrs"), mt.name)?;
        if let Some(other) = seen.iter().find(|o| o.excludes(mt) || mt.excludes(o)) {
            return Err(Invalid::new(
                mpath,
                format!("{name} cannot be combined with {}", other.name),
            ));
        }
        if mt.name == "link" {
            let href = m
                .get("attrs")
                .and_then(|a| a.get("href"))
                .and_then(Value::as_str);
            if !href.is_some_and(link_ok) {
                return Err(Invalid::new(
                    format!("{mpath}/attrs/href"),
                    "a link needs an http, https, mailto or quire URL, a path or a fragment",
                ));
            }
        }
        seen.push(mt);
    }
    Ok(normal_text(node).expect("not empty"))
}

fn check_image(node: &Value, path: &str, image_ok: UrlCheck) -> Result<(), Invalid> {
    let attrs = node.get("attrs");
    let attachment = attrs
        .and_then(|a| a.get("attachmentId"))
        .and_then(Value::as_str);
    let src = attrs.and_then(|a| a.get("src")).and_then(Value::as_str);
    if attachment.is_none()
        && let Some(src) = src
        && !image_ok(src)
    {
        return Err(Invalid::new(
            format!("{path}/attrs/src"),
            "an external image needs an http or https URL or a path",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture() -> Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/schema/v1.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixtures/schema/v1.json"))
            .expect("JSON")
    }

    #[test]
    fn matches_tiptap_fixture() {
        let ours = serde_json::to_value(v1().doc_schema()).unwrap();
        assert_eq!(
            ours,
            fixture(),
            "schema v1 differs from the TipTap dump; run scripts/dev.sh pnpm --dir web fixtures"
        );
    }

    #[test]
    fn every_mark_excludes_itself() {
        for m in v1().marks {
            assert!(
                m.excludes(m),
                "{} allows overlapping copies of itself",
                m.name
            );
        }
        let code = v1().mark("code").unwrap();
        for m in v1().marks {
            assert!(code.excludes(m), "code must exclude {}", m.name);
        }
    }

    #[test]
    fn content_expressions() {
        let s = v1();
        assert!(s.content_valid("doc", &["paragraph"]));
        assert!(!s.content_valid("doc", &[]));
        assert!(s.content_valid("listItem", &["paragraph", "bulletList", "codeBlock"]));
        assert!(!s.content_valid("listItem", &["heading"]));
        assert!(s.content_valid("tableRow", &[]));
        assert!(s.content_valid("tableRow", &["tableHeader", "tableCell"]));
        assert!(!s.content_valid("tableRow", &["paragraph"]));
        assert!(s.content_valid("paragraph", &["text", "hardBreak", "mention", "text"]));
        assert!(!s.content_valid("codeBlock", &["hardBreak"]));
        assert!(s.content_valid("codeBlock", &[]));
        assert!(!s.content_valid("image", &["text"]));
        assert_eq!(
            s.content_error("bulletList", &["listItem", "paragraph"]),
            Some(1)
        );
        assert_eq!(s.content_error("bulletList", &[]), Some(0));
    }

    #[test]
    fn normalize_fills_defaults_sorts_marks_and_joins_text() {
        let raw = json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"blockId": "b1", "level": 2.0, "stray": 1}, "content": [
                {"type": "text", "text": "a", "marks": [{"type": "italic", "attrs": {}}, {"type": "link", "attrs": {"href": "/x"}}]},
                {"type": "text", "text": "b", "marks": [{"type": "link", "attrs": {"href": "/x", "itemId": null}}, {"type": "italic"}]}
            ]},
            {"type": "future", "attrs": {"x": 1}, "content": [{"type": "paragraph"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "c", "marks": [{"type": "comment", "attrs": {"id": 1}}]}]}
        ]});
        let want = json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"blockId": "b1", "level": 2}, "content": [
                {"type": "text", "text": "ab", "marks": [{"type": "link", "attrs": {"href": "/x", "itemId": null}}, {"type": "italic"}]}
            ]},
            {"type": "future", "attrs": {"x": 1}, "content": [{"type": "paragraph", "attrs": {"blockId": null}}]},
            {"type": "paragraph", "attrs": {"blockId": null}, "content": [{"type": "text", "text": "c", "marks": [{"type": "comment", "attrs": {"id": 1}}]}]}
        ]});
        assert_eq!(normalize(&raw), want);
        assert!(has_unknown(&raw));
        assert!(!has_unknown(
            &json!({"type": "doc", "content": [{"type": "paragraph"}]})
        ));
    }

    fn ok(_: &str) -> bool {
        true
    }

    fn no_js(s: &str) -> bool {
        !s.starts_with("javascript:")
    }

    #[test]
    fn validate_reports_the_pointer() {
        let check = |doc: Value| validate(&doc, "/doc", no_js, ok);
        let p = |content: Value| json!({"type": "doc", "content": [{"type": "paragraph", "content": content}]});
        let err = |doc: Value| check(doc).unwrap_err();
        assert_eq!(
            err(json!({"type": "doc", "content": [{"type": "nope"}]})).path,
            "/doc/content/0/type"
        );
        assert_eq!(
            err(json!({"type": "doc", "content": [{"type": "bulletList", "content": [{"type": "paragraph"}]}]})).path,
            "/doc/content/0/content/0"
        );
        assert_eq!(
            err(json!({"type": "doc", "content": [{"type": "heading", "attrs": {"level": "2"}}]}))
                .path,
            "/doc/content/0/attrs/level"
        );
        assert_eq!(
            err(p(json!([{"type": "text", "text": "x", "marks": [{"type": "code"}, {"type": "bold"}]}]))).path,
            "/doc/content/0/content/0/marks/1"
        );
        assert_eq!(
            err(p(json!([{"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]}])))
                .path,
            "/doc/content/0/content/0/marks/0/attrs/href"
        );
        assert_eq!(
            err(json!({"type": "doc", "content": []})).path,
            "/doc/content"
        );
        assert_eq!(
            err(p(json!([{"type": "text", "text": ""}]))).path,
            "/doc/content/0/content/0/text"
        );
        let mut deep = json!({"type": "paragraph"});
        for _ in 0..101 {
            deep = json!({"type": "blockquote", "content": [deep]});
        }
        assert!(
            err(json!({"type": "doc", "content": [deep]}))
                .reason
                .contains("deeper")
        );
        let good = check(p(json!([{"type": "text", "text": "x", "marks": [{"type": "bold"}, {"type": "link", "attrs": {"href": "/a"}}]}])))
            .unwrap();
        assert_eq!(
            good["content"][0]["content"][0]["marks"],
            json!([{"type": "link", "attrs": {"href": "/a", "itemId": null}}, {"type": "bold"}])
        );
        assert_eq!(good["content"][0]["attrs"], json!({"blockId": null}));
    }
}
