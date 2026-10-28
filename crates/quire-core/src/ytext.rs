//! The Y walk: plain text, links, attachment ids and emptiness of a document (ARCHITECTURE §6.1) (owner: W4)
//!
//! [`walk`] reads `XmlFragment("default")` directly (no JSON in between, since it runs on every save) with an
//! explicit stack:
//! - **text:** every textblock (`paragraph`, `heading`, `codeBlock`, code included) is a line, a `hardBreak` ends a
//!   line, and an image's caption is a line of its own; a `tableRow` is one line whose cells are separated by `\t`
//!   (a cell's own lines joined by a space). Empty lines are dropped; the rest are joined by `\n`. Elements of other
//!   types are containers: their child blocks contribute lines (so a newer client's unknown block still has its text
//!   indexed).
//! - **links:** `mention.itemId` → `mention`, `boardEmbed.itemId` → `embed`, `link.itemId` → `link`, sorted by target
//!   then kind, without duplicates.
//! - **attachment ids:** every image `attachmentId`, sorted, without duplicates.
//! - **is_empty:** the fragment holds only empty paragraphs (y-tiptap writes one into an empty fragment when the
//!   editor starts, so empty bytes are not the test).
//!
//! The web fixture writer computes the same from `editor.getJSON()` (`fixtures/ydoc/*.txt`, `*.links.json`), which
//! `ytext::tests::golden_fixtures` compares against.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use yrs::types::text::YChange;
use yrs::{
    Any, Doc, Out, ReadTxn, Text, Transact, Xml, XmlElementRef, XmlFragment, XmlOut, XmlTextRef,
};

use crate::api::YFIELD;
use crate::convert::pm::mark_name;

/// The kind of an item link (lookup table `link_kinds`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Embed,
    Link,
    Mention,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Embed => "embed",
            LinkKind::Link => "link",
            LinkKind::Mention => "mention",
        }
    }
}

/// One item link of a document.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Link {
    pub target_id: String,
    pub kind: LinkKind,
}

/// What the walk derives from a document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Walk {
    /// The indexed body text.
    #[serde(skip)]
    pub text: String,
    pub links: Vec<Link>,
    pub attachment_ids: Vec<String>,
    pub is_empty: bool,
}

const TEXTBLOCKS: &[&str] = &["paragraph", "heading", "codeBlock"];

/// Walks the document's `XmlFragment("default")` (module docs).
pub fn walk(doc: &Doc) -> Walk {
    let frag = doc.get_or_insert_xml_fragment(YFIELD);
    let txn = doc.transact();
    walk_fragment(&txn, &frag)
}

/// [`walk`] over a fragment inside an open transaction.
pub fn walk_fragment<T: ReadTxn, F: XmlFragment>(txn: &T, frag: &F) -> Walk {
    let mut st = State::default();
    let top: Vec<XmlOut> = frag.children(txn).collect();
    let is_empty = top.iter().all(|c| match c {
        XmlOut::Element(e) => {
            e.tag().as_ref() == "paragraph"
                && e.children(txn)
                    .all(|c| matches!(&c, XmlOut::Text(t) if Text::len(t, txn) == 0))
        }
        XmlOut::Text(t) => Text::len(t, txn) == 0,
        XmlOut::Fragment(_) => true,
    });
    // Blocks to visit, in document order (a stack, so pushed in reverse).
    let mut stack: Vec<XmlOut> = top.into_iter().rev().collect();
    while let Some(node) = stack.pop() {
        let XmlOut::Element(e) = node else {
            continue;
        };
        let tag = e.tag().clone();
        match tag.as_ref() {
            t if TEXTBLOCKS.contains(&t) => {
                let lines = st.inline(txn, &e);
                st.lines.extend(lines);
            }
            "image" => {
                if let Some(caption) = st.image(txn, &e) {
                    st.lines.push(caption);
                }
            }
            "boardEmbed" => st.attr_link(txn, &e, "itemId", LinkKind::Embed),
            "tableRow" => {
                let cells: Vec<String> = e
                    .children(txn)
                    .filter_map(|c| match c {
                        XmlOut::Element(cell) => Some(st.cell(txn, &cell)),
                        _ => None,
                    })
                    .collect();
                st.lines.push(cells.join("\t"));
            }
            _ => stack.extend(e.children(txn).collect::<Vec<_>>().into_iter().rev()),
        }
    }
    let text = st
        .lines
        .into_iter()
        .filter(|l| l.chars().any(|c| c != '\t'))
        .collect::<Vec<_>>()
        .join("\n");
    Walk {
        text,
        links: st.links.into_iter().collect(),
        attachment_ids: st.attachments.into_iter().collect(),
        is_empty,
    }
}

#[derive(Default)]
struct State {
    lines: Vec<String>,
    links: BTreeSet<Link>,
    attachments: BTreeSet<String>,
}

fn string_attr<T: ReadTxn>(txn: &T, e: &XmlElementRef, name: &str) -> Option<String> {
    match e.get_attribute(txn, name)? {
        Out::Any(Any::String(s)) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

impl State {
    fn add_link(&mut self, target: String, kind: LinkKind) {
        self.links.insert(Link {
            target_id: target,
            kind,
        });
    }

    fn attr_link<T: ReadTxn>(&mut self, txn: &T, e: &XmlElementRef, attr: &str, kind: LinkKind) {
        if let Some(id) = string_attr(txn, e, attr) {
            self.add_link(id, kind);
        }
    }

    /// The image's attachment id (recorded) and its caption.
    fn image<T: ReadTxn>(&mut self, txn: &T, e: &XmlElementRef) -> Option<String> {
        if let Some(id) = string_attr(txn, e, "attachmentId") {
            self.attachments.insert(id);
        }
        string_attr(txn, e, "caption")
    }

    /// The lines of a textblock: its text, split at hard breaks.
    fn inline<T: ReadTxn>(&mut self, txn: &T, e: &XmlElementRef) -> Vec<String> {
        let mut lines = vec![String::new()];
        for c in e.children(txn) {
            match c {
                XmlOut::Text(t) => self.text(txn, &t, lines.last_mut().expect("one line")),
                XmlOut::Element(inline) => match inline.tag().as_ref() {
                    "hardBreak" => lines.push(String::new()),
                    "mention" => self.attr_link(txn, &inline, "itemId", LinkKind::Mention),
                    _ => {}
                },
                XmlOut::Fragment(_) => {}
            }
        }
        lines
    }

    fn text<T: ReadTxn>(&mut self, txn: &T, t: &XmlTextRef, line: &mut String) {
        for d in t.diff(txn, YChange::identity) {
            if let Out::Any(Any::String(s)) = &d.insert {
                line.push_str(s);
            }
            if let Some(attrs) = &d.attributes {
                for (k, v) in attrs.iter() {
                    if mark_name(k) == "link"
                        && let Any::Map(m) = v
                        && let Some(Any::String(id)) = m.get("itemId")
                        && !id.is_empty()
                    {
                        self.add_link(id.to_string(), LinkKind::Link);
                    }
                }
            }
        }
    }

    /// A table cell's text: the lines of its blocks (non-empty ones), joined by a space.
    fn cell<T: ReadTxn>(&mut self, txn: &T, cell: &XmlElementRef) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut stack: Vec<XmlOut> = cell
            .children(txn)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        while let Some(node) = stack.pop() {
            let XmlOut::Element(e) = node else {
                continue;
            };
            let tag = e.tag().clone();
            match tag.as_ref() {
                t if TEXTBLOCKS.contains(&t) => {
                    let lines = self.inline(txn, &e);
                    out.extend(lines.into_iter().filter(|l| !l.is_empty()));
                }
                "image" => {
                    if let Some(caption) = self.image(txn, &e) {
                        out.push(caption);
                    }
                }
                "boardEmbed" => self.attr_link(txn, &e, "itemId", LinkKind::Embed),
                _ => stack.extend(e.children(txn).collect::<Vec<_>>().into_iter().rev()),
            }
        }
        out.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use yrs::updates::decoder::Decode;
    use yrs::{Transact, Update};

    use super::*;

    fn ydoc_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ydoc")
    }

    fn load(bytes: &[u8]) -> Doc {
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(bytes).unwrap())
            .unwrap();
        doc
    }

    /// Every web-written `fixtures/ydoc/*.bin` (not `server/`).
    pub(crate) fn web_fixtures() -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(ydoc_dir()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "bin") {
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
        out.sort();
        assert!(
            out.len() >= 9,
            "the web fixture writer's documents are missing"
        );
        out
    }

    #[test]
    fn golden_fixtures() {
        for (name, bin) in web_fixtures() {
            let w = walk(&load(&bin));
            let dir = ydoc_dir();
            let text = std::fs::read_to_string(dir.join(format!("{name}.txt"))).unwrap();
            assert_eq!(w.text, text, "{name}.txt");
            let links: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(dir.join(format!("{name}.links.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&w).unwrap(),
                links,
                "{name}.links.json"
            );
        }
    }

    #[test]
    fn is_empty_with_editor_paragraph() {
        use yrs::{XmlElementPrelim, XmlTextPrelim};
        let empty = Doc::new();
        assert!(walk(&empty).is_empty, "an empty fragment");
        let with = |f: &dyn Fn(&mut yrs::TransactionMut, &yrs::XmlFragmentRef)| {
            let doc = Doc::new();
            let frag = doc.get_or_insert_xml_fragment(YFIELD);
            f(&mut doc.transact_mut(), &frag);
            walk(&doc)
        };
        let w = with(&|txn, frag| {
            frag.insert(txn, 0, XmlElementPrelim::empty("paragraph"));
        });
        assert!(w.is_empty, "the editor's one empty paragraph");
        let w = with(&|txn, frag| {
            let p = frag.insert(txn, 0, XmlElementPrelim::empty("paragraph"));
            p.insert(txn, 0, XmlTextPrelim::new(""));
            frag.insert(txn, 1, XmlElementPrelim::empty("paragraph"));
        });
        assert!(w.is_empty, "empty paragraphs, one with an empty text");
        let w = with(&|txn, frag| {
            let img = frag.insert(txn, 0, XmlElementPrelim::empty("image"));
            img.insert_attribute(txn, "src", "/api/v1/attachments/x");
        });
        assert!(!w.is_empty, "an image alone is not empty");
        assert_eq!(w.text, "");
        let w = with(&|txn, frag| {
            frag.insert(txn, 0, XmlElementPrelim::empty("horizontalRule"));
        });
        assert!(!w.is_empty);
        let w = with(&|txn, frag| {
            let p = frag.insert(txn, 0, XmlElementPrelim::empty("paragraph"));
            p.insert(txn, 0, XmlTextPrelim::new("x"));
        });
        assert!(!w.is_empty);
        assert_eq!(w.text, "x");
    }
}
