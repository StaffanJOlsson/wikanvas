//! Document conversion between Y, ProseMirror JSON and Markdown (ARCHITECTURE §6.4, D49, D50) (owner: W4)
//!
//! - [`pm`]: Y ↔ raw and normalized ProseMirror JSON, after `@tiptap/y-tiptap`.
//! - [`md_out`]: Quire Markdown 1 output (the printer), and [`md_out::md_normalize`], the lossy rules of the §6.4
//!   table applied in JSON space.
//! - [`md_in`]: Quire Markdown 1 input (the nesting pre-scan, comrak, conversion to normalized JSON).
//! - [`ydiff`]: the diff writer that turns a target document into one Yjs update against a base.
//!
//! This module holds what they share: the scheme allow-list, item and attachment URL recognition, the lookup the
//! store provides, and conversion warnings.

pub mod md_in;
pub mod md_out;
pub mod pm;
pub mod ydiff;

/// The Yjs implementation the converters use, for crates and tests that speak Yjs without their own dependency.
pub use yrs;

#[cfg(test)]
mod tests;

use crate::api::{ConversionWarning, ConversionWarningCode, ItemKind};

/// Item links: `quire://item/<uuid>` (§6.4).
pub const QUIRE_ITEM_PREFIX: &str = "quire://item/";
/// The attachment URL path prefix (§7.5).
pub const ATTACHMENT_PREFIX: &str = "/api/v1/attachments/";

/// The scheme of `cleaned` (lowercase); `None` without one.
fn scheme_of(cleaned: &str) -> Option<String> {
    let colon = cleaned.find(':')?;
    let scheme = &cleaned[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| scheme.to_ascii_lowercase())
}

/// Every check runs on `url` without the control characters and spaces (U+0000–U+0020, U+007F–U+009F), as
/// `web/src/editor/links.ts` does: a browser drops tab and newline anywhere in a URL, so `/\t/evil.example` is the
/// protocol-relative `//evil.example`, and `java\tscript:` is `javascript:`.
fn allowed(url: &str, schemes: &[&str]) -> bool {
    let cleaned: String = url
        .chars()
        .filter(|c| !(c.is_control() || *c == ' '))
        .collect();
    if cleaned.is_empty() {
        return false;
    }
    if cleaned.starts_with('#') {
        return true;
    }
    if cleaned.starts_with('/') {
        return !cleaned.starts_with("//") && !cleaned.starts_with("/\\");
    }
    scheme_of(&cleaned).is_some_and(|s| schemes.contains(&s.as_str()))
}

/// The §6.4 allow-list for `link.href`: http, https, mailto, quire, same-origin paths and fragments.
pub fn link_allowed(href: &str) -> bool {
    allowed(href, &["http", "https", "mailto", "quire"])
}

/// The §6.4 allow-list for an external `image.src`: http, https, quire and same-origin paths (never `mailto:`).
pub fn image_allowed(src: &str) -> bool {
    allowed(src, &["http", "https", "quire"])
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// The item id of `quire://item/<uuid>` (lowercase), or `None`.
pub fn quire_item_id(url: &str) -> Option<String> {
    let id = url.strip_prefix(QUIRE_ITEM_PREFIX)?;
    is_uuid(id).then(|| id.to_ascii_lowercase())
}

/// `quire://item/<id>`.
pub fn quire_item_url(id: &str) -> String {
    format!("{QUIRE_ITEM_PREFIX}{id}")
}

/// How an input URL names an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemRef {
    /// `quire://item/<uuid>`.
    Id(String),
    /// `/doc/<slug>-<url_id>` or `/board/<slug>-<url_id>`, relative or under the public URL.
    UrlId(String),
}

/// The path of `url` when it is same-origin: a path, or an absolute URL under `public_origin` (`scheme://host[:port]`).
fn same_origin_path<'a>(url: &'a str, public_origin: Option<&str>) -> Option<&'a str> {
    if url.starts_with('/') && !url.starts_with("//") {
        return Some(url);
    }
    let origin = public_origin?.trim_end_matches('/');
    let rest = url
        .get(..origin.len())
        .filter(|p| p.eq_ignore_ascii_case(origin))?;
    let path = &url[rest.len()..];
    path.starts_with('/').then_some(path)
}

/// Whether `url` names an item of this instance (§6.4 "Item links").
pub fn item_ref(url: &str, public_origin: Option<&str>) -> Option<ItemRef> {
    if let Some(id) = quire_item_id(url) {
        return Some(ItemRef::Id(id));
    }
    let path = same_origin_path(url, public_origin)?;
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');
    let rest = path
        .strip_prefix("/doc/")
        .or_else(|| path.strip_prefix("/board/"))?;
    if rest.contains('/') || rest.len() < 10 {
        return None;
    }
    let (head, url_id) = rest.split_at(rest.len() - 10);
    if !url_id.bytes().all(|b| b.is_ascii_alphanumeric())
        || !(head.is_empty() || head.ends_with('-'))
    {
        return None;
    }
    Some(ItemRef::UrlId(url_id.to_owned()))
}

/// The attachment id of a same-instance attachment URL (`/api/v1/attachments/<id>`), or `None`.
pub fn attachment_ref(url: &str, public_origin: Option<&str>) -> Option<String> {
    let path = same_origin_path(url, public_origin)?;
    let id = path.strip_prefix(ATTACHMENT_PREFIX)?;
    let id = id.split(['?', '#']).next().unwrap_or(id);
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .then(|| id.to_owned())
}

/// `/api/v1/attachments/<id>`.
pub fn attachment_url(id: &str) -> String {
    format!("{ATTACHMENT_PREFIX}{id}")
}

/// An item as the converters see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemInfo {
    pub id: String,
    pub kind: ItemKind,
    /// The live title (mentions and board embeds print it).
    pub title: String,
}

/// What the converters need from the database (§6.4: "the parser takes a lookup closure from the store").
pub trait Lookup {
    /// The item an input URL names, if it exists (and is not purged).
    fn item(&self, r: &ItemRef) -> Option<ItemInfo>;
    /// Whether an attachment exists.
    fn attachment(&self, id: &str) -> bool;
    /// The public URL (`scheme://host[:port]`), for absolute same-instance URLs.
    fn public_origin(&self) -> Option<&str> {
        None
    }
}

/// A lookup that knows nothing: every item and attachment is unknown.
pub struct NoLookup;

impl Lookup for NoLookup {
    fn item(&self, _: &ItemRef) -> Option<ItemInfo> {
        None
    }

    fn attachment(&self, _: &str) -> bool {
        false
    }
}

/// Collects conversion warnings: one entry per code, with the first path and the count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Warnings(Vec<ConversionWarning>);

impl Warnings {
    pub fn add(&mut self, code: ConversionWarningCode, path: Option<String>) {
        if let Some(w) = self.0.iter_mut().find(|w| w.code == code) {
            w.count += 1;
            return;
        }
        self.0.push(ConversionWarning {
            code,
            path,
            count: 1,
        });
    }

    pub fn extend(&mut self, other: Warnings) {
        for w in other.0 {
            match self.0.iter_mut().find(|x| x.code == w.code) {
                Some(x) => x.count += w.count,
                None => self.0.push(w),
            }
        }
    }

    pub fn has(&self, code: ConversionWarningCode) -> bool {
        self.0.iter().any(|w| w.code == code)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The warnings, in the order their codes first occurred.
    pub fn into_vec(self) -> Vec<ConversionWarning> {
        self.0
    }

    pub fn as_slice(&self) -> &[ConversionWarning] {
        &self.0
    }

    /// The codes, comma-separated (the `Quire-Conversion-Warnings` header).
    pub fn header(&self) -> String {
        self.0
            .iter()
            .map(|w| w.code.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A parsed target of a document PUT.
#[derive(Clone, Debug)]
pub enum Target {
    /// Validated, normalized ProseMirror JSON (`schema::validate`).
    Json(serde_json::Value),
    /// Parsed Markdown (`md_in::parse`).
    Markdown(serde_json::Value),
}

impl Target {
    pub fn doc(&self) -> &serde_json::Value {
        match self {
            Target::Json(d) | Target::Markdown(d) => d,
        }
    }
}

/// Why a base cannot take a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseError {
    /// The stored updates do not form a document.
    Corrupt(String),
    /// The document holds nodes or marks schema v1 does not know (409 `schema_too_new`).
    SchemaTooNew,
}

/// The result of [`write`].
#[derive(Clone, Debug)]
pub struct Written {
    pub update: Vec<u8>,
    pub changed: bool,
    pub warnings: Warnings,
    pub client_id: u32,
}

/// The Markdown comparison tree of a base (§6.4 step 3): the lowered base with its block map, checked against what
/// the parser reads back from the printed base. Where they differ (a lossy step the lowering does not model), the
/// re-parsed top-level blocks are used, each mapped to the lowered block it equals, or standing for the base blocks
/// of its gap.
pub fn comparison_tree(base: &serde_json::Value, lookup: &dyn Lookup) -> serde_json::Value {
    use serde_json::Value;
    let tree = ydiff::base_tree(base);
    let lowered = md_out::lower(&tree, lookup, &mut Warnings::default());
    let printed = md_out::render(&lowered, lookup);
    let Ok((reparsed, _)) = md_in::parse(&printed, lookup) else {
        return lowered;
    };
    if ydiff::same(&lowered, &reparsed) {
        return lowered;
    }
    let c1 = schema::content_of(&lowered);
    let mut c2: Vec<Value> = schema::content_of(&reparsed).to_vec();
    let h1: Vec<Value> = c1.iter().map(strip_clone).collect();
    let h2: Vec<Value> = c2.iter().map(strip_clone).collect();
    let anchors = ydiff::myers(c1.len(), c2.len(), &|i, j| h1[i] == h2[j], 2000);
    let stands_for = |n: &Value| -> Vec<Value> {
        match n.get(md_out::ORIGIN) {
            Some(o) => vec![o.clone()],
            None => n
                .get(md_out::CONSUMED)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    };
    let (mut i0, mut j0) = (0, 0);
    let mut bounds = anchors.clone();
    bounds.push((c1.len(), c2.len()));
    for (ai, aj) in bounds {
        let gap: Vec<Value> = c1[i0..ai].iter().flat_map(stands_for).collect();
        for n in &mut c2[j0..aj] {
            if let Some(o) = n.as_object_mut() {
                o.insert(md_out::CONSUMED.into(), Value::Array(gap.clone()));
            }
        }
        if ai < c1.len() && aj < c2.len() {
            c2[aj] = c1[ai].clone();
        }
        i0 = ai + 1;
        j0 = aj + 1;
    }
    serde_json::json!({"type": "doc", "content": c2})
}

fn strip_clone(n: &serde_json::Value) -> serde_json::Value {
    let mut n = n.clone();
    md_out::strip_map(&mut n);
    let mut stack = vec![&mut n];
    while let Some(x) = stack.pop() {
        if let Some(o) = x.as_object_mut() {
            if let Some(serde_json::Value::Object(a)) = o.get_mut("attrs") {
                a.remove(schema::BLOCK_ID);
            }
            if let Some(serde_json::Value::Array(c)) = o.get_mut("content") {
                stack.extend(c.iter_mut());
            }
        }
    }
    n
}

/// Applies a target to the base made of `base` (a full state and/or stored updates) as one update (§6.4 steps 3–5):
/// the client id derives from `h` (re-hashed while it is in the base's state vector), new block ids from
/// `sha256(h ‖ counter)`.
pub fn write(
    base: &[&[u8]],
    target: &Target,
    lookup: &dyn Lookup,
    h: &[u8; 32],
) -> Result<Written, BaseError> {
    let sv = ydiff::state_vector_of(base).map_err(BaseError::Corrupt)?;
    let client_id = ydiff::client_id(h, &sv);
    let doc = ydiff::load(base, Some(client_id)).map_err(BaseError::Corrupt)?;
    let raw = {
        let frag = pm::fragment(&doc);
        let txn = yrs::Transact::transact(&doc);
        pm::raw(&txn, &frag).map_err(|e| BaseError::Corrupt(e.to_string()))?
    };
    if schema::has_unknown(&raw) {
        return Err(BaseError::SchemaTooNew);
    }
    let (b, markdown) = match target {
        Target::Json(_) => (ydiff::annotate(&raw), false),
        Target::Markdown(_) => (comparison_tree(&raw, lookup), true),
    };
    let out = ydiff::write(&doc, &b, target.doc(), markdown, *h);
    Ok(Written {
        update: out.update,
        changed: out.changed,
        warnings: out.warnings,
        client_id,
    })
}

/// `?title=from_h1`: removes a leading level-1 heading from a parsed target and returns its text.
pub fn take_h1(doc: &mut serde_json::Value) -> Option<String> {
    use serde_json::Value;
    let content = doc.get_mut("content")?.as_array_mut()?;
    let first = content.first()?;
    if schema::type_of(first) != "heading"
        || first.get("attrs").and_then(|a| a.get("level")) != Some(&Value::from(1))
    {
        return None;
    }
    let heading = content.remove(0);
    if content.is_empty() {
        content.push(serde_json::json!({"type": "paragraph", "attrs": {"blockId": null}}));
    }
    let text: String = schema::content_of(&heading)
        .iter()
        .filter_map(|n| n.get("text").and_then(Value::as_str))
        .collect();
    Some(text.trim().to_owned())
}

use crate::schema;
