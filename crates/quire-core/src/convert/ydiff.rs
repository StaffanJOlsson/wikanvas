//! The diff writer: a target document applied to a base as one Yjs update (D50) (owner: W4)
//!
//! [`write`] diffs a normalized target against a base Y document, in one transaction on that document (§6.4 step 4):
//! - **children, per level:** anchors are `blockId` matches (JSON input only) and children with equal content hashes
//!   (name, attrs without `blockId`, content), of which the longest order-preserving set is kept (Myers' O((N+M)·D)
//!   diff). An anchored pair whose content differs (a `blockId` match) is updated in place; a `blockId` match out of
//!   order is deleted and re-inserted with its `blockId`. Between anchors, old and new children of the same name pair
//!   in order while their text similarity (common prefix plus suffix over the longer length) is at least 0.5 and are
//!   updated in place (attrs with `insert_attribute`/`remove_attribute`, children recursively); the rest are deleted
//!   or inserted.
//! - **text:** the common prefix and suffix (UTF-16 units, never splitting a surrogate pair) stay; the middle is
//!   deleted and the new middle inserted; then every range whose marks differ is formatted.
//! - **block ids:** a new block without a `blockId` gets a UUID v4 from `sha256(seed ‖ counter)`; a `blockId` sent in
//!   JSON is kept unless another block already has it.
//!
//! The comparison side is a tree annotated with the block map (`md_out::ORIGIN`, `CONSUMED`, `FILLER`): the base
//! itself for JSON input ([`annotate`]), the lowered base for Markdown input (`md_out::lower`), whose nodes stand for
//! base nodes (or for none). Base children that nothing stands for (empty paragraphs Markdown drops) stay in place.
//!
//! For Markdown input, a changed block paired with its base keeps what Markdown cannot express: its `blockId`,
//! `image.width`, `orderedList.type`, and a table's spans and widths when the rows keep their shape (otherwise the
//! table is replaced, with the warning `table_span_dropped`).
//!
//! Determinism: every text is inserted plain and formatted one mark at a time in name order, and attributes are set
//! in name order, so the same request against the same base gives the same update. (yrs encodes a map value's keys
//! in hash-map order, so a link mark with both `href` and `itemId` may differ in byte order between processes; its
//! content, client id and clocks do not.)

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use yrs::types::Attrs;
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Doc, Out, ReadTxn, StateVector, Text, Transact, TransactionMut, Update, Xml,
    XmlElementPrelim, XmlElementRef, XmlFragment, XmlOut, XmlTextPrelim, XmlTextRef,
};

use crate::convert::Warnings;
use crate::convert::md_out::{CONSUMED, FILLER, ORIGIN};
use crate::convert::pm::{self, any_to_json, element_attrs, json_to_any, mark_name};
use crate::schema::{self, BLOCK_ID, content_of, type_of, v1};

/// What a write produced.
#[derive(Clone, Debug)]
pub struct Output {
    /// The Yjs v1 update (empty when nothing changed).
    pub update: Vec<u8>,
    pub changed: bool,
    pub warnings: Warnings,
}

/// The transaction's client id: the first 4 bytes of `h` as a non-zero u32, re-hashed while it is 0 or already in
/// the base's state vector (§6.4 step 4).
pub fn client_id(h: &[u8; 32], base: &StateVector) -> u32 {
    let mut cur = *h;
    loop {
        let id = u32::from_be_bytes([cur[0], cur[1], cur[2], cur[3]]);
        if id != 0 && !base.contains_client(&yrs::ClientID::new(u64::from(id))) {
            return id;
        }
        cur = Sha256::digest(cur).into();
    }
}

/// The state vector of a document stored as updates, without loading it.
pub fn state_vector_of(updates: &[&[u8]]) -> Result<StateVector, String> {
    let mut sv = StateVector::default();
    for u in updates {
        let update = Update::decode_v1(u).map_err(|e| e.to_string())?;
        sv.merge(update.state_vector());
    }
    Ok(sv)
}

/// A UUID v4 string from 16 bytes.
fn uuid_v4(bytes: &[u8]) -> String {
    let mut b = [0u8; 16];
    b.copy_from_slice(&bytes[..16]);
    uuid::Builder::from_random_bytes(b).into_uuid().to_string()
}

/// The base document as a comparison tree for JSON input: its normalized JSON, each child annotated with its index
/// among its parent's element children (stray text at block level is dropped from the tree and left alone).
pub fn annotate(raw: &Value) -> Value {
    let mut root = base_tree(raw);
    let mut stack = vec![&mut root];
    while let Some(n) = stack.pop() {
        if let Some(Value::Array(children)) = n.get_mut("content") {
            for (i, c) in children.iter_mut().enumerate() {
                if let Some(o) = c.as_object_mut() {
                    o.insert(ORIGIN.into(), Value::from(i));
                }
            }
            stack.extend(children.iter_mut());
        }
    }
    root
}

/// The normalized base with block-level stray text removed, so content indexes are element ordinals.
pub fn base_tree(raw: &Value) -> Value {
    let mut doc = schema::normalize(&json!({"type": "doc", "content": Value::Array(
        content_of(raw).iter().filter(|c| c.is_object()).cloned().collect()
    )}));
    let mut stack = vec![&mut doc];
    while let Some(n) = stack.pop() {
        let textblock = v1().node(type_of(n)).is_some_and(|t| t.is_textblock());
        if textblock {
            continue;
        }
        if let Some(Value::Array(children)) = n.get_mut("content") {
            children.retain(|c| type_of(c) != "text");
            stack.extend(children.iter_mut());
        }
    }
    doc
}

/// A content hash: the node without `blockId`s and block-map keys.
fn hash(node: &Value) -> [u8; 32] {
    let mut n = node.clone();
    strip(&mut n);
    Sha256::digest(serde_json::to_vec(&n).unwrap_or_default()).into()
}

fn strip(node: &mut Value) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let Some(o) = n.as_object_mut() {
            o.remove(ORIGIN);
            o.remove(CONSUMED);
            o.remove(FILLER);
            if let Some(Value::Object(a)) = o.get_mut("attrs") {
                a.remove(BLOCK_ID);
            }
            if let Some(Value::Array(c)) = o.get_mut("content") {
                stack.extend(c.iter_mut());
            }
        }
    }
}

/// Whether two documents are equal ignoring `blockId`s (and block-map keys).
pub fn same(a: &Value, b: &Value) -> bool {
    hash(a) == hash(b)
}

fn block_id(node: &Value) -> Option<&str> {
    node.get("attrs")?.get(BLOCK_ID)?.as_str()
}

fn plain_text(node: &Value) -> Vec<char> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let Some(t) = n.get("text").and_then(Value::as_str) {
            out.extend(t.chars());
        }
        stack.extend(content_of(n).iter().rev());
    }
    out
}

/// Common prefix plus suffix over the longer length (1 when both are empty).
fn similarity(a: &[char], b: &[char]) -> f64 {
    let longer = a.len().max(b.len());
    if longer == 0 {
        return 1.0;
    }
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let rest = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(rest)
        .take_while(|(x, y)| x == y)
        .count();
    (prefix + suffix) as f64 / longer as f64
}

/// Myers' diff with a match predicate: the matched pairs `(i, j)`, in order. Gives up (keeping only the common
/// prefix and suffix) beyond `max_d` edits.
pub(crate) fn myers(
    n: usize,
    m: usize,
    eq: &dyn Fn(usize, usize) -> bool,
    max_d: usize,
) -> Vec<(usize, usize)> {
    let mut pre = 0;
    while pre < n && pre < m && eq(pre, pre) {
        pre += 1;
    }
    let mut suf = 0;
    while suf < n - pre && suf < m - pre && eq(n - 1 - suf, m - 1 - suf) {
        suf += 1;
    }
    let (n2, m2) = (n - pre - suf, m - pre - suf);
    let mut pairs: Vec<(usize, usize)> = (0..pre).map(|i| (i, i)).collect();
    let mid = if n2 == 0 || m2 == 0 {
        Vec::new()
    } else {
        myers_core(n2, m2, &|i, j| eq(pre + i, pre + j), max_d)
            .into_iter()
            .map(|(i, j)| (pre + i, pre + j))
            .collect()
    };
    pairs.extend(mid);
    pairs.extend((0..suf).map(|k| (n - suf + k, m - suf + k)));
    pairs
}

fn myers_core(
    n: usize,
    m: usize,
    eq: &dyn Fn(usize, usize) -> bool,
    max_d: usize,
) -> Vec<(usize, usize)> {
    let max = n + m;
    let off = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let limit = max.min(max_d);
    let mut found = None;
    for d in 0..=limit as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while (x as usize) < n && (y as usize) < m && y >= 0 && eq(x as usize, y as usize) {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x as usize >= n && y as usize >= m {
                found = Some(d);
                break;
            }
            k += 2;
        }
        if found.is_some() {
            trace.push(v.clone());
            break;
        }
    }
    let Some(dmax) = found else {
        return Vec::new();
    };
    // Backtrack.
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n as isize, m as isize);
    for d in (1..=dmax).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let idx = (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + off) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        x = prev_x;
        y = prev_y;
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        pairs.push((x as usize, y as usize));
    }
    pairs.reverse();
    pairs
}

/// The plan for one level of children.
#[derive(Debug)]
struct LevelPlan {
    /// Element ordinals of the base children to delete.
    deletes: Vec<usize>,
    /// The new children, in order.
    entries: Vec<Entry>,
}

#[derive(Debug)]
enum Entry {
    Keep(usize),
    Update(usize, Box<NodePlan>),
    Insert(Value),
}

#[derive(Debug)]
struct NodePlan {
    /// The attrs to set (normalized; null removes).
    attrs: Map<String, Value>,
    body: Body,
}

#[derive(Debug)]
enum Body {
    None,
    Inline(Vec<Value>),
    Level(LevelPlan),
    /// A table whose spans stay: (row ordinal, [(cell ordinal, the cell's children)]).
    Rows(Vec<(usize, Vec<(usize, LevelPlan)>)>),
}

struct Writer {
    seed: [u8; 32],
    counter: u64,
    markdown: bool,
    taken: HashSet<String>,
    warnings: Warnings,
    ops: usize,
}

/// Applies `target` (normalized) to `doc` (the base, created with the transaction's client id) and returns the
/// update. `b` is the comparison tree ([`annotate`] for JSON, the lowered base for Markdown).
pub fn write(doc: &Doc, b: &Value, target: &Value, markdown: bool, seed: [u8; 32]) -> Output {
    let frag = pm::fragment(doc);
    let mut w = Writer {
        seed,
        counter: 0,
        markdown,
        taken: HashSet::new(),
        warnings: Warnings::default(),
        ops: 0,
    };
    let update;
    {
        let mut txn = doc.transact_mut();
        let elems = elements(&txn, &frag);
        let plan = w.plan_level(&txn, &elems, content_of(b), content_of(target), "doc");
        // Ids that stay: every base id not in a deleted subtree.
        let mut base_ids = HashSet::new();
        collect_ids(b, &mut base_ids);
        let mut deleted = HashSet::new();
        collect_deleted(&plan, content_of(b), &mut deleted);
        w.taken = base_ids.difference(&deleted).cloned().collect();
        w.apply_level(&mut txn, &frag, plan);
        update = if w.ops > 0 {
            txn.encode_update_v1()
        } else {
            Vec::new()
        };
    }
    Output {
        changed: w.ops > 0,
        update,
        warnings: w.warnings,
    }
}

fn collect_ids(node: &Value, out: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let Some(id) = block_id(n) {
            out.insert(id.to_owned());
        }
        stack.extend(content_of(n).iter());
    }
}

fn collect_deleted(plan: &LevelPlan, b: &[Value], out: &mut HashSet<String>) {
    let by_origin = |ord: usize| b.iter().find(|n| origin(n) == Some(ord));
    for &d in &plan.deletes {
        if let Some(n) = by_origin(d) {
            collect_ids(n, out);
        }
    }
    for e in &plan.entries {
        if let Entry::Update(ord, np) = e
            && let Some(n) = by_origin(*ord)
        {
            match &np.body {
                Body::Level(lp) => collect_deleted(lp, content_of(n), out),
                Body::Rows(rows) => {
                    for (r, cells) in rows {
                        if let Some(row) = content_of(n).iter().find(|x| origin(x) == Some(*r)) {
                            for (c, lp) in cells {
                                if let Some(cell) =
                                    content_of(row).iter().find(|x| origin(x) == Some(*c))
                                {
                                    collect_deleted(lp, content_of(cell), out);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn origin(n: &Value) -> Option<usize> {
    n.get(ORIGIN).and_then(Value::as_u64).map(|x| x as usize)
}

fn consumed(n: &Value) -> Vec<usize> {
    n.get(CONSUMED)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_u64().map(|x| x as usize))
                .collect()
        })
        .unwrap_or_default()
}

fn is_filler(n: &Value) -> bool {
    n.get(FILLER).is_some()
}

/// The element children of a Y parent (ordinal → element).
fn elements<T: ReadTxn, P: XmlFragment>(txn: &T, parent: &P) -> Vec<XmlElementRef> {
    parent
        .children(txn)
        .filter_map(|c| match c {
            XmlOut::Element(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// A Y element's attrs, normalized for its type.
fn y_attrs<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Map<String, Value> {
    let raw = element_attrs(txn, el);
    match v1().node(el.tag()) {
        Some(nt) => schema::normal_attrs(nt.attrs, Some(&raw)),
        None => raw,
    }
}

fn is_textblock(name: &str) -> bool {
    v1().node(name).is_some_and(|n| n.is_textblock())
}

fn table_has_map(b: &Value) -> bool {
    content_of(b)
        .iter()
        .any(|row| origin(row).is_none() || content_of(row).iter().any(is_filler))
}

fn is_empty_cell(c: &Value) -> bool {
    content_of(c)
        .iter()
        .all(|p| type_of(p) == "paragraph" && content_of(p).is_empty())
}

/// Whether a target table keeps the shape (rows, cells, empty fillers) of a lowered base table with spans.
fn table_compatible(b: &Value, t: &Value) -> bool {
    let (br, tr) = (content_of(b), content_of(t));
    br.len() == tr.len()
        && br.iter().zip(tr).all(|(brow, trow)| {
            let (bc, tc) = (content_of(brow), content_of(trow));
            bc.len() == tc.len()
                && bc.iter().zip(tc).all(|(bcell, tcell)| {
                    if is_filler(bcell) || origin(brow).is_none() {
                        is_empty_cell(tcell)
                    } else {
                        true
                    }
                })
        })
}

impl Writer {
    fn new_block_id(&mut self) -> String {
        loop {
            let mut h = Sha256::new();
            h.update(self.seed);
            h.update(self.counter.to_be_bytes());
            self.counter += 1;
            let id = uuid_v4(&h.finalize());
            if self.taken.insert(id.clone()) {
                return id;
            }
        }
    }

    // ----------------------------------------------------------------------------------------------- planning

    fn plan_level<T: ReadTxn>(
        &mut self,
        txn: &T,
        ys: &[XmlElementRef],
        b: &[Value],
        t: &[Value],
        parent: &str,
    ) -> LevelPlan {
        let hb: Vec<[u8; 32]> = b.iter().map(hash).collect();
        let ht: Vec<[u8; 32]> = t.iter().map(hash).collect();
        let json = !self.markdown;
        let eq = |i: usize, j: usize| {
            if hb[i] == ht[j] {
                return true;
            }
            json && block_id(&b[i]).is_some() && block_id(&b[i]) == block_id(&t[j])
        };
        let anchors = myers(b.len(), t.len(), &eq, 2000);
        let target_ids: HashSet<&str> = t.iter().filter_map(block_id).collect();
        let base_ids: HashSet<&str> = b.iter().filter_map(block_id).collect();
        let texts_b: Vec<Vec<char>> = b.iter().map(plain_text).collect();
        let mut pairs: Vec<(usize, usize, bool)> = Vec::new(); // (i, j, anchored)
        let mut bi = 0;
        let mut tj = 0;
        let mut gaps = anchors.clone();
        gaps.push((b.len(), t.len()));
        for (ai, aj) in gaps {
            // Pair the gap b[bi..ai] with t[tj..aj].
            let mut x = bi;
            for j in tj..aj {
                let tj_node = &t[j];
                let tid = block_id(tj_node);
                if json && tid.is_some_and(|id| base_ids.contains(id)) {
                    continue;
                }
                let tt = plain_text(tj_node);
                let mut k = x;
                let window = ai.min(x + 64);
                while k < window {
                    let bk = &b[k];
                    let pairable = type_of(bk) == type_of(tj_node)
                        && !(json && block_id(bk).is_some_and(|id| target_ids.contains(id)))
                        && self.pairable(txn, ys, bk, tj_node)
                        // An empty base block (the editor's empty paragraph) takes any content.
                        && (texts_b[k].is_empty() && is_textblock(type_of(bk)) || similarity(&texts_b[k], &tt) >= 0.5);
                    if pairable {
                        break;
                    }
                    k += 1;
                }
                if k < window {
                    pairs.push((k, j, false));
                    x = k + 1;
                }
            }
            if ai < b.len() && aj < t.len() {
                pairs.push((ai, aj, true));
            }
            bi = ai + 1;
            tj = aj + 1;
        }
        pairs.sort_by_key(|p| p.1);
        let paired_b: HashSet<usize> = pairs.iter().map(|p| p.0).collect();
        let mut deletes: Vec<usize> = Vec::new();
        let mut entries: Vec<Entry> = Vec::new();
        for (i, node) in b.iter().enumerate() {
            if !paired_b.contains(&i) {
                match origin(node) {
                    Some(o) => deletes.push(o),
                    None => deletes.extend(consumed(node)),
                }
            }
        }
        let mut pi = 0;
        for (j, tnode) in t.iter().enumerate() {
            if pi < pairs.len() && pairs[pi].1 == j {
                let (i, _, anchored) = pairs[pi];
                pi += 1;
                let bnode = &b[i];
                match origin(bnode) {
                    Some(o) if o < ys.len() && ys[o].tag().as_ref() == type_of(tnode) => {
                        if anchored
                            && hb[i] == ht[j]
                            && !(json
                                && block_id(tnode).is_some_and(|id| Some(id) != block_id(bnode)))
                        {
                            entries.push(Entry::Keep(o));
                        } else {
                            let np = self.plan_node(txn, &ys[o], bnode, tnode);
                            entries.push(Entry::Update(o, Box::new(np)));
                        }
                    }
                    Some(o) => {
                        if anchored && hb[i] == ht[j] && self.markdown {
                            // Unchanged Markdown: never touched, whatever its type in the base.
                            entries.push(Entry::Keep(o));
                        } else {
                            deletes.push(o);
                            entries.push(Entry::Insert(tnode.clone()));
                        }
                    }
                    None => {
                        if anchored && hb[i] == ht[j] {
                            // A made-up node that stays as printed: the base nodes it stands for stay.
                            for c in consumed(bnode) {
                                entries.push(Entry::Keep(c));
                            }
                        } else {
                            deletes.extend(consumed(bnode));
                            if !(is_filler(bnode) && is_empty_cell(tnode)) {
                                entries.push(Entry::Insert(tnode.clone()));
                            }
                        }
                    }
                }
            } else {
                entries.push(Entry::Insert(tnode.clone()));
            }
        }
        let _ = parent;
        deletes.sort_unstable();
        deletes.dedup();
        // A Keep of a consumed node must not also be deleted.
        let kept: HashSet<usize> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::Keep(o) | Entry::Update(o, _) => Some(*o),
                Entry::Insert(_) => None,
            })
            .collect();
        deletes.retain(|d| !kept.contains(d));
        LevelPlan { deletes, entries }
    }

    /// Whether a base node may be updated in place with a target node (beyond equal names).
    fn pairable<T: ReadTxn>(&self, txn: &T, ys: &[XmlElementRef], b: &Value, t: &Value) -> bool {
        let Some(o) = origin(b) else {
            return true;
        };
        if o >= ys.len() || ys[o].tag().as_ref() != type_of(t) {
            return false;
        }
        let _ = txn;
        if self.markdown && type_of(t) == "table" && table_has_map(b) {
            return table_compatible(b, t);
        }
        true
    }

    fn plan_node<T: ReadTxn>(
        &mut self,
        txn: &T,
        y: &XmlElementRef,
        b: &Value,
        t: &Value,
    ) -> NodePlan {
        let name = type_of(t);
        let base_attrs = y_attrs(txn, y);
        let mut attrs = t
            .get("attrs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        // Block ids: Markdown keeps the base's; JSON a sent one (checked against the others when applied).
        let base_id = base_attrs.get(BLOCK_ID).cloned().unwrap_or(Value::Null);
        if base_attrs.contains_key(BLOCK_ID) {
            let sent = attrs.get(BLOCK_ID).filter(|v| v.is_string()).cloned();
            match sent {
                Some(id) if !self.markdown => {
                    attrs.insert(BLOCK_ID.into(), id);
                }
                _ => {
                    attrs.insert(BLOCK_ID.into(), base_id.clone());
                }
            }
        }
        if self.markdown {
            let keep: &[&str] = match name {
                "image" => &["width"],
                "orderedList" => &["type"],
                "tableCell" | "tableHeader" => &["colspan", "rowspan", "colwidth", "align"],
                _ => &[],
            };
            for k in keep {
                if let Some(v) = base_attrs.get(*k) {
                    attrs.insert((*k).to_owned(), v.clone());
                }
            }
        }
        let body = if is_textblock(name) {
            Body::Inline(content_of(t).to_vec())
        } else if v1().node(name).is_some_and(|n| n.content.is_some()) {
            let ys = elements(txn, y);
            if self.markdown && name == "table" && table_has_map(b) {
                // Spans stay: update each real cell's content in place.
                let mut rows = Vec::new();
                for (brow, trow) in content_of(b).iter().zip(content_of(t)) {
                    let Some(r) = origin(brow) else {
                        continue;
                    };
                    let Some(yrow) = ys.get(r) else {
                        continue;
                    };
                    let ycells = elements(txn, yrow);
                    let mut cells = Vec::new();
                    for (bcell, tcell) in content_of(brow).iter().zip(content_of(trow)) {
                        let Some(c) = origin(bcell) else {
                            continue;
                        };
                        if hash(bcell) == hash(tcell) {
                            continue;
                        }
                        let Some(ycell) = ycells.get(c) else {
                            continue;
                        };
                        let yc = elements(txn, ycell);
                        let lp = self.plan_level(
                            txn,
                            &yc,
                            content_of(bcell),
                            content_of(tcell),
                            type_of(tcell),
                        );
                        cells.push((c, lp));
                    }
                    if !cells.is_empty() {
                        rows.push((r, cells));
                    }
                }
                // Cell types and attrs stay as they are.
                return NodePlan {
                    attrs: base_attrs,
                    body: Body::Rows(rows),
                };
            }
            Body::Level(self.plan_level(txn, &ys, content_of(b), content_of(t), name))
        } else {
            Body::None
        };
        NodePlan { attrs, body }
    }

    // ----------------------------------------------------------------------------------------------- applying

    fn apply_level<P: XmlFragment>(
        &mut self,
        txn: &mut TransactionMut,
        parent: &P,
        plan: LevelPlan,
    ) {
        // Element ordinal → child index, before any change.
        let children: Vec<XmlOut> = parent.children(txn).collect();
        let mut elem_child: Vec<u32> = Vec::new();
        for (i, c) in children.iter().enumerate() {
            if matches!(c, XmlOut::Element(_)) {
                elem_child.push(i as u32);
            }
        }
        let elems: Vec<XmlElementRef> = children
            .iter()
            .filter_map(|c| match c {
                XmlOut::Element(e) => Some(e.clone()),
                _ => None,
            })
            .collect();
        // Deletions, last first.
        let mut removed: Vec<u32> = plan
            .deletes
            .iter()
            .filter_map(|&o| elem_child.get(o).copied())
            .collect();
        removed.sort_unstable();
        for &ci in removed.iter().rev() {
            parent.remove_range(txn, ci, 1);
            self.ops += 1;
        }
        // The live child list: original child indexes that remain, in order.
        let removed_set: HashSet<u32> = removed.iter().copied().collect();
        let mut live: Vec<Option<u32>> = (0..children.len() as u32)
            .filter(|i| !removed_set.contains(i))
            .map(Some)
            .collect();
        let deleted_ordinals: HashSet<usize> = plan.deletes.iter().copied().collect();
        let mut cursor = 0usize;
        let mut updates: Vec<(XmlElementRef, NodePlan)> = Vec::new();
        for entry in plan.entries {
            match entry {
                Entry::Keep(o) | Entry::Update(o, _) if deleted_ordinals.contains(&o) => {}
                Entry::Keep(o) => {
                    if let Some(&ci) = elem_child.get(o)
                        && let Some(p) = live.iter().position(|x| *x == Some(ci))
                    {
                        cursor = cursor.max(p + 1);
                    }
                }
                Entry::Update(o, np) => {
                    if let Some(&ci) = elem_child.get(o)
                        && let Some(p) = live.iter().position(|x| *x == Some(ci))
                    {
                        cursor = cursor.max(p + 1);
                        updates.push((elems[o].clone(), *np));
                    }
                }
                Entry::Insert(node) => {
                    self.insert_new(txn, parent, cursor as u32, &node);
                    live.insert(cursor, None);
                    cursor += 1;
                }
            }
        }
        for (el, np) in updates {
            self.apply_node(txn, &el, np);
        }
    }

    fn apply_node(&mut self, txn: &mut TransactionMut, el: &XmlElementRef, mut np: NodePlan) {
        // Attributes, in name order.
        let current = element_attrs(txn, el);
        let have_id = current
            .get(BLOCK_ID)
            .and_then(Value::as_str)
            .map(str::to_owned);
        let want_id = np
            .attrs
            .get(BLOCK_ID)
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(want) = want_id
            && have_id.as_deref() != Some(want.as_str())
        {
            if self.taken.insert(want.clone()) {
                if let Some(h) = &have_id {
                    self.taken.remove(h);
                }
            } else {
                // Another block has it: keep the current id.
                np.attrs.insert(
                    BLOCK_ID.into(),
                    have_id.clone().map_or(Value::Null, Value::from),
                );
            }
        }
        let mut keys: Vec<&String> = np.attrs.keys().chain(current.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let want = np.attrs.get(k).cloned().unwrap_or(Value::Null);
            let have = current.get(k).cloned().unwrap_or(Value::Null);
            if !np.attrs.contains_key(k) && v1().node(el.tag()).is_some_and(|n| n.attr(k).is_none())
            {
                // An attr the schema does not know: leave it.
                continue;
            }
            if want == have {
                continue;
            }
            if want.is_null() {
                el.remove_attribute(txn, k);
            } else {
                el.insert_attribute(txn, k.as_str(), json_to_any(&want));
            }
            self.ops += 1;
        }
        match np.body {
            Body::None => {}
            Body::Inline(content) => self.inline(txn, el, &content),
            Body::Level(lp) => self.apply_level(txn, el, lp),
            Body::Rows(rows) => {
                let yrows: Vec<XmlElementRef> = elements(txn, el);
                for (r, cells) in rows {
                    let Some(yrow) = yrows.get(r) else {
                        continue;
                    };
                    let ycells = elements(txn, yrow);
                    for (c, lp) in cells {
                        if let Some(ycell) = ycells.get(c) {
                            self.apply_level(txn, ycell, lp);
                        }
                    }
                }
            }
        }
    }

    /// Inserts a new block (with block ids assigned) at child index `at`.
    fn insert_new<P: XmlFragment>(
        &mut self,
        txn: &mut TransactionMut,
        parent: &P,
        at: u32,
        node: &Value,
    ) {
        let mut node = node.clone();
        crate::convert::md_out::strip_map(&mut node);
        // Block ids: keep a sent one unless taken, else a new one.
        {
            let mut stack = vec![&mut node];
            while let Some(n) = stack.pop() {
                let is_block = v1()
                    .node(type_of(n))
                    .is_some_and(|t| t.is_block() && t.attr(BLOCK_ID).is_some());
                if is_block && let Some(Value::Object(a)) = n.get_mut("attrs") {
                    let sent = a.get(BLOCK_ID).and_then(Value::as_str).map(str::to_owned);
                    let id = match sent {
                        Some(id) if !self.markdown && self.taken.insert(id.clone()) => id,
                        _ => self.new_block_id(),
                    };
                    a.insert(BLOCK_ID.into(), Value::from(id));
                }
                if let Some(Value::Array(c)) = n.get_mut("content") {
                    stack.extend(c.iter_mut());
                }
            }
        }
        let root = parent.insert(txn, at, XmlElementPrelim::empty(type_of(&node)));
        self.ops += 1;
        self.set_attrs(txn, &root, &node);
        let mut stack: Vec<(XmlElementRef, Value)> = vec![(root, node)];
        while let Some((el, n)) = stack.pop() {
            for (i, group) in pm::child_groups(content_of(&n)).into_iter().enumerate() {
                let i = i as u32;
                if type_of(group[0]) == "text" {
                    let text: XmlTextRef = el.insert(txn, i, XmlTextPrelim::new(""));
                    self.write_text(txn, &text, 0, &group, &Attrs::new());
                } else {
                    let child = el.insert(txn, i, XmlElementPrelim::empty(type_of(group[0])));
                    self.set_attrs(txn, &child, group[0]);
                    stack.push((child, group[0].clone()));
                }
            }
        }
    }

    fn set_attrs(&mut self, txn: &mut TransactionMut, el: &XmlElementRef, node: &Value) {
        if let Some(attrs) = node.get("attrs").and_then(Value::as_object) {
            for (k, v) in attrs {
                if !v.is_null() {
                    el.insert_attribute(txn, k.as_str(), json_to_any(v));
                }
            }
        }
    }

    /// Inserts text nodes at `at` of `text` (plain, inheriting `inherited`), then formats each to its marks.
    fn write_text(
        &mut self,
        txn: &mut TransactionMut,
        text: &XmlTextRef,
        at: u32,
        nodes: &[&Value],
        inherited: &Attrs,
    ) {
        let mut s = String::new();
        let mut runs: Vec<(u32, u32, Attrs)> = Vec::new();
        let mut pos = at;
        for n in nodes {
            let t = n.get("text").and_then(Value::as_str).unwrap_or("");
            let len = t.encode_utf16().count() as u32;
            s.push_str(t);
            runs.push((pos, len, pm::mark_attrs(pm::marks_of(n))));
            pos += len;
        }
        if s.is_empty() {
            return;
        }
        text.insert(txn, at, &s);
        self.ops += 1;
        for (start, len, want) in runs {
            self.format_to(txn, text, start, len, inherited, &want);
        }
    }

    /// Formats `[start, start+len)` from attrs `have` to `want`, one key at a time in name order.
    fn format_to(
        &mut self,
        txn: &mut TransactionMut,
        text: &XmlTextRef,
        start: u32,
        len: u32,
        have: &Attrs,
        want: &Attrs,
    ) {
        if len == 0 {
            return;
        }
        let mut keys: Vec<Arc<str>> = have.keys().chain(want.keys()).cloned().collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let w = want.get(&k).cloned().unwrap_or(Any::Null);
            let h = have.get(&k).cloned().unwrap_or(Any::Null);
            if w == h {
                continue;
            }
            let mut a = Attrs::new();
            a.insert(k.clone(), w);
            text.format(txn, start, len, a);
            self.ops += 1;
        }
    }

    // ----------------------------------------------------------------------------------------------- inline

    fn inline(&mut self, txn: &mut TransactionMut, el: &XmlElementRef, target: &[Value]) {
        let base = layout(txn, el);
        let tu = target_units(target);
        let nb = base.units.len();
        let nt = tu.len();
        let eq = |i: usize, j: usize| match (&base.units[i], &tu[j]) {
            (BUnit::Char(a, _), TUnit::Char(b, _)) => a == b,
            (BUnit::Elem(a), TUnit::Elem(b)) => a == *b,
            _ => false,
        };
        let mut p = 0;
        while p < nb && p < nt && eq(p, p) {
            p += 1;
        }
        let mut s = 0;
        while s < nb - p && s < nt - p && eq(nb - 1 - s, nt - 1 - s) {
            s += 1;
        }
        // Never split a surrogate pair.
        if p > 0 && matches!(base.units[p - 1], BUnit::Char(c, _) if (0xD800..0xDC00).contains(&c))
        {
            p -= 1;
        }
        if s > 0 && matches!(base.units[nb - s], BUnit::Char(c, _) if (0xDC00..0xE000).contains(&c))
        {
            s -= 1;
        }
        let (del_start, del_end) = (p, nb - s);
        let (ins_start, ins_end) = (p, nt - s);
        if del_start < del_end {
            self.delete_units(txn, el, &base, del_start, del_end);
        }
        if ins_start < ins_end {
            self.insert_units(txn, el, ins_start, &tu[ins_start..ins_end]);
        }
        // Marks: format every range whose marks differ from the target's.
        let now = layout(txn, el);
        let mut j = 0;
        while j < now.units.len().min(tu.len()) {
            let (BUnit::Char(_, chunk), TUnit::Char(_, want)) = (&now.units[j], &tu[j]) else {
                j += 1;
                continue;
            };
            let have = &now.chunks[*chunk];
            if &have.marks == want {
                j += 1;
                continue;
            }
            // A run within the same chunk (one XmlText) with the same target marks.
            let mut k = j + 1;
            while k < now.units.len().min(tu.len())
                && matches!((&now.units[k], &tu[k]), (BUnit::Char(_, c2), TUnit::Char(_, w2)) if c2 == chunk && w2 == want)
            {
                k += 1;
            }
            let seg = &now.segs[have.seg];
            if let Seg::Text { text, start, .. } = seg {
                let want_attrs = pm::mark_attrs(want);
                self.format_to(
                    txn,
                    text,
                    (j - start) as u32,
                    (k - j) as u32,
                    &have.raw,
                    &want_attrs,
                );
            }
            j = k;
        }
    }

    fn delete_units(
        &mut self,
        txn: &mut TransactionMut,
        el: &XmlElementRef,
        base: &Layout,
        from: usize,
        to: usize,
    ) {
        // Last segment first, so child indexes stay valid.
        for seg in base.segs.iter().rev() {
            match seg {
                Seg::Text {
                    text, start, len, ..
                } => {
                    let (a, b) = ((*start).max(from), (start + len).min(to));
                    if a < b {
                        text.remove_range(txn, (a - start) as u32, (b - a) as u32);
                        self.ops += 1;
                    }
                }
                Seg::Elem { child, unit } => {
                    if (from..to).contains(unit) {
                        el.remove_range(txn, *child, 1);
                        self.ops += 1;
                    }
                }
            }
        }
    }

    fn insert_units(
        &mut self,
        txn: &mut TransactionMut,
        el: &XmlElementRef,
        at: usize,
        units: &[TUnit],
    ) {
        let now = layout(txn, el);
        // Pieces: text runs (as text nodes) and elements.
        let mut pieces: Vec<Piece> = Vec::new();
        for u in units {
            match u {
                TUnit::Char(c, marks) => match pieces.last_mut() {
                    Some(Piece::Text(buf, runs)) => {
                        buf.push(*c);
                        match runs.last_mut() {
                            Some((m, n)) if m == marks => *n += 1,
                            _ => runs.push((marks.clone(), 1)),
                        }
                    }
                    _ => pieces.push(Piece::Text(vec![*c], vec![(marks.clone(), 1)])),
                },
                TUnit::Elem(v) => pieces.push(Piece::Elem((*v).clone())),
            }
        }
        // Where `at` falls: inside or at the end of a text segment (the first that holds it), or before an element.
        let has_elems = pieces.iter().any(|p| matches!(p, Piece::Elem(_)));
        let mut current: Option<(XmlTextRef, u32, Attrs)> = None; // (text, offset, inherited attrs)
        let mut child: u32 = now.child_count;
        let mut tail: Vec<Value> = Vec::new();
        for seg in &now.segs {
            match seg {
                Seg::Text {
                    text,
                    start,
                    len,
                    child: ci,
                } => {
                    if at >= *start && at <= start + len {
                        let off = (at - start) as u32;
                        let inherited = if off == 0 {
                            Attrs::new()
                        } else {
                            now.raw_at(at - 1).cloned().unwrap_or_default()
                        };
                        current = Some((text.clone(), off, inherited));
                        child = ci + 1;
                        if at < start + len && has_elems {
                            // Split: the tail moves after the inserted elements.
                            tail = now.text_nodes(at, start + len);
                            text.remove_range(txn, off, (start + len - at) as u32);
                            self.ops += 1;
                        }
                        break;
                    }
                    if *start > at {
                        child = *ci;
                        break;
                    }
                }
                Seg::Elem { child: ci, unit } => {
                    if *unit >= at {
                        child = *ci;
                        break;
                    }
                }
            }
        }
        for piece in pieces {
            match piece {
                Piece::Text(buf, runs) => {
                    let s = String::from_utf16_lossy(&buf);
                    let mut nodes = Vec::new();
                    let mut pos = 0usize;
                    for (marks, n) in runs {
                        let t = String::from_utf16_lossy(&buf[pos..pos + n]);
                        pos += n;
                        let mut node = json!({"type": "text", "text": t});
                        if !marks.is_empty() {
                            node["marks"] = Value::Array(marks.clone());
                        }
                        nodes.push(node);
                    }
                    let refs: Vec<&Value> = nodes.iter().collect();
                    match current.take() {
                        Some((text, off, inherited)) => {
                            self.write_text(txn, &text, off, &refs, &inherited);
                            let len = s.encode_utf16().count() as u32;
                            current = Some((
                                text,
                                off + len,
                                pm::mark_attrs(pm::marks_of(nodes.last().expect("one"))),
                            ));
                        }
                        None => {
                            let text: XmlTextRef = el.insert(txn, child, XmlTextPrelim::new(""));
                            self.ops += 1;
                            child += 1;
                            self.write_text(txn, &text, 0, &refs, &Attrs::new());
                            let len = s.encode_utf16().count() as u32;
                            current = Some((
                                text,
                                len,
                                pm::mark_attrs(pm::marks_of(nodes.last().expect("one"))),
                            ));
                        }
                    }
                }
                Piece::Elem(v) => {
                    self.insert_new(txn, el, child, &v);
                    child += 1;
                    current = None;
                }
            }
        }
        if !tail.is_empty() {
            let refs: Vec<&Value> = tail.iter().collect();
            match current {
                Some((text, off, inherited)) => self.write_text(txn, &text, off, &refs, &inherited),
                None => {
                    let text: XmlTextRef = el.insert(txn, child, XmlTextPrelim::new(""));
                    self.ops += 1;
                    self.write_text(txn, &text, 0, &refs, &Attrs::new());
                }
            }
        }
    }
}

enum Piece {
    Text(Vec<u16>, Vec<(Vec<Value>, usize)>),
    Elem(Value),
}

/// A base unit: a UTF-16 code unit of a chunk, or an inline element (normalized).
enum BUnit {
    Char(u16, usize),
    Elem(Value),
}

/// A target unit: a UTF-16 code unit with its normalized marks, or an inline node.
enum TUnit<'a> {
    Char(u16, Vec<Value>),
    Elem(&'a Value),
}

enum Seg {
    Text {
        text: XmlTextRef,
        start: usize,
        len: usize,
        child: u32,
    },
    Elem {
        child: u32,
        unit: usize,
    },
}

struct Chunk {
    /// The format attributes as stored.
    raw: Attrs,
    /// The normalized marks.
    marks: Vec<Value>,
    seg: usize,
}

struct Layout {
    units: Vec<BUnit>,
    chunks: Vec<Chunk>,
    segs: Vec<Seg>,
    child_count: u32,
}

impl Layout {
    fn raw_at(&self, unit: usize) -> Option<&Attrs> {
        match self.units.get(unit)? {
            BUnit::Char(_, c) => Some(&self.chunks[*c].raw),
            BUnit::Elem(_) => None,
        }
    }

    /// The text nodes (with marks) of units `[from, to)`, all inside one text segment.
    fn text_nodes(&self, from: usize, to: usize) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        let mut j = from;
        while j < to {
            let BUnit::Char(_, c) = &self.units[j] else {
                j += 1;
                continue;
            };
            let mut k = j;
            let mut buf: Vec<u16> = Vec::new();
            while k < to && matches!(&self.units[k], BUnit::Char(_, c2) if c2 == c) {
                if let BUnit::Char(u, _) = &self.units[k] {
                    buf.push(*u);
                }
                k += 1;
            }
            let mut node = json!({"type": "text", "text": String::from_utf16_lossy(&buf)});
            let marks = &self.chunks[*c].marks;
            if !marks.is_empty() {
                node["marks"] = Value::Array(marks.clone());
            }
            out.push(node);
            j = k;
        }
        out
    }
}

fn chunk_marks(raw: &Attrs) -> Vec<Value> {
    let marks: Vec<Value> = raw
        .iter()
        .map(|(k, v)| json!({"type": mark_name(k), "attrs": any_to_json(v)}))
        .collect();
    schema::normal_marks(&marks)
}

fn layout<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Layout {
    let mut l = Layout {
        units: Vec::new(),
        chunks: Vec::new(),
        segs: Vec::new(),
        child_count: 0,
    };
    for (ci, child) in el.children(txn).enumerate() {
        l.child_count = ci as u32 + 1;
        match child {
            XmlOut::Text(t) => {
                let start = l.units.len();
                let seg = l.segs.len();
                for d in t.diff(txn, YChange::identity) {
                    let Out::Any(Any::String(s)) = &d.insert else {
                        continue;
                    };
                    let raw: Attrs = d.attributes.map(|a| *a).unwrap_or_default();
                    let marks = chunk_marks(&raw);
                    let idx = l.chunks.len();
                    l.chunks.push(Chunk { raw, marks, seg });
                    for u in s.encode_utf16() {
                        l.units.push(BUnit::Char(u, idx));
                    }
                }
                let len = l.units.len() - start;
                l.segs.push(Seg::Text {
                    text: t,
                    start,
                    len,
                    child: ci as u32,
                });
            }
            XmlOut::Element(e) => {
                let raw = element_raw(txn, &e);
                let v = schema::normalize(&raw);
                l.segs.push(Seg::Elem {
                    child: ci as u32,
                    unit: l.units.len(),
                });
                l.units.push(BUnit::Elem(v));
            }
            XmlOut::Fragment(_) => {}
        }
    }
    l
}

/// The raw JSON of one inline element (no children expected).
fn element_raw<T: ReadTxn>(txn: &T, e: &XmlElementRef) -> Value {
    let mut node = Map::new();
    node.insert("type".into(), Value::from(e.tag().as_ref()));
    let attrs = element_attrs(txn, e);
    if !attrs.is_empty() {
        node.insert("attrs".into(), Value::Object(attrs));
    }
    Value::Object(node)
}

fn target_units(content: &[Value]) -> Vec<TUnit<'_>> {
    let mut out = Vec::new();
    for n in content {
        if type_of(n) == "text" {
            let marks = pm::marks_of(n).to_vec();
            for u in n
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .encode_utf16()
            {
                out.push(TUnit::Char(u, marks.clone()));
            }
        } else {
            out.push(TUnit::Elem(n));
        }
    }
    out
}

/// Loads Yjs updates into a new conversion document with `client_id`; an error names why they do not form a
/// complete state.
pub fn load(updates: &[&[u8]], client_id: Option<u32>) -> Result<Doc, String> {
    let doc = pm::new_doc(client_id);
    {
        let mut txn = doc.transact_mut();
        for bytes in updates {
            let update = Update::decode_v1(bytes)
                .map_err(|e| format!("the Yjs update does not decode: {e}"))?;
            txn.apply_update(update)
                .map_err(|e| format!("the Yjs update does not apply: {e}"))?;
        }
    }
    if doc.transact().has_missing_updates() {
        return Err("the Yjs update depends on structs it does not contain".into());
    }
    Ok(doc)
}
