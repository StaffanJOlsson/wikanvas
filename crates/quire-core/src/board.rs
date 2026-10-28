//! The Inkboard envelope codec: split, splice, inspect and preview on the raw JSON (ARCHITECTURE §6.2) (owner: W6)
//!
//! A board travels as the native envelope `{"format":"inkboard","version":2,"document":{…}}` (D8). The server never
//! re-serializes it through the typed model (D9): [`split`] and [`splice`] edit the JSON text itself, at the byte
//! spans that `serde_json::value::RawValue` borrows from the input, so fields this build does not know (a newer
//! engine's) survive a PUT and a GET unchanged. Only [`inspect`] and [`preview`] parse the typed model
//! (`inkboard_core::serial::from_json`), for validation, the search text, the meta and the SVG previews.
//!
//! - [`split`] cuts every `document.assets[*].data` out of the envelope (decoded from base64) and leaves `""` in its
//!   place; every other byte, the asset entries and their `mime` included, stays as it was (a leading UTF-8 byte order
//!   mark is dropped, as `from_json` does).
//! - [`splice`] puts the bytes back as base64 and replaces `document.name` with the item's title (D31).
//! - [`inspect`] validates the full envelope and derives the text, [`Meta`] (§5 `board_state.meta`) and the targets
//!   of the board's cards ([`embeds`], §18.8), which the store records as `item_links` of kind `embed`.
//! - [`Previewer`] (and [`preview`] / [`previews`]) render page SVGs with Inkboard's SVG export and its approximate
//!   measurer (D10), within [`PAGE_SVG_MAX`] per page and [`BOARD_SVG_MAX`] per board: images that do not fit are
//!   drawn as placeholders, a page that does not fit becomes a [`card_svg`].

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use inkboard_core::connector::End;
use inkboard_core::model::{Asset, AssetId, Document, ImageObj, Kind, Page, Table};
use inkboard_core::serial::{self, LoadError};
use inkboard_core::svg::SvgOptions;
use inkboard_core::text::{ApproxMeasurer, TextLayout, layout_rich};
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::api::ErrorCode;

/// Bytes of the board text kept as `meta.text_excerpt` (§9.5: "the first 4 KB of the server-derived search text"),
/// cut back to a character boundary.
pub const EXCERPT_BYTES: usize = 4096;

/// Why an envelope was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardError {
    /// Not a board envelope this server can read: 422 `invalid_board`.
    Invalid(String),
    /// `version` is newer than `inkboard_core::serial::VERSION`: 422 `board_too_new`.
    TooNew(u32),
}

impl BoardError {
    /// The API error code (both are 422).
    pub fn code(&self) -> ErrorCode {
        match self {
            BoardError::Invalid(_) => ErrorCode::InvalidBoard,
            BoardError::TooNew(_) => ErrorCode::BoardTooNew,
        }
    }

    fn invalid(message: impl fmt::Display) -> BoardError {
        BoardError::Invalid(message.to_string())
    }
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoardError::Invalid(m) => write!(f, "not a valid Inkboard board: {m}"),
            BoardError::TooNew(v) => write!(
                f,
                "the board has format version {v}, newer than this server's {}; update Quire",
                serial::VERSION
            ),
        }
    }
}

impl std::error::Error for BoardError {}

impl From<LoadError> for BoardError {
    fn from(e: LoadError) -> Self {
        match e {
            LoadError::TooNew(v) => BoardError::TooNew(v),
            other => BoardError::invalid(other),
        }
    }
}

/// One asset cut out of an envelope by [`split`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitAsset {
    /// The key in `document.assets`.
    pub id: String,
    /// The asset's `mime`, as the envelope declares it.
    pub mime: String,
    /// The decoded `data`; empty when the envelope already had `""`.
    pub bytes: Vec<u8>,
}

/// The result of [`split`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    /// The envelope with every `document.assets[*].data` replaced by `""`.
    pub stripped: String,
    /// The assets in the order of the envelope (a repeated key appears once, with its last value, as in
    /// `from_json`).
    pub assets: Vec<SplitAsset>,
    /// The envelope's `version`.
    pub format_version: u32,
}

/// One page of [`Meta`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaPage {
    pub id: String,
    pub name: String,
}

/// What [`inspect`] derives besides the text: the §5 `board_state.meta` JSON
/// `{"pages":[{"id","name"}],"objects":n,"background":"#rrggbb","bytes":n,"text_excerpt":"…"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub pages: Vec<MetaPage>,
    /// Objects on all pages.
    pub objects: u64,
    /// `#rrggbb`.
    pub background: String,
    /// Size of the full envelope in bytes.
    pub bytes: u64,
    /// The first [`EXCERPT_BYTES`] of the text.
    pub text_excerpt: String,
}

/// The result of [`inspect`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspect {
    /// The search text (`item_text.body`): per page, its name, then the text of its objects in drawing order (box
    /// labels, connector labels, table rows with cells separated by `\t`, image LaTeX), one entry per line, empty
    /// entries left out. A card's label is its title snapshot (§18.2), so a board is found by the titles of the
    /// documents it shows, never by their bodies.
    pub text: String,
    pub meta: Meta,
    /// The targets of the board's cards that are canonical UUIDs ([`embeds`]): what `item_links` records.
    pub embeds: Vec<String>,
}

/// The most targets of one board that [`embeds`] returns (§18.8); the targets after them are not linked.
pub const EMBED_LINKS_MAX: usize = 1000;

/// The envelope with the document kept as raw JSON. Unknown top-level fields are allowed (and kept by the span edits).
#[derive(Deserialize)]
struct Envelope<'a> {
    format: String,
    version: u32,
    #[serde(borrow)]
    document: &'a RawValue,
}

/// The entries of a JSON object, in order, with their values as raw JSON borrowed from the input.
struct Entries<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Entries<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Entries<'de>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entries<'de>, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(4));
                while let Some((k, v)) = map.next_entry::<String, &'de RawValue>()? {
                    entries.push((k, v));
                }
                Ok(Entries(entries))
            }
        }
        d.deserialize_map(V)
    }
}

/// One edit of the envelope text: replace `input[start..end]` with `with`.
struct Edit {
    start: usize,
    end: usize,
    with: String,
}

/// The byte range of `part` (a slice of `base`) in `base`.
fn span(base: &str, part: &str) -> (usize, usize) {
    let start = part.as_ptr() as usize - base.as_ptr() as usize;
    debug_assert!(start + part.len() <= base.len(), "part is a slice of base");
    (start, start + part.len())
}

fn apply(input: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| e.start);
    let removed: usize = edits.iter().map(|e| e.end - e.start).sum();
    let added: usize = edits.iter().map(|e| e.with.len()).sum();
    let mut out = String::with_capacity(input.len() - removed + added);
    let mut at = 0;
    for e in &edits {
        out.push_str(&input[at..e.start]);
        out.push_str(&e.with);
        at = e.end;
    }
    out.push_str(&input[at..]);
    out
}

/// The envelope and its document's entries, after the format and version checks.
fn parse_envelope(input: &str) -> Result<(Envelope<'_>, Entries<'_>), BoardError> {
    let env: Envelope<'_> = serde_json::from_str(input).map_err(BoardError::invalid)?;
    if !serial::is_native_format(&env.format) {
        return Err(BoardError::invalid(format!(
            "the format is \"{}\", not \"{}\"",
            env.format,
            serial::FORMAT
        )));
    }
    if env.version > serial::VERSION {
        return Err(BoardError::TooNew(env.version));
    }
    let doc: Entries<'_> = serde_json::from_str(env.document.get())
        .map_err(|e| BoardError::invalid(format!("document: {e}")))?;
    Ok((env, doc))
}

/// A raw asset entry: its `mime` and its raw `data` value.
struct RawAsset<'a> {
    id: String,
    mime: String,
    data: &'a RawValue,
}

/// Every `document.assets` entry, in order.
fn raw_assets<'a>(doc: &Entries<'a>) -> Result<Vec<RawAsset<'a>>, BoardError> {
    let mut out = Vec::new();
    for (key, value) in &doc.0 {
        if key != "assets" {
            continue;
        }
        let assets: Entries<'a> = serde_json::from_str(value.get())
            .map_err(|e| BoardError::invalid(format!("document.assets: {e}")))?;
        for (id, raw) in assets.0 {
            let fields: Entries<'a> = serde_json::from_str(raw.get())
                .map_err(|e| BoardError::invalid(format!("document.assets.{id}: {e}")))?;
            let mut mime = None;
            let mut data = None;
            for (name, v) in fields.0 {
                match name.as_str() {
                    "mime" => {
                        mime = Some(serde_json::from_str::<String>(v.get()).map_err(|e| {
                            BoardError::invalid(format!("document.assets.{id}.mime: {e}"))
                        })?);
                    }
                    "data" => data = Some(v),
                    _ => {}
                }
            }
            let mime =
                mime.ok_or_else(|| BoardError::invalid(format!("document.assets.{id}: no mime")))?;
            let data =
                data.ok_or_else(|| BoardError::invalid(format!("document.assets.{id}: no data")))?;
            out.push(RawAsset { id, mime, data });
        }
    }
    Ok(out)
}

/// Splits the assets out of a full envelope (§6.2): every `document.assets[*].data` is base64-decoded and replaced by
/// `""`; everything else is re-emitted byte for byte. Refuses another `format` (422 `invalid_board`) and a newer
/// `version` (422 `board_too_new`). It does not validate the document itself: the save pipeline runs [`inspect`] on
/// the full body first.
pub fn split(body: &str) -> Result<Split, BoardError> {
    let input = serial::strip_bom(body);
    let (env, doc) = parse_envelope(input)?;
    let mut edits = Vec::new();
    let mut assets: Vec<SplitAsset> = Vec::new();
    for a in raw_assets(&doc)? {
        let text: String = serde_json::from_str(a.data.get())
            .map_err(|e| BoardError::invalid(format!("document.assets.{}.data: {e}", a.id)))?;
        let bytes = STANDARD.decode(text.as_bytes()).map_err(|e| {
            BoardError::invalid(format!("document.assets.{}.data: not base64: {e}", a.id))
        })?;
        drop(text);
        let (start, end) = span(input, a.data.get());
        edits.push(Edit {
            start,
            end,
            with: "\"\"".to_owned(),
        });
        let asset = SplitAsset {
            id: a.id,
            mime: a.mime,
            bytes,
        };
        match assets.iter_mut().find(|x| x.id == asset.id) {
            Some(earlier) => *earlier = asset,
            None => assets.push(asset),
        }
    }
    Ok(Split {
        stripped: apply(input, edits),
        assets,
        format_version: env.version,
    })
}

/// The inverse of [`split`] (§6.2): puts each asset's bytes back as base64 (assets without stored bytes stay `""`) and
/// replaces `document.name` with `title` (D31). Refuses what [`split`] refuses.
pub fn splice(
    stripped: &str,
    assets: &HashMap<String, Vec<u8>>,
    title: &str,
) -> Result<String, BoardError> {
    let input = serial::strip_bom(stripped);
    let (env, doc) = parse_envelope(input)?;
    let mut edits = Vec::new();
    for a in raw_assets(&doc)? {
        if let Some(bytes) = assets.get(&a.id) {
            let (start, end) = span(input, a.data.get());
            let mut with = String::with_capacity(bytes.len().div_ceil(3) * 4 + 2);
            with.push('"');
            STANDARD.encode_string(bytes, &mut with);
            with.push('"');
            edits.push(Edit { start, end, with });
        }
    }
    let name = serde_json::to_string(title).map_err(BoardError::invalid)?;
    let names: Vec<&RawValue> = doc
        .0
        .iter()
        .filter(|(k, _)| k == "name")
        .map(|(_, v)| *v)
        .collect();
    if names.is_empty() {
        // Every valid document has a name; insert one right after the document's `{`.
        let (start, _) = span(input, env.document.get());
        let sep = if doc.0.is_empty() { "" } else { "," };
        edits.push(Edit {
            start: start + 1,
            end: start + 1,
            with: format!("\"name\":{name}{sep}"),
        });
    }
    for raw in names {
        let (start, end) = span(input, raw.get());
        edits.push(Edit {
            start,
            end,
            with: name.clone(),
        });
    }
    Ok(apply(input, edits))
}

/// Validates a full envelope with `serial::from_json` (422 `invalid_board` / `board_too_new`), and refuses tables
/// whose cells are not rows × columns and box kinds with the key `embed` twice ([`embeds`]), then derives the search
/// text, the meta (§6.2) and the targets of the cards (§18.8). `meta.bytes` is the length of `full`.
pub fn inspect(full: &str) -> Result<Inspect, BoardError> {
    let doc = serial::from_json(full)?;
    check_shapes(&doc)?;
    let text = text_of(&doc);
    let meta = Meta {
        pages: doc
            .pages
            .iter()
            .map(|p| MetaPage {
                id: p.id.as_str().to_owned(),
                name: p.name.clone(),
            })
            .collect(),
        objects: doc.pages.iter().map(|p| p.objects.len() as u64).sum(),
        background: doc.background.preset.hex(),
        bytes: full.len() as u64,
        text_excerpt: excerpt(&text).to_owned(),
    };
    // The scan's lists are never held beside the typed document (a 50 MB board's peak, D25).
    drop(doc);
    let embeds = embeds(full)?;
    Ok(Inspect { text, meta, embeds })
}

/// The targets of a board's cards (§18.2, §18.8): the `item` of every object whose `kind` has `type` `"box"`, `geom`
/// `"rect"` and an `embed` that is a JSON object with a string `item`, when that `item` is a canonical UUID
/// ([`is_canonical_uuid`]). Distinct, in the order of the pages and their objects, at most [`EMBED_LINKS_MAX`]. An
/// object whose id repeats an earlier one on its page is not read, as the engine's `serial::sanitize` drops it.
///
/// It is a second, borrowed pass over `full`, meant for a body that `serial::from_json` has accepted ([`inspect`]
/// runs it after that): it reads `document.pages[].objects[].{id, kind}`, of the kind only `type`, `geom` and `embed`,
/// with strings borrowed unless they hold an escape, `embed` as raw JSON read leniently (as the M1.1 engine reads it:
/// anything but an object with a string `item` is no card, and a repeated `item` keeps its last value), and skips
/// everything else; no `Value` tree is built (D9). No value of `embed` is refused (D60). The one refusal (422
/// `invalid_board`) is a box kind with the key `embed` twice: the M1.1 engine's model knows `embed` and refuses the
/// repeated field (serde's `duplicate_field`), while this server's unpatched model ignores both, so such a board
/// could be stored but never opened in the web client (§18.3 "Lenient model").
///
/// The pass reads every shape `from_json` accepts: its envelope, document, page and object structs have the engine's
/// fields in the engine's order, so serde reads each from a JSON object or, by position, from a JSON array, as the
/// engine does; a kind by position is never a card (`from_json` refuses one long enough to hold `embed`). A body the
/// pass cannot read (one that
/// `from_json` refuses) gives no targets, never an error.
pub fn embeds(full: &str) -> Result<Vec<String>, BoardError> {
    let Ok(env) = serde_json::from_str::<ScanEnvelope<'_>>(serial::strip_bom(full)) else {
        return Ok(Vec::new());
    };
    let mut targets: Vec<String> = Vec::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for (index, page) in env.document.pages.iter().enumerate() {
        let mut ids: HashSet<&str> = HashSet::with_capacity(page.objects.len());
        for o in &page.objects {
            if o.kind.is_box && o.kind.embed_keys > 1 {
                return Err(BoardError::invalid(format!(
                    "page {index}: the object {}: duplicate field `embed` (a card holds one embed)",
                    o.id
                )));
            }
            if !ids.insert(&o.id) {
                continue;
            }
            if let Some(item) = o.kind.card.as_deref()
                && targets.len() < EMBED_LINKS_MAX
                && is_canonical_uuid(item)
                && taken.insert(item)
            {
                targets.push(item.to_owned());
            }
        }
    }
    Ok(targets)
}

/// Whether `s` is a UUID in canonical form: 36 characters, lowercase hexadecimal digits with hyphens at 8, 13, 18 and
/// 23, as `items.id` holds them (§18.2: the only `item` values the server links and the host fetches).
pub fn is_canonical_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// What the embed scan reads of an envelope. This struct and the three below have exactly the fields of `serial`'s
/// envelope and of `model::{Document, Page, Object}`, in the same order and with the same defaults, the unread ones as
/// [`IgnoredAny`]: serde reads a struct from a JSON object (by name) or from a JSON array (by position), and the
/// engine accepts both at every level, so with fewer or reordered fields the scan would read an array differently or
/// not at all, and lose every link of a board the engine opens (or let a repeated `embed` key through).
#[derive(Deserialize)]
struct ScanEnvelope<'a> {
    #[serde(rename = "format")]
    _format: IgnoredAny,
    #[serde(rename = "version")]
    _version: IgnoredAny,
    #[serde(borrow)]
    document: ScanDocument<'a>,
}

#[derive(Deserialize)]
struct ScanDocument<'a> {
    #[serde(rename = "name")]
    _name: IgnoredAny,
    #[serde(rename = "background", default)]
    _background: IgnoredAny,
    #[serde(borrow)]
    pages: Vec<ScanPage<'a>>,
    #[serde(rename = "assets", default)]
    _assets: IgnoredAny,
}

#[derive(Deserialize)]
struct ScanPage<'a> {
    #[serde(rename = "id")]
    _id: IgnoredAny,
    #[serde(rename = "name")]
    _name: IgnoredAny,
    #[serde(rename = "layers")]
    _layers: IgnoredAny,
    #[serde(borrow)]
    objects: Vec<ScanObject<'a>>,
    #[serde(rename = "view", default)]
    _view: IgnoredAny,
}

#[derive(Deserialize)]
struct ScanObject<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(borrow)]
    kind: ScanKind<'a>,
    #[serde(rename = "style", default)]
    _style: IgnoredAny,
    #[serde(rename = "group", default)]
    _group: IgnoredAny,
    #[serde(rename = "locked", default)]
    _locked: IgnoredAny,
    #[serde(rename = "layer", default)]
    _layer: IgnoredAny,
}

/// What the embed scan reads of an object's `kind`. Read by hand rather than derived, so that a key repeated in a
/// kind that both engines accept (a `geom` in a pen, say) neither fails the scan nor counts as a card.
///
/// A kind may also be a JSON array: serde reads an internally tagged enum from `[tag, field…]`, the variant's fields
/// by position. Such a kind holds no `embed` key and is no card: the unpatched model refuses a `box` array with more
/// elements than `BoxObj`'s four fields (`geom`, `frame`, `label`, `lang`), so `from_json` never accepts one that
/// would hold a fifth, the M1.1 engine's `embed`.
struct ScanKind<'a> {
    /// `type` is `"box"`.
    is_box: bool,
    /// The `embed.item` of a card (§18.2), in whatever form it has.
    card: Option<Cow<'a, str>>,
    /// How many `embed` keys the kind holds.
    embed_keys: usize,
}

impl<'de: 'a, 'a> Deserialize<'de> for ScanKind<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = ScanKind<'a>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object's kind")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ScanKind<'a>, A::Error> {
                let (mut ty, mut geom, mut embed, mut embed_keys) = (None, None, None, 0);
                while let Some(Text(key)) = map.next_key::<Text<'a>>()? {
                    match &*key {
                        "type" => ty = text(map.next_value::<&'a RawValue>()?),
                        "geom" => geom = text(map.next_value::<&'a RawValue>()?),
                        "embed" => {
                            let value = map.next_value::<&'a RawValue>()?;
                            embed_keys += 1;
                            embed.get_or_insert(value);
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let is_box = ty.as_deref() == Some("box");
                let card = if is_box && geom.as_deref() == Some("rect") {
                    embed.and_then(embed_item)
                } else {
                    None
                };
                Ok(ScanKind {
                    is_box,
                    card,
                    embed_keys,
                })
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ScanKind<'a>, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ScanKind {
                    is_box: false,
                    card: None,
                    embed_keys: 0,
                })
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// The last `item` of an `embed` object, as raw JSON; `None` inside when it has none.
struct EmbedItem<'a>(Option<&'a RawValue>);

impl<'de: 'a, 'a> Deserialize<'de> for EmbedItem<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = EmbedItem<'a>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an embed object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<EmbedItem<'a>, A::Error> {
                let mut item = None;
                while let Some(Text(key)) = map.next_key::<Text<'a>>()? {
                    if key == "item" {
                        item = Some(map.next_value::<&'a RawValue>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(EmbedItem(item))
            }
        }
        d.deserialize_map(V(PhantomData))
    }
}

/// The `item` of an `embed` value when it is an object whose (last) `item` is a string.
fn embed_item(raw: &RawValue) -> Option<Cow<'_, str>> {
    let EmbedItem(item) = serde_json::from_str(raw.get()).ok()?;
    text(item?)
}

/// A raw JSON value as a string, `None` when it is not one.
fn text(raw: &RawValue) -> Option<Cow<'_, str>> {
    serde_json::from_str::<Text<'_>>(raw.get())
        .ok()
        .map(|t| t.0)
}

/// A JSON string, borrowed from the input unless it holds an escape (then it is unescaped into an owned copy).
struct Text<'a>(Cow<'a, str>);

impl<'de: 'a, 'a> Deserialize<'de> for Text<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Text<'a>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }

            fn visit_borrowed_str<E>(self, s: &'de str) -> Result<Text<'a>, E> {
                Ok(Text(Cow::Borrowed(s)))
            }

            fn visit_str<E>(self, s: &str) -> Result<Text<'a>, E> {
                Ok(Text(Cow::Owned(s.to_owned())))
            }

            fn visit_string<E>(self, s: String) -> Result<Text<'a>, E> {
                Ok(Text(Cow::Owned(s)))
            }
        }
        d.deserialize_str(V(PhantomData))
    }
}

/// The SVG of page `page` (0-based) of a full envelope, `None` when the page is empty or does not exist. Rendered by
/// a [`Previewer`], so within [`PAGE_SVG_MAX`].
pub fn preview(full: &str, page: usize) -> Result<Option<String>, BoardError> {
    let mut previewer = Previewer::new(full)?;
    Ok(previewer.render(page))
}

/// The SVG of every page of a full envelope, parsed once, rendered by one [`Previewer`]; `None` for an empty page.
/// When the board budget ([`BOARD_SVG_MAX`]) is spent the list stops: the pages after it get no preview.
pub fn previews(full: &str) -> Result<Vec<Option<String>>, BoardError> {
    let mut previewer = Previewer::new(full)?;
    let mut out = Vec::with_capacity(previewer.pages());
    for page in 0..previewer.pages() {
        if previewer.spent() {
            break;
        }
        out.push(previewer.render(page));
    }
    Ok(out)
}

/// The most one page preview may hold, by the estimate [`Previewer::render`] makes before it renders (D10, D25): an
/// image embeds its whole asset as base64 once per image object, so a small board can describe a huge SVG.
pub const PAGE_SVG_MAX: usize = 16 << 20;

/// The most all page previews of one board may hold together (the same estimate, then the real lengths).
pub const BOARD_SVG_MAX: usize = 48 << 20;

/// The text of the card that stands in for a page too large to preview.
pub const TOO_LARGE: &str = "Too large to preview";

/// What an image whose data does not fit the budget is drawn with: a grey box stretched over the image's frame.
const PLACEHOLDER: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" preserveAspectRatio="none"><rect width="10" height="10" fill="#8b93a1" fill-opacity="0.35"/></svg>"##;

/// Renders the page previews of one board, parsed once, page by page, within [`PAGE_SVG_MAX`] per page and
/// [`BOARD_SVG_MAX`] in all. Before each page it estimates the SVG from the model (numbers at their printed width,
/// text from its layout, images at the base64 size of their asset). When the estimate passes what is left, the
/// images with the largest share are drawn as grey placeholders, largest first; a page whose drawing alone passes it
/// becomes a small "too large to preview" card ([`card_svg`]).
pub struct Previewer {
    doc: Document,
    left: usize,
}

impl Previewer {
    /// Parses and validates a full envelope (as [`inspect`] does).
    pub fn new(full: &str) -> Result<Previewer, BoardError> {
        let doc = serial::from_json(full)?;
        check_shapes(&doc)?;
        Ok(Previewer {
            doc,
            left: BOARD_SVG_MAX,
        })
    }

    pub fn pages(&self) -> usize {
        self.doc.pages.len()
    }

    /// The board budget is spent: the pages not rendered yet get no preview.
    pub fn spent(&self) -> bool {
        self.left == 0
    }

    /// The board background (`#rrggbb`), for [`card_svg`].
    pub fn background(&self) -> String {
        self.doc.background.preset.hex()
    }

    /// The SVG of page `index`: `None` for an empty or missing page, the card when even its drawing without images
    /// does not fit. Its length is taken from the board budget.
    pub fn render(&mut self, index: usize) -> Option<String> {
        let page = self.doc.pages.get(index)?;
        if !page.objects.iter().any(|o| page.is_visible(o)) {
            return None;
        }
        let allowance = PAGE_SVG_MAX.min(self.left);
        let estimate = estimate(&self.doc, page);
        let svg = if estimate.drawing > allowance {
            card_svg(&self.doc.background.preset.hex(), TOO_LARGE)
        } else {
            let swap = estimate.over(allowance);
            if swap.is_empty() {
                inkboard_core::svg::export_approx(&self.doc, page)?
            } else {
                // The assets are shared (`Arc`), so this copy is cheap; `export` reads only the assets and the
                // background of the document it is given.
                let mut assets = self.doc.assets.clone();
                for id in swap {
                    if let Some(a) = assets.get_mut(&id) {
                        *a = Asset {
                            mime: "image/svg+xml".to_owned(),
                            data: Arc::new(PLACEHOLDER.to_vec()),
                        };
                    }
                }
                let light = Document {
                    name: String::new(),
                    background: self.doc.background,
                    pages: Vec::new(),
                    assets,
                };
                inkboard_core::svg::export(
                    &light,
                    page,
                    None,
                    &ApproxMeasurer,
                    &SvgOptions::default(),
                )?
            }
        };
        self.left = self.left.saturating_sub(svg.len());
        Some(svg)
    }
}

/// A small SVG card on the board's `background` (`#rrggbb`; anything else is drawn on white) with `message` in its
/// middle: what stands in for a page that is too large to preview.
pub fn card_svg(background: &str, message: &str) -> String {
    let hex = background.len() == 7
        && background.starts_with('#')
        && background[1..].bytes().all(|b| b.is_ascii_hexdigit());
    let fill = if hex { background } else { "#ffffff" };
    let text: String = message
        .chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            c => c.to_string(),
        })
        .collect();
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360" viewBox="0 0 640 360"><rect width="640" height="360" fill="{fill}"/><text x="320" y="180" text-anchor="middle" dominant-baseline="central" font-family="system-ui, sans-serif" font-size="20" fill="#8b93a1">{text}</text></svg>"##
    )
}

/// Refuses what the engine's own readers accept but its SVG export and text extraction index blindly: a table whose
/// cells are not rows × columns (`Table::cell` and the export panic on it).
fn check_shapes(doc: &Document) -> Result<(), BoardError> {
    for (index, page) in doc.pages.iter().enumerate() {
        for o in &page.objects {
            if let Kind::Table(t) = &o.kind
                && (t.rows() == 0
                    || t.cols() == 0
                    || t.rows().checked_mul(t.cols()) != Some(t.cells.len()))
            {
                return Err(BoardError::invalid(format!(
                    "page {index}: the table {} has {} cells for {} rows and {} columns",
                    o.id.as_str(),
                    t.cells.len(),
                    t.rows(),
                    t.cols()
                )));
            }
        }
    }
    Ok(())
}

/// Bytes of the SVG markup around one object (its group, transform, rotation, opacity and stroke attributes).
const OBJECT_COST: usize = 512;
/// Bytes of a drop shadow filter and its group.
const SHADOW_COST: usize = 512;
/// Bytes of a text block's group.
const TEXT_COST: usize = 256;
/// Bytes of one text line (`<text …>`), besides its two numbers.
const LINE_COST: usize = 160;
/// Bytes of one run (`<tspan …>` and a highlight rectangle), besides its numbers and text.
const RUN_COST: usize = 200;
/// Bytes of both arrowheads of a connector, besides their numbers.
const HEADS_COST: usize = 1200;
/// Points of a connector's path per point it is given (elbows and curves add points).
const PATH_POINTS: usize = 32;

/// The estimate of one page's SVG.
struct Estimate<'a> {
    /// Everything but the image data (image objects counted with the placeholder).
    drawing: usize,
    /// Per drawn asset: the bytes its data adds, once per image object that draws it.
    images: BTreeMap<&'a AssetId, usize>,
}

impl Estimate<'_> {
    /// The assets to draw as placeholders so that the page fits `allowance` (largest share first).
    fn over(&self, allowance: usize) -> Vec<AssetId> {
        let mut total = self
            .images
            .values()
            .fold(self.drawing, |t, b| t.saturating_add(*b));
        if total <= allowance {
            return Vec::new();
        }
        let mut shares: Vec<(&AssetId, usize)> =
            self.images.iter().map(|(k, v)| (*k, *v)).collect();
        shares.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let mut swap = Vec::new();
        for (id, bytes) in shares {
            if total <= allowance {
                break;
            }
            total -= bytes;
            swap.push(id.clone());
        }
        swap
    }
}

/// Base64 length of `n` bytes.
fn b64(n: usize) -> usize {
    n.div_ceil(3).saturating_mul(4)
}

/// The printed width of a number of magnitude `m` as the SVG export writes it (`{:.2}`, sign included), plus slack.
fn num_width(m: f64) -> usize {
    let m = m.abs();
    if !m.is_finite() || m < 1.0 {
        return 6;
    }
    // At most 309 integer digits for an f64.
    (m.log10().floor() as usize).min(310) + 7
}

/// The largest magnitude of the numbers the export prints for `page` (positions, sizes, widths, font sizes).
fn magnitude(page: &Page) -> f64 {
    let mut m: f64 = 0.0;
    let mut see = |v: f64| {
        if v.is_finite() {
            m = m.max(v.abs());
        }
    };
    let frame = |see: &mut dyn FnMut(f64), f: &inkboard_core::geom::Frame| {
        see(f.x.abs() + f.w.abs());
        see(f.y.abs() + f.h.abs());
        see(f.angle.to_degrees());
    };
    for o in &page.objects {
        see(o.style.width);
        match &o.kind {
            Kind::Box(b) => {
                frame(&mut see, &b.frame);
                see(b.label.size);
            }
            Kind::Image(i) => frame(&mut see, &i.frame),
            Kind::Pen(p) => {
                for pt in p.pts.iter() {
                    see(pt.x);
                    see(pt.y);
                }
            }
            Kind::Connector(c) => {
                for pt in &c.via {
                    see(pt.x);
                    see(pt.y);
                }
                for end in [&c.start, &c.end] {
                    if let End::Free { at } = end {
                        see(at.x);
                        see(at.y);
                    }
                }
                if let Some(l) = &c.label {
                    see(l.size);
                }
            }
            Kind::Table(t) => {
                see(t.origin.x.abs() + t.col_w.iter().map(|w| w.abs()).sum::<f64>());
                see(t.origin.y.abs() + t.row_h.iter().map(|h| h.abs()).sum::<f64>());
                see(t.size);
            }
        }
    }
    m
}

/// The bytes of a laid-out text block, its lines placed from a position of magnitude `m`.
fn text_cost(lay: &TextLayout, m: f64) -> usize {
    let w = num_width(m + lay.height.abs());
    let mut cost = TEXT_COST;
    for line in &lay.lines {
        cost = cost.saturating_add(LINE_COST + 2 * w);
        for run in line {
            // `esc` turns `"` into `&quot;`: at most six bytes per byte.
            cost = cost.saturating_add(RUN_COST + 4 * w + 6 * run.text.len());
        }
    }
    cost
}

/// The bytes an image's data adds: its base64, three times for an SVG whose colours are replaced (a short colour
/// name can become `#rrggbb`).
fn image_bytes(i: &ImageObj, a: &Asset) -> usize {
    let grows = a.mime == "image/svg+xml" && (i.is_tinted() || !i.recolor.is_empty());
    b64(a.data.len().saturating_mul(if grows { 3 } else { 1 }))
}

/// An upper estimate of `page`'s SVG, from the model alone.
fn estimate<'a>(doc: &'a Document, page: &'a Page) -> Estimate<'a> {
    let m = magnitude(page);
    let w = num_width(m);
    let mut drawing = 1024 + 8 * w;
    let mut images: BTreeMap<&AssetId, usize> = BTreeMap::new();
    for o in page.objects.iter().filter(|o| page.is_visible(o)) {
        let mut cost = OBJECT_COST + 8 * w;
        if o.style.shadow {
            cost += SHADOW_COST + 4 * w;
        }
        let kind = match &o.kind {
            Kind::Box(b) => {
                let label = if b.label.text.is_empty() {
                    0
                } else {
                    text_cost(&b.layout(&ApproxMeasurer), m)
                };
                (24 * w).saturating_add(label)
            }
            Kind::Pen(p) => {
                let n = p.pts.len().saturating_add(2);
                if p.pressures.is_some() && !p.highlighter {
                    // Smoothed four times, then both sides of the stroke: about eight outline points per point.
                    n.saturating_mul(8).saturating_mul(2 * w + 3)
                } else {
                    n.saturating_mul(4 * w + 4)
                }
            }
            Kind::Connector(c) => {
                let points = c.via.len().saturating_add(2).saturating_mul(PATH_POINTS);
                let label = match &c.label {
                    Some(l) if !l.text.is_empty() => text_cost(
                        &layout_rich(&l.text, None, &ApproxMeasurer, l.family, l.size),
                        m,
                    ),
                    _ => 0,
                };
                points
                    .saturating_mul(2 * w + 3)
                    .saturating_add(HEADS_COST + 24 * w)
                    .saturating_add(label)
            }
            Kind::Image(i) => {
                if let Some(a) = doc.assets.get(&i.asset) {
                    let e = images.entry(&i.asset).or_default();
                    *e = e.saturating_add(image_bytes(i, a));
                }
                8 * w + b64(PLACEHOLDER.len())
            }
            Kind::Table(t) => table_cost(t, m, w),
        };
        drawing = drawing.saturating_add(cost).saturating_add(kind);
    }
    Estimate { drawing, images }
}

fn table_cost(t: &Table, m: f64, w: usize) -> usize {
    let (rows, cols) = (t.rows(), t.cols());
    let mut cost = 2 * (160 + 4 * w) + (rows + cols).saturating_mul(120 + 4 * w);
    if cols == 0 || rows.checked_mul(cols) != Some(t.cells.len()) {
        return cost;
    }
    for (i, cell) in t.cells.iter().enumerate() {
        if cell.is_empty() {
            continue;
        }
        // As the export lays out a cell: its column width less the padding.
        let width = (t.col_w[i % cols] - 16.0).max(1.0);
        let lay = layout_rich(cell, Some(width), &ApproxMeasurer, t.family, t.size);
        cost = cost.saturating_add(text_cost(&lay, m));
    }
    cost
}

/// The first [`EXCERPT_BYTES`] of `text`, cut back to a character boundary.
pub fn excerpt(text: &str) -> &str {
    if text.len() <= EXCERPT_BYTES {
        return text;
    }
    let mut end = EXCERPT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn text_of(doc: &Document) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.trim().is_empty() {
            lines.push(s);
        }
    };
    for page in &doc.pages {
        push(page.name.clone());
        for o in &page.objects {
            match &o.kind {
                Kind::Box(b) => push(b.label.text.to_plain()),
                Kind::Connector(c) => {
                    if let Some(l) = &c.label {
                        push(l.text.to_plain());
                    }
                }
                Kind::Table(t) => {
                    for r in 0..t.rows() {
                        let cells: Vec<String> = (0..t.cols())
                            .map(|c| t.cell(r, c).map(|x| x.to_plain()).unwrap_or_default())
                            .collect();
                        push(cells.join("\t"));
                    }
                }
                Kind::Image(i) => {
                    if let Some(latex) = &i.latex {
                        push(latex.clone());
                    }
                }
                Kind::Pen(_) => {}
            }
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkboard_core::connector::{Connector, End};
    use inkboard_core::geom::{Frame, Pt};
    use inkboard_core::model::{BoxGeom, ImageObj, Label, Object, Page, PenObj, Table};
    use inkboard_core::style::{Align, Style};
    use inkboard_core::text::RichText;
    use serde_json::{Value, json};

    const PNG: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;

    fn image(asset: &inkboard_core::model::AssetId, x: f64, latex: Option<&str>) -> Object {
        Object::new(
            Kind::Image(ImageObj {
                frame: Frame::new(x, 0.0, 40.0, 40.0),
                asset: asset.clone(),
                latex: latex.map(str::to_owned),
                mono: false,
                recolor: Vec::new(),
            }),
            Style::default(),
        )
    }

    /// Two images sharing one asset, a third image with another asset, a labelled box: the fixture of the round trip.
    fn board_with_images() -> Document {
        let mut d = Document::new("Original name");
        let png = d.add_asset("image/png", PNG.to_vec());
        let svg = d.add_asset("image/svg+xml", SVG.to_vec());
        let label = Label::new("Hello", 20.0, Align::Center);
        d.pages[0].objects.extend([
            Object::new_box(
                BoxGeom::Rect,
                Frame::new(0.0, 0.0, 120.0, 60.0),
                Style::default(),
                label,
            ),
            image(&png, 200.0, None),
            image(&png, 300.0, None),
            image(&svg, 400.0, Some("x^2")),
        ]);
        d
    }

    fn no_assets() -> HashMap<String, Vec<u8>> {
        HashMap::new()
    }

    fn bytes_of(split: &Split) -> HashMap<String, Vec<u8>> {
        split
            .assets
            .iter()
            .map(|a| (a.id.clone(), a.bytes.clone()))
            .collect()
    }

    #[test]
    fn split_splice_roundtrip_byte_identical() {
        let doc = board_with_images();
        for pretty in [false, true] {
            let full = serial::to_json(&doc, pretty);
            let split = split(&full).unwrap();
            assert_eq!(split.format_version, 2);
            assert_eq!(split.assets.len(), 2, "the shared asset is one entry");
            let png = split.assets.iter().find(|a| a.mime == "image/png").unwrap();
            assert_eq!(png.bytes, PNG);
            let svg = split
                .assets
                .iter()
                .find(|a| a.mime == "image/svg+xml")
                .unwrap();
            assert_eq!(svg.bytes, SVG);
            assert!(split.stripped.len() < full.len());
            let back = splice(&split.stripped, &bytes_of(&split), "Original name").unwrap();
            assert_eq!(back, full, "pretty = {pretty}");
            // The spliced envelope is the same document for the engine.
            assert_eq!(serial::from_json(&back).unwrap(), doc);
        }
    }

    #[test]
    fn splice_sets_the_title_and_leaves_missing_assets_empty() {
        let full = serial::to_json(&board_with_images(), false);
        let split = split(&full).unwrap();
        let back = splice(&split.stripped, &no_assets(), "Quire \"title\" ü").unwrap();
        let v: serde_json::Value = serde_json::from_str(&back).unwrap();
        assert_eq!(v["document"]["name"], "Quire \"title\" ü");
        for a in v["document"]["assets"].as_object().unwrap().values() {
            assert_eq!(a["data"], "");
        }
        // One asset back: only that one has data.
        let png = split.assets.iter().find(|a| a.mime == "image/png").unwrap();
        let one = HashMap::from([(png.id.clone(), png.bytes.clone())]);
        let back = splice(&split.stripped, &one, "t").unwrap();
        let v: serde_json::Value = serde_json::from_str(&back).unwrap();
        assert_eq!(
            v["document"]["assets"][&png.id]["data"],
            STANDARD.encode(PNG)
        );
    }

    #[test]
    fn stripped_keeps_asset_entries_with_empty_data() {
        let doc = board_with_images();
        let full = serial::to_json(&doc, false);
        let split = split(&full).unwrap();
        let v: serde_json::Value = serde_json::from_str(&split.stripped).unwrap();
        let assets = v["document"]["assets"].as_object().unwrap();
        assert_eq!(assets.len(), 2);
        for (id, a) in assets {
            assert_eq!(a["data"], "", "{id}");
            let mime = a["mime"].as_str().unwrap();
            assert!(mime == "image/png" || mime == "image/svg+xml");
        }
        // The engine still sees every image: sanitize keeps images whose asset entry exists (empty data).
        let stripped_doc = serial::from_json(&split.stripped).unwrap();
        assert_eq!(stripped_doc.pages[0].objects.len(), 4);
        assert!(stripped_doc.assets.values().all(|a| a.data.is_empty()));
    }

    #[test]
    fn split_decodes_escaped_base64_and_drops_a_bom() {
        let encoded = STANDARD.encode(b"\xff\xfe\xfd binary/with+slashes?");
        let escaped = encoded.replace('/', "\\/");
        let body = format!(
            "\u{feff}{{\"format\":\"inkboard\",\"version\":2,\"document\":{{\"name\":\"n\",\"pages\":[],\
             \"assets\":{{\"a\\u0031\":{{\"data\":\"{escaped}\",\"mime\":\"image/png\"}}}}}}}}"
        );
        let split = split(&body).unwrap();
        assert_eq!(split.assets.len(), 1);
        assert_eq!(split.assets[0].id, "a1", "keys are unescaped");
        assert_eq!(split.assets[0].bytes, b"\xff\xfe\xfd binary/with+slashes?");
        assert!(!split.stripped.starts_with('\u{feff}'));
        assert!(split.stripped.contains(r#"{"data":"","mime":"image/png"}"#));
    }

    #[test]
    fn unknown_fields_survive_put_get() {
        let doc = board_with_images();
        let full = serial::to_json(&doc, false);
        // An extra field on an object, on the document, on an asset and on the envelope.
        let mut v: serde_json::Value = serde_json::from_str(&full).unwrap();
        v["document"]["pages"][0]["objects"][0]["x_future"] = 1.into();
        v["document"]["x_future"] = 1.into();
        v["x_future"] = serde_json::json!({"nested": [1, 2]});
        let id = v["document"]["assets"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        v["document"]["assets"][&id]["x_future"] = true.into();
        let full = serde_json::to_string(&v).unwrap();
        // Validation accepts it (the engine ignores what it does not know) …
        let inspected = inspect(&full).unwrap();
        assert_eq!(inspected.meta.objects, 4);
        // … and the stored form and the GET keep it, byte for byte.
        let split = split(&full).unwrap();
        assert_eq!(split.stripped.matches("x_future").count(), 4);
        let back = splice(&split.stripped, &bytes_of(&split), "Original name").unwrap();
        assert_eq!(back, full);
        let renamed = splice(&split.stripped, &bytes_of(&split), "Renamed").unwrap();
        let r: serde_json::Value = serde_json::from_str(&renamed).unwrap();
        assert_eq!(r["document"]["pages"][0]["objects"][0]["x_future"], 1);
        assert_eq!(r["document"]["x_future"], 1);
        assert_eq!(r["x_future"]["nested"][1], 2);
        assert_eq!(r["document"]["name"], "Renamed");
    }

    #[test]
    fn version_3_rejected_board_too_new() {
        let full = serial::to_json(&board_with_images(), false).replacen(
            r#""version":2"#,
            r#""version":3"#,
            1,
        );
        for e in [
            split(&full).unwrap_err(),
            inspect(&full).unwrap_err(),
            splice(&full, &no_assets(), "t").unwrap_err(),
        ] {
            assert_eq!(e, BoardError::TooNew(3));
            assert_eq!(e.code(), ErrorCode::BoardTooNew);
        }
        // Older versions are read.
        let older = serial::to_json(&Document::new(""), false).replacen(
            r#""version":2"#,
            r#""version":1"#,
            1,
        );
        assert_eq!(split(&older).unwrap().format_version, 1);
        inspect(&older).unwrap();
    }

    #[test]
    fn tableau_blanc_accepted() {
        let full = serial::to_json(&board_with_images(), false).replacen(
            r#""format":"inkboard""#,
            r#""format":"tableau-blanc""#,
            1,
        );
        inspect(&full).unwrap();
        let split = split(&full).unwrap();
        assert!(split.stripped.contains(r#""format":"tableau-blanc""#));
        assert_eq!(
            splice(&split.stripped, &bytes_of(&split), "Original name").unwrap(),
            full
        );
    }

    #[test]
    fn other_formats_and_broken_json_rejected() {
        for body in [
            r#"{"format":"excalidraw","version":2,"document":{"name":"x","pages":[]}}"#,
            "{",
            "[]",
            r#"{"format":"inkboard","version":2}"#,
            r#"{"format":"inkboard","version":2,"document":[]}"#,
            r#"{"format":"inkboard","version":2,"document":{"name":"x","pages":[],"assets":{"a":{"mime":"image/png","data":"!!"}}}}"#,
        ] {
            let e = split(body).unwrap_err();
            assert_eq!(e.code(), ErrorCode::InvalidBoard, "{body}: {e}");
            let e = inspect(body).unwrap_err();
            assert_eq!(e.code(), ErrorCode::InvalidBoard, "{body}: {e}");
        }
        // Structurally fine for split, but not a document: only inspect (from_json) refuses it.
        let body = r#"{"format":"inkboard","version":2,"document":{"pages":"no"}}"#;
        split(body).unwrap();
        assert_eq!(inspect(body).unwrap_err().code(), ErrorCode::InvalidBoard);
    }

    #[test]
    fn splice_inserts_a_missing_name() {
        let body = r#"{"format":"inkboard","version":2,"document":{"pages":[]}}"#;
        let back = splice(body, &no_assets(), "T").unwrap();
        assert_eq!(
            back,
            r#"{"format":"inkboard","version":2,"document":{"name":"T","pages":[]}}"#
        );
        let empty = r#"{"format":"inkboard","version":2,"document":{}}"#;
        assert_eq!(
            splice(empty, &no_assets(), "T").unwrap(),
            r#"{"format":"inkboard","version":2,"document":{"name":"T"}}"#
        );
    }

    #[test]
    fn inspect_text_includes_labels_tables_pages_latex() {
        let mut d = board_with_images();
        d.pages[0].name = "First page".into();
        let a = d.pages[0].objects[0].id.clone();
        let mut sticky = Object::new_box(
            BoxGeom::Sticky,
            Frame::new(0.0, 200.0, 160.0, 160.0),
            Style::default(),
            Label::new("Sticky note", 20.0, Align::Left),
        );
        sticky.id = inkboard_core::model::Id::from("sticky");
        let mut conn = Connector::new(End::bound(a), End::bound(sticky.id.clone()));
        conn.label = Some(Label::new("connects", 16.0, Align::Center));
        let mut table = Table::new(Pt::new(500.0, 500.0), 2, 2);
        *table.cell_mut(0, 0).unwrap() = RichText::plain("Name");
        *table.cell_mut(0, 1).unwrap() = RichText::plain("Value");
        *table.cell_mut(1, 1).unwrap() = RichText::plain("42");
        d.pages[0].objects.extend([
            sticky,
            Object::new(Kind::Connector(conn), Style::default()),
            Object::new(Kind::Table(table), Style::default()),
            Object::new_pen(
                vec![Pt::new(0.0, 0.0), Pt::new(5.0, 5.0)],
                false,
                Style::default(),
            ),
        ]);
        let mut second = Page::new("Second page");
        second.objects.push(Object::new_box(
            BoxGeom::Text,
            Frame::new(0.0, 0.0, 50.0, 20.0),
            Style::default(),
            Label::new("Free text", 20.0, Align::Left),
        ));
        d.pages.push(second);
        let full = serial::to_json(&d, false);
        let i = inspect(&full).unwrap();
        assert_eq!(
            i.text,
            "First page\nHello\nx^2\nSticky note\nconnects\nName\tValue\n\t42\nSecond page\nFree text"
        );
        assert_eq!(i.meta.pages.len(), 2);
        assert_eq!(i.meta.pages[0].name, "First page");
        assert_eq!(i.meta.pages[0].id, d.pages[0].id.as_str());
        assert_eq!(i.meta.pages[1].name, "Second page");
        assert_eq!(i.meta.objects, 9);
        assert_eq!(i.meta.background, "#ffffff");
        assert_eq!(i.meta.bytes as usize, full.len());
        assert_eq!(i.meta.text_excerpt, i.text);
    }

    #[test]
    fn excerpt_is_cut_at_a_char_boundary() {
        let text = "é".repeat(EXCERPT_BYTES);
        let cut = excerpt(&text);
        assert!(cut.len() <= EXCERPT_BYTES && cut.len() > EXCERPT_BYTES - 2);
        assert!(cut.chars().all(|c| c == 'é'));
        assert_eq!(excerpt("short"), "short");
    }

    #[test]
    fn previews_per_page_and_empty_pages() {
        let mut d = board_with_images();
        d.pages.push(Page::new("Empty"));
        let full = serial::to_json(&d, false);
        let all = previews(&full).unwrap();
        assert_eq!(all.len(), 2);
        let svg = all[0].as_deref().unwrap();
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("Hello"));
        assert_eq!(all[1], None, "an empty page has no SVG");
        assert_eq!(preview(&full, 0).unwrap().as_deref(), Some(svg));
        assert_eq!(preview(&full, 1).unwrap(), None);
        assert_eq!(preview(&full, 9).unwrap(), None);
        // Within the budgets, a preview is exactly Inkboard's export.
        assert_eq!(
            Some(svg),
            inkboard_core::svg::export_approx(&d, &d.pages[0]).as_deref()
        );
    }

    /// Bytes that start like a PNG, `n` long.
    fn big_png(n: usize) -> Vec<u8> {
        PNG.iter()
            .copied()
            .chain((0..n).map(|i| (i % 251) as u8))
            .take(n)
            .collect()
    }

    #[test]
    fn repeated_image_previews_are_bounded() {
        // One 1 MiB image drawn 1,000 times is a 1.4 MB envelope, but 1.4 GB of SVG if every copy were inlined.
        let mut d = Document::new("");
        let big = big_png(1 << 20);
        let a = d.add_asset("image/png", big.clone());
        let small = d.add_asset("image/png", PNG.to_vec());
        for k in 0..1000 {
            d.pages[0]
                .objects
                .push(image(&a, f64::from(k) * 10.0, None));
        }
        d.pages[0].objects.push(image(&small, -100.0, None));
        let full = serial::to_json(&d, false);
        assert!(full.len() < 2 << 20);
        let svg = preview(&full, 0).unwrap().unwrap();
        assert!(svg.len() < 1 << 20, "{} bytes", svg.len());
        assert!(
            svg.contains(&STANDARD.encode(PNG)),
            "the small image is kept"
        );
        assert!(svg.contains(&STANDARD.encode(PLACEHOLDER)));
        assert!(!svg.contains(&STANDARD.encode(&big[..300])));
        // A few copies fit, and are drawn as they are.
        d.pages[0].objects.truncate(3);
        let full = serial::to_json(&d, false);
        let svg = preview(&full, 0).unwrap().unwrap();
        assert_eq!(svg.matches(&STANDARD.encode(&big[..300])).count(), 3);
        assert!(!svg.contains(&STANDARD.encode(PLACEHOLDER)));
    }

    #[test]
    fn oversized_drawing_becomes_a_card_and_the_board_budget_ends_the_list() {
        // Numbers are printed in full: 20,000 points at 1e300 are a 0.6 MB envelope and a 25 MB path.
        let mut d = Document::new("");
        d.background.preset = inkboard_core::model::BgPreset::Slate;
        let pts: Vec<Pt> = (0..20_000).map(|i| Pt::new(1e300, f64::from(i))).collect();
        d.pages[0]
            .objects
            .push(Object::new_pen(pts, false, Style::default()));
        let mut second = Page::new("Second");
        second.objects.push(Object::new_box(
            BoxGeom::Rect,
            Frame::new(0.0, 0.0, 200.0, 80.0),
            Style::default(),
            Label::new("fits", 20.0, Align::Center),
        ));
        d.pages.push(second.clone());
        d.pages.push(second);
        let full = serial::to_json(&d, false);
        let all = previews(&full).unwrap();
        assert_eq!(all.len(), 3);
        let card = all[0].as_deref().unwrap();
        assert_eq!(card, card_svg(&d.background.preset.hex(), TOO_LARGE));
        assert!(card.contains(TOO_LARGE) && card.len() < 1024);
        assert!(all[1].as_deref().unwrap().contains("fits"));
        // With the board budget almost spent, a page becomes the card and the pages after it get none.
        let mut p = Previewer::new(&full).unwrap();
        p.left = 100;
        assert_eq!(p.render(1).as_deref(), Some(card));
        assert!(p.spent());
        assert_eq!(card_svg("not a colour", "<b>"), card_svg("#ffffff", "<b>"));
        assert!(card_svg("#ffffff", "<b>").contains("&lt;b&gt;"));
    }

    #[test]
    fn estimate_bounds_the_export() {
        let mut d = board_with_images();
        let a = d.pages[0].objects[0].id.clone();
        let mut shadowed = Object::new_box(
            BoxGeom::Diamond,
            Frame::new(-500.0, 900.0, 60.0, 30.0),
            Style {
                shadow: true,
                ..Style::default()
            },
            Label::new(&"long words wrap ".repeat(40), 18.0, Align::Left),
        );
        shadowed.id = inkboard_core::model::Id::from("shadowed");
        let mut conn = Connector::new(End::bound(a), End::bound(shadowed.id.clone()));
        conn.via = vec![Pt::new(10.0, 400.0), Pt::new(-300.0, 450.0)];
        conn.label = Some(Label::new("via two points", 16.0, Align::Center));
        let mut table = Table::new(Pt::new(500.0, 500.0), 3, 4);
        for r in 0..3 {
            for c in 0..4 {
                *table.cell_mut(r, c).unwrap() =
                    RichText::plain(format!("cell {r} {c} with \"quotes\" & more"));
            }
        }
        let n = 400;
        let pts: Vec<Pt> = (0..n)
            .map(|i| Pt::new(f64::from(i) * 3.25, (f64::from(i) * 0.1).sin() * 80.0))
            .collect();
        d.pages[0].objects.extend([
            shadowed,
            Object::new(Kind::Connector(conn), Style::default()),
            Object::new(Kind::Table(table), Style::default()),
            Object::new_pen(pts.clone(), false, Style::default()),
            Object::new(
                Kind::Pen(PenObj {
                    pts: Arc::new(pts),
                    highlighter: false,
                    pressures: Some(Arc::new((0..n).map(|i| (i % 10) as f32 / 10.0).collect())),
                }),
                Style::default(),
            ),
        ]);
        let page = &d.pages[0];
        let e = estimate(&d, page);
        let total = e.images.values().sum::<usize>() + e.drawing;
        let real = inkboard_core::svg::export_approx(&d, page).unwrap();
        assert!(total >= real.len(), "{total} < {}", real.len());
        assert!(total < 20 * real.len(), "{total} vs {}", real.len());
    }

    #[test]
    fn malformed_tables_refused() {
        let mut d = Document::new("");
        let mut t = Table::new(Pt::new(0.0, 0.0), 2, 2);
        t.cells.pop();
        d.pages[0]
            .objects
            .push(Object::new(Kind::Table(t), Style::default()));
        let full = serial::to_json(&d, false);
        let e = inspect(&full).unwrap_err();
        assert_eq!(e.code(), ErrorCode::InvalidBoard);
        assert!(
            e.to_string().contains("3 cells for 2 rows and 2 columns"),
            "{e}"
        );
        assert!(Previewer::new(&full).is_err());
        let mut d = Document::new("");
        let mut t = Table::new(Pt::new(0.0, 0.0), 1, 1);
        t.col_w.clear();
        t.cells.clear();
        d.pages[0]
            .objects
            .push(Object::new(Kind::Table(t), Style::default()));
        assert!(inspect(&serial::to_json(&d, false)).is_err());
    }

    /// The shared M1.1 fixture (§18.2): two cards (targets [`P1`], [`P2`], labels "First page", "Second page") and a
    /// plain box, in the engine's canonical form.
    const EMBED_FIXTURE: &str = include_str!("../../../fixtures/boards/embed.inkboard");
    const P1: &str = "0192f0e0-0000-7000-8000-000000000001";
    const P2: &str = "0192f0e0-0000-7000-8000-000000000002";

    fn none() -> Vec<String> {
        Vec::new()
    }

    /// The fixture with its page 0 objects changed by `f`.
    fn with_objects(f: impl FnOnce(&mut Vec<Value>)) -> String {
        let mut v: Value = serde_json::from_str(EMBED_FIXTURE).unwrap();
        f(v["document"]["pages"][0]["objects"].as_array_mut().unwrap());
        serde_json::to_string(&v).unwrap()
    }

    /// The fixture with `object` (raw JSON text) inserted first on page 0.
    fn with_first_object(object: &str) -> String {
        let at = r#""objects":["#;
        assert_eq!(EMBED_FIXTURE.matches(at).count(), 1);
        EMBED_FIXTURE.replacen(at, &format!("{at}{object},"), 1)
    }

    /// A pen object as the engine writes it, with `extra` (raw JSON members) added to its kind right after `type`.
    fn pen_with(extra: &str) -> String {
        let mut pen = Object::new_pen(
            vec![Pt::new(0.0, 0.0), Pt::new(5.0, 5.0)],
            false,
            Style::default(),
        );
        pen.id = inkboard_core::model::Id::from("pen1");
        let json = serde_json::to_string(&pen).unwrap();
        assert!(json.contains(r#""kind":{"type":"pen","#), "{json}");
        json.replacen(r#""type":"pen","#, &format!(r#""type":"pen",{extra},"#), 1)
    }

    fn uuid(n: u64) -> String {
        format!("0192f0e0-0000-7000-8000-{n:012x}")
    }

    #[test]
    fn embeds_found_in_rect_cards_only() {
        let i = inspect(EMBED_FIXTURE).unwrap();
        assert_eq!(i.embeds, [P1, P2], "both targets, in drawing order");
        assert_eq!(embeds(EMBED_FIXTURE).unwrap(), i.embeds);
        assert_eq!(i.meta.objects, 3);
        // The order is the drawing order.
        let swapped = with_objects(|o| o.swap(0, 1));
        assert_eq!(inspect(&swapped).unwrap().embeds, [P2, P1]);
        // The plain box is no card: without the two cards there is no target.
        let plain = with_objects(|o| {
            o.drain(0..2);
        });
        assert_eq!(inspect(&plain).unwrap().embeds, none());
        // Cards on every page count, page by page.
        let mut v: Value = serde_json::from_str(EMBED_FIXTURE).unwrap();
        let mut second = v["document"]["pages"][0].clone();
        second["id"] = "embedtest2".into();
        second["name"] = "Page 2".into();
        second["objects"][0]["kind"]["embed"]["item"] = uuid(3).into();
        second["objects"][1]["kind"]["embed"]["item"] = P1.into();
        v["document"]["pages"].as_array_mut().unwrap().push(second);
        let two_pages = serde_json::to_string(&v).unwrap();
        assert_eq!(
            inspect(&two_pages).unwrap().embeds,
            [P1.to_owned(), P2.to_owned(), uuid(3)]
        );
        // An object whose id repeats an earlier one on its page is dropped by the engine (`sanitize`), and not read.
        let repeated_id = with_objects(|o| o[1]["id"] = o[0]["id"].clone());
        assert_eq!(
            serial::from_json(&repeated_id).unwrap().pages[0]
                .objects
                .len(),
            2
        );
        assert_eq!(inspect(&repeated_id).unwrap().embeds, [P1]);
    }

    #[test]
    fn embeds_ignore_malformed_values() {
        let upper = P1.to_uppercase();
        for bad in [
            json!(5),
            json!({}),
            json!({"item": 3}),
            json!({"item": "abc"}),
            json!({"item": upper}),
            json!(null),
            json!(P1),
            json!([P1]),
            json!({"item": null}),
            json!({"item": {"id": P1}}),
            json!({"id": P1}),
            json!({"item": format!("{{{P1}}}")}),
            json!({"item": P1.replace('-', "")}),
            json!({"item": format!("urn:uuid:{P1}")}),
            json!({"item": format!("{P1} ")}),
        ] {
            let body = with_objects(|o| {
                o[0]["kind"]["embed"] = bad.clone();
                o.remove(1);
            });
            let i = inspect(&body).unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert_eq!(i.embeds, none(), "{bad}");
            assert_eq!(i.meta.objects, 2, "{bad}");
        }
        // An `embed` on any other geometry, and on any other kind, is no card.
        for geom in ["ellipse", "diamond", "triangle", "sticky", "text"] {
            let body = with_objects(|o| {
                o[0]["kind"]["geom"] = geom.into();
                o.remove(1);
            });
            assert_eq!(inspect(&body).unwrap().embeds, none(), "{geom}");
        }
        let pen = with_first_object(&pen_with(&format!(r#""embed":{{"item":"{}"}}"#, uuid(9))));
        assert_eq!(inspect(&pen).unwrap().embeds, [P1, P2]);
        // `embed` is read as the M1.1 engine reads it (a `serde_json::Value`): a repeated `item` keeps its last value.
        let last_item = |items: &str| {
            let card1 = format!(r#""embed":{{"item":"{P1}"}}"#);
            EMBED_FIXTURE.replacen(&card1, &format!(r#""embed":{{{items}}}"#), 1)
        };
        assert_eq!(
            inspect(&last_item(&format!(r#""item":3,"item":"{}""#, uuid(7))))
                .unwrap()
                .embeds,
            [uuid(7), P2.to_owned()]
        );
        assert_eq!(
            inspect(&last_item(&format!(r#""item":"{}","item":3"#, uuid(7))))
                .unwrap()
                .embeds,
            [P2]
        );
        // A body the scan cannot read gives no targets (inspect refuses it before, through `from_json`).
        assert_eq!(embeds("{").unwrap(), none());
        assert_eq!(embeds(r#"{"document":{"pages":"no"}}"#).unwrap(), none());
    }

    #[test]
    fn embeds_read_escaped_strings() {
        // The engine unescapes every key and string, so the scan does too.
        let body = EMBED_FIXTURE
            .replacen(r#""geom":"rect","#, r#""geom":"re\u0063t","#, 1)
            .replacen(
                &format!(r#""embed":{{"item":"{P1}"}}"#),
                r#""emb\u0065d":{"it\u0065m":"0192f0e0-0000-7000-8000-00000000000\u0031"}"#,
                1,
            );
        assert_ne!(body, EMBED_FIXTURE);
        assert_eq!(inspect(&body).unwrap().embeds, [P1, P2]);
        // An escaped character that leaves a non-canonical id is still not linked.
        let upper = EMBED_FIXTURE.replacen(P1, r"0192f0e0-0000-7000-8000-00000000000\u0041", 1);
        assert_eq!(inspect(&upper).unwrap().embeds, [P2]);
    }

    /// `body` with its envelope and its page 0 written as JSON arrays (by position), as serde also reads them.
    fn positional(body: &str) -> String {
        let mut v: Value = serde_json::from_str(body).unwrap();
        let mut page = v["document"]["pages"][0].take();
        v["document"]["pages"][0] = Value::Array(
            ["id", "name", "layers", "objects", "view"]
                .map(|k| page[k].take())
                .into(),
        );
        let envelope: Vec<Value> = ["format", "version", "document"]
            .map(|k| v[k].take())
            .into();
        serde_json::to_string(&envelope).unwrap()
    }

    #[test]
    fn embeds_read_positional_forms() {
        // serde reads a struct, and an internally tagged enum, from a JSON array too (fields by position), and
        // `from_json` accepts that at every level: the scan finds the same cards there, and refuses the same key.
        let frame = r#"{"x":0.0,"y":0.0,"w":10.0,"h":10.0}"#;
        let style = r#"{"stroke":"ink","fill":null,"width":3.0,"dash":"solid","opacity":1.0,"roundness":0.0}"#;
        let plain_kind = format!(r#"{{"id":"bypos","kind":["box","rect",{frame}]}}"#);
        let card_object = format!(
            r#"["objbypos",{{"type":"box","geom":"rect","frame":{frame},"embed":{{"item":"{}"}}}},{style},null,false,"main"]"#,
            uuid(4)
        );
        let body = with_first_object(&format!("{plain_kind},{card_object}"));
        assert_eq!(serial::from_json(&body).unwrap().pages[0].objects.len(), 5);
        let want = [uuid(4), P1.to_owned(), P2.to_owned()];
        assert_eq!(inspect(&body).unwrap().embeds, want);
        let all = positional(&body);
        assert!(all.starts_with(r#"["inkboard",2,{"#), "{all}");
        assert_eq!(serial::from_json(&all).unwrap().pages[0].objects.len(), 5);
        assert_eq!(inspect(&all).unwrap().embeds, want);
        // A repeated `embed` key is refused beside a kind by position, however the rest is written.
        // (Repeated after `positional`, whose `Value` would keep only one of the two keys.)
        let card1 = format!(r#""embed":{{"item":"{P1}"}}"#);
        let repeat = |b: &str| {
            assert_eq!(b.matches(&card1).count(), 1, "{b}");
            b.replacen(&card1, &format!(r#"{card1},"embed":5"#), 1)
        };
        for b in [repeat(&body), repeat(&all)] {
            assert!(serial::from_json(&b).is_ok(), "{b}");
            assert_eq!(
                inspect(&b).unwrap_err().code(),
                ErrorCode::InvalidBoard,
                "{b}"
            );
        }
        // A box kind by position is never a card: the unpatched model takes at most its four fields, so a fifth
        // element (where the M1.1 engine would read `embed`) is refused; four or fewer is a plain box.
        let label = r#"{"size":24.0}"#;
        let with_lang = format!(r#"{{"id":"lang","kind":["box","rect",{frame},{label},null]}}"#);
        assert_eq!(
            inspect(&with_first_object(&with_lang)).unwrap().embeds,
            [P1, P2]
        );
        let fifth = format!(
            r#"{{"id":"fifth","kind":["box","rect",{frame},{label},null,{{"item":"{}"}}]}}"#,
            uuid(5)
        );
        let e = inspect(&with_first_object(&fifth)).unwrap_err();
        assert_eq!(e.code(), ErrorCode::InvalidBoard);
    }

    #[test]
    fn embeds_distinct_and_capped() {
        let same = with_objects(|o| o[1]["kind"]["embed"]["item"] = P1.into());
        assert_eq!(inspect(&same).unwrap().embeds, [P1]);
        let card = serde_json::from_str::<Value>(EMBED_FIXTURE).unwrap()["document"]["pages"][0]
            ["objects"][0]
            .clone();
        let many: Vec<Value> = (0..1001)
            .map(|n| {
                let mut c = card.clone();
                c["id"] = format!("card{n}").into();
                c["kind"]["embed"]["item"] = uuid(n).into();
                c
            })
            .collect();
        // A repeated target before the cap does not take a place.
        let mut with_repeat = many.clone();
        let mut again = card.clone();
        again["id"] = "again".into();
        again["kind"]["embed"]["item"] = uuid(0).into();
        with_repeat.insert(1, again);
        for objects in [many, with_repeat] {
            let body = with_objects(|o| *o = objects);
            let found = inspect(&body).unwrap().embeds;
            assert_eq!(found.len(), EMBED_LINKS_MAX);
            assert_eq!(found, (0..1000).map(uuid).collect::<Vec<_>>());
        }
    }

    #[test]
    fn embed_fixture_round_trips_byte_identical() {
        let s = split(EMBED_FIXTURE).unwrap();
        assert!(s.assets.is_empty());
        assert_eq!(s.stripped, EMBED_FIXTURE);
        assert_eq!(
            splice(&s.stripped, &no_assets(), "Embed test").unwrap(),
            EMBED_FIXTURE
        );
        // With another title, only `document.name` changes.
        let renamed = splice(&s.stripped, &no_assets(), "Board \"title\"").unwrap();
        assert_eq!(
            renamed,
            EMBED_FIXTURE.replacen(r#""name":"Embed test""#, r#""name":"Board \"title\"""#, 1)
        );
        // The unpatched model reads the cards as plain boxes.
        let doc = serial::from_json(EMBED_FIXTURE).unwrap();
        assert!(
            doc.pages[0]
                .objects
                .iter()
                .all(|o| matches!(&o.kind, Kind::Box(b) if b.geom == BoxGeom::Rect))
        );
    }

    #[test]
    fn embed_fixture_text_and_preview() {
        let i = inspect(EMBED_FIXTURE).unwrap();
        assert_eq!(i.text, "Page 1\nFirst page\nSecond page\nPlain box");
        assert_eq!(i.meta.text_excerpt, i.text);
        let svg = preview(EMBED_FIXTURE, 0).unwrap().unwrap();
        for label in ["First page", "Second page", "Plain box"] {
            assert!(svg.contains(label), "{label} in {svg}");
        }
        // The card's title snapshot keeps its colour, on the card's white fill.
        assert!(svg.contains("#1d2430"), "{svg}");
    }

    #[test]
    fn embeds_repeated_key_is_refused() {
        let card1 = format!(r#""embed":{{"item":"{P1}"}}"#);
        assert_eq!(EMBED_FIXTURE.matches(&card1).count(), 1);
        let repeated =
            |repeat: &str| EMBED_FIXTURE.replacen(&card1, &format!("{card1},{repeat}"), 1);
        for repeat in [
            r#""embed":5"#.to_owned(),
            format!(r#""embed":{{"item":"{P2}"}}"#),
            r#""emb\u0065d":null"#.to_owned(),
        ] {
            let body = repeated(&repeat);
            // The unpatched model ignores both keys …
            assert!(serial::from_json(&body).is_ok(), "{repeat}");
            // … the M1.1 engine's would refuse the repeated field, so the server refuses the board.
            let e = inspect(&body).unwrap_err();
            assert_eq!(e.code(), ErrorCode::InvalidBoard, "{repeat}");
            let message = e.to_string();
            assert!(message.contains("duplicate field `embed`"), "{message}");
            assert!(
                message.contains("page 0: the object embedcard1"),
                "{message}"
            );
            assert_eq!(embeds(&body).unwrap_err(), e);
        }
        // Whatever the geometry: the engine's box model knows `embed` on every box.
        let ellipse =
            repeated(r#""embed":5"#).replacen(r#""geom":"rect""#, r#""geom":"ellipse""#, 1);
        assert_eq!(
            inspect(&ellipse).unwrap_err().code(),
            ErrorCode::InvalidBoard
        );
        // The key once in each of two boxes (the fixture itself) is fine …
        assert!(inspect(EMBED_FIXTURE).is_ok());
        // … and so is a kind whose `type` holds an escape: it is still a box, and its card counts.
        let escaped = EMBED_FIXTURE.replacen(r#""type":"box""#, r#""type":"bo\u0078""#, 1);
        assert_ne!(escaped, EMBED_FIXTURE);
        assert_eq!(inspect(&escaped).unwrap().embeds, [P1, P2]);
        // A repeated key in a kind that knows no `embed` (a pen) is ignored by both models.
        let pen = with_first_object(&pen_with(r#""embed":1,"embed":2,"geom":"a","geom":"b""#));
        assert!(serial::from_json(&pen).is_ok());
        assert_eq!(inspect(&pen).unwrap().embeds, [P1, P2]);
    }
}
