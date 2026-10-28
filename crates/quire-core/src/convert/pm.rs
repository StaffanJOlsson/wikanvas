//! Y XmlFragment <-> ProseMirror JSON, after @tiptap/y-tiptap (ARCHITECTURE §6.4) (owner: W4)
//!
//! Ported from `@tiptap/y-tiptap` 3.0.10 (MIT, © Kevin Jahns and Tiptap GmbH): `yXmlFragmentToProsemirrorJSON`
//! ([`raw`]), `createTypeFromElementNode` / `createTypeFromTextNodes` ([`insert_node`], [`insert_text`]) and
//! `yattr2markname` ([`mark_name`]). The mapping (§6.4):
//! - the document is `XmlFragment("default")`; each non-text node is an `XmlElement` whose tag is the node name, with
//!   one element attribute per attr whose value is not null (the JSON value itself);
//! - consecutive text nodes inside a block are one `XmlText`, whose delta chunks carry the marks as attributes
//!   `{<mark name>: <mark attrs object>}`; a `--<hash>` suffix on a key is stripped on read.
//!
//! One difference from y-tiptap: yrs reports a chunk's attributes as a map, so [`raw`] lists a text node's marks in
//! schema rank order (unknown marks last, by name), which is the order y-tiptap writes them in (it inserts every text
//! run with all its marks at once, in rank order); Yjs would list a mark added later to existing text last.
//!
//! Every walk uses an explicit stack. Text offsets are UTF-16 code units ([`new_doc`] creates documents so).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use yrs::types::Attrs;
use yrs::types::Delta;
use yrs::types::text::YChange;
use yrs::types::xml::{XmlDeltaPrelim, XmlIn};
use yrs::{
    Any, ClientID, Doc, In, OffsetKind, Options, Out, ReadTxn, Text, Transact, TransactionMut, Xml,
    XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};

use crate::api::YFIELD;
use crate::schema::{self, canonical_number, content_of, type_of, v1};

/// The deepest stored document the walkers accept (far beyond anything an editor produces; it bounds the memory and
/// the recursive drop of the JSON they build).
pub const MAX_WALK_DEPTH: usize = 5_000;

/// A new Y document for conversion: UTF-16 offsets, as in Yjs, and the given client id (random when `None`).
pub fn new_doc(client_id: Option<u32>) -> Doc {
    let mut options = match client_id {
        Some(id) => Options::with_client_id(ClientID::new(u64::from(id))),
        None => Options::default(),
    };
    options.offset_kind = OffsetKind::Utf16;
    Doc::with_options(options)
}

/// The document fragment.
pub fn fragment(doc: &Doc) -> XmlFragmentRef {
    doc.get_or_insert_xml_fragment(YFIELD)
}

/// A Yjs value as JSON, as JavaScript would serialize it (integral numbers without a fraction).
pub fn any_to_json(a: &Any) -> Value {
    match a {
        Any::Null | Any::Undefined => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(n) => match n {
            yrs::any::Number::Int(i) => Value::from(*i),
            yrs::any::Number::Float(f) => canonical_number(*f),
        },
        Any::String(s) => Value::from(s.as_ref()),
        Any::Buffer(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        Any::Array(items) => Value::Array(items.iter().map(any_to_json).collect()),
        Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), any_to_json(v))).collect()),
    }
}

/// JSON as a Yjs value. Numbers stay JavaScript numbers (never a BigInt).
pub fn json_to_any(v: &Value) -> Any {
    match v {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) if (-(1i64 << 53)..=(1i64 << 53)).contains(&i) => Any::from(i),
            _ => Any::from(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Any::from(s.as_str()),
        Value::Array(items) => Any::Array(items.iter().map(json_to_any).collect::<Vec<_>>().into()),
        Value::Object(m) => Any::Map(Arc::new(
            m.iter()
                .map(|(k, v)| (k.clone(), json_to_any(v)))
                .collect::<HashMap<_, _>>(),
        )),
    }
}

/// `yattr2markname`: a Y format key with its `--<8 base64 chars>` suffix (overlapping marks) removed.
pub fn mark_name(key: &str) -> &str {
    if key.len() > 10 && key.is_char_boundary(key.len() - 10) {
        let (base, suffix) = key.split_at(key.len() - 10);
        if let Some(hash) = suffix.strip_prefix("--")
            && hash.len() == 8
            && hash
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
            && !base.is_empty()
        {
            return base;
        }
    }
    key
}

/// The rank a format key sorts by in [`raw`] (module docs).
fn mark_order(key: &str) -> (u32, &str) {
    let name = mark_name(key);
    (v1().mark(name).map_or(u32::MAX, |m| m.rank), key)
}

/// The text nodes of one `XmlText` as raw JSON (`{type: "text", text, marks?}` per delta chunk).
pub fn text_chunks<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> Vec<Value> {
    let mut out = Vec::new();
    for d in text.diff(txn, YChange::identity) {
        let Out::Any(Any::String(s)) = &d.insert else {
            continue;
        };
        let mut node = Map::new();
        node.insert("type".into(), Value::from("text"));
        node.insert("text".into(), Value::from(s.as_ref()));
        if let Some(attrs) = &d.attributes {
            let mut keys: Vec<&Arc<str>> = attrs.keys().collect();
            keys.sort_by(|a, b| mark_order(a).cmp(&mark_order(b)));
            let marks = keys
                .into_iter()
                .map(|k| {
                    let mut m = Map::new();
                    m.insert("type".into(), Value::from(mark_name(k)));
                    m.insert("attrs".into(), any_to_json(&attrs[k]));
                    Value::Object(m)
                })
                .collect();
            node.insert("marks".into(), Value::Array(marks));
        }
        out.push(Value::Object(node));
    }
    out
}

/// An element's attributes as a JSON object.
pub fn element_attrs<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Map<String, Value> {
    el.attributes(txn)
        .map(|(k, v)| {
            let v = match v {
                Out::Any(a) => any_to_json(&a),
                _ => Value::Null,
            };
            (k.to_owned(), v)
        })
        .collect()
}

/// Why a stored document cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooDeep;

impl std::fmt::Display for TooDeep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the document is nested deeper than {MAX_WALK_DEPTH} levels"
        )
    }
}

/// `yXmlFragmentToProsemirrorJSON`: the raw JSON of the fragment (stored attrs only). A top-level `XmlText` stays an
/// array inside `content`, as in y-tiptap.
pub fn raw<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> Result<Value, TooDeep> {
    struct Frame {
        el: Option<XmlElementRef>,
        children: Vec<XmlOut>,
        next: usize,
        out: Vec<Value>,
    }
    let mut stack = vec![Frame {
        el: None,
        children: frag.children(txn).collect(),
        next: 0,
        out: Vec::new(),
    }];
    loop {
        let depth = stack.len();
        let top = stack.last_mut().expect("non-empty");
        if let Some(child) = top.children.get(top.next).cloned() {
            top.next += 1;
            let top_level = top.el.is_none();
            match child {
                XmlOut::Text(t) => {
                    let chunks = text_chunks(txn, &t);
                    if top_level {
                        top.out.push(Value::Array(chunks));
                    } else {
                        top.out.extend(chunks);
                    }
                }
                XmlOut::Element(e) => {
                    if depth > MAX_WALK_DEPTH {
                        return Err(TooDeep);
                    }
                    let children = e.children(txn).collect();
                    stack.push(Frame {
                        el: Some(e),
                        children,
                        next: 0,
                        out: Vec::new(),
                    });
                }
                XmlOut::Fragment(_) => {}
            }
            continue;
        }
        let frame = stack.pop().expect("non-empty");
        match frame.el {
            None => {
                let mut doc = Map::new();
                doc.insert("type".into(), Value::from("doc"));
                doc.insert("content".into(), Value::Array(frame.out));
                return Ok(Value::Object(doc));
            }
            Some(el) => {
                let mut node = Map::new();
                node.insert("type".into(), Value::from(el.tag().as_ref()));
                let attrs = element_attrs(txn, &el);
                if !attrs.is_empty() {
                    node.insert("attrs".into(), Value::Object(attrs));
                }
                if !frame.out.is_empty() {
                    node.insert("content".into(), Value::Array(frame.out));
                }
                stack
                    .last_mut()
                    .expect("the root frame is below")
                    .out
                    .push(Value::Object(node));
            }
        }
    }
}

/// The normalized JSON of the fragment: what `editor.getJSON()` returns (`schema::normalize(raw)`).
pub fn normalized<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> Result<Value, TooDeep> {
    Ok(schema::normalize(&raw(txn, frag)?))
}

/// The normalized JSON of a document.
pub fn read(doc: &Doc) -> Result<Value, TooDeep> {
    let frag = fragment(doc);
    let txn = doc.transact();
    normalized(&txn, &frag)
}

/// The Y format attributes of a text node's marks: `{<name>: <attrs object>}` (`{}` for a mark without attrs).
pub fn mark_attrs(marks: &[Value]) -> Attrs {
    marks
        .iter()
        .map(|m| {
            let attrs = m
                .get("attrs")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));
            (Arc::from(type_of(m)), json_to_any(&attrs))
        })
        .collect()
}

/// The marks of a text node (empty when it has none).
pub fn marks_of(node: &Value) -> &[Value] {
    node.get("marks")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A new `XmlText` holding `texts` (consecutive text nodes), as `createTypeFromTextNodes` builds it.
fn text_prelim(texts: &[&Value]) -> XmlDeltaPrelim {
    XmlDeltaPrelim {
        attributes: HashMap::default(),
        delta: texts
            .iter()
            .map(|t| {
                let s = t.get("text").and_then(Value::as_str).unwrap_or("");
                Delta::Inserted(
                    In::Any(Any::from(s)),
                    Some(Box::new(mark_attrs(marks_of(t)))),
                )
            })
            .collect(),
    }
}

/// Inserts consecutive text nodes as one new `XmlText` at `index` of `parent`.
pub fn insert_text<P: XmlFragment>(
    txn: &mut TransactionMut,
    parent: &P,
    index: u32,
    texts: &[&Value],
) -> XmlTextRef {
    parent
        .insert(txn, index, XmlIn::Text(text_prelim(texts)))
        .into_xml_text()
        .expect("an XmlText was inserted")
}

/// Sets the element attributes of `el` from a node's attrs (null attrs are not stored).
pub fn set_attrs(txn: &mut TransactionMut, el: &XmlElementRef, node: &Value) {
    if let Some(attrs) = node.get("attrs").and_then(Value::as_object) {
        for (k, v) in attrs {
            if !v.is_null() {
                el.insert_attribute(txn, k.as_str(), json_to_any(v));
            }
        }
    }
}

/// The Y children of a node's content: runs of consecutive text nodes, and every other node on its own.
pub fn child_groups(content: &[Value]) -> Vec<Vec<&Value>> {
    let mut groups: Vec<Vec<&Value>> = Vec::new();
    for c in content {
        let is_text = type_of(c) == "text";
        match groups.last_mut() {
            Some(g) if is_text && type_of(g[0]) == "text" => g.push(c),
            _ => groups.push(vec![c]),
        }
    }
    groups
}

/// Inserts `node` (normalized JSON, not a text node) as a new element at `index` of `parent`, with its whole subtree
/// (`createTypeFromElementNode`). Returns the element.
pub fn insert_node<P: XmlFragment>(
    txn: &mut TransactionMut,
    parent: &P,
    index: u32,
    node: &Value,
) -> XmlElementRef {
    let root = parent.insert(txn, index, XmlElementPrelim::empty(type_of(node)));
    set_attrs(txn, &root, node);
    let mut stack: Vec<(XmlElementRef, &Value)> = vec![(root.clone(), node)];
    while let Some((el, n)) = stack.pop() {
        for (i, group) in child_groups(content_of(n)).into_iter().enumerate() {
            let i = i as u32;
            if type_of(group[0]) == "text" {
                insert_text(txn, &el, i, &group);
            } else {
                let child = el.insert(txn, i, XmlElementPrelim::empty(type_of(group[0])));
                set_attrs(txn, &child, group[0]);
                stack.push((child, group[0]));
            }
        }
    }
    root
}

/// Writes a normalized document into a new Y document (client id `client_id`) and returns its full state.
pub fn write_doc(doc: &Value, client_id: u32) -> Vec<u8> {
    let ydoc = new_doc(Some(client_id));
    let frag = fragment(&ydoc);
    {
        let mut txn = ydoc.transact_mut();
        for (i, child) in content_of(doc).iter().enumerate() {
            insert_node(&mut txn, &frag, i as u32, child);
        }
    }
    ydoc.transact()
        .encode_state_as_update_v1(&yrs::StateVector::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_name_strips_the_hash_suffix() {
        assert_eq!(mark_name("link--AbCd12+/"), "link");
        assert_eq!(mark_name("link"), "link");
        assert_eq!(mark_name("--AbCd1234"), "--AbCd1234");
        assert_eq!(mark_name("bold--short"), "bold--short");
    }

    #[test]
    fn numbers_stay_javascript_numbers() {
        assert_eq!(any_to_json(&json_to_any(&Value::from(2))), Value::from(2));
        assert_eq!(any_to_json(&Any::from(2.0)), Value::from(2));
        assert_eq!(any_to_json(&Any::from(2.5)), serde_json::json!(2.5));
    }
}
