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
//! - [`inspect`] validates the full envelope and derives the text and [`Meta`] (§5 `board_state.meta`).
//! - [`preview`] / [`previews`] render page SVGs with `svg::export_approx` (D10).

use std::collections::HashMap;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use inkboard_core::model::{Document, Kind};
use inkboard_core::serial::{self, LoadError};
use serde::de::{Deserializer, MapAccess, Visitor};
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
    /// entries left out.
    pub text: String,
    pub meta: Meta,
}

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

/// Validates a full envelope with `serial::from_json` (422 `invalid_board` / `board_too_new`) and derives the search
/// text and the meta (§6.2). `meta.bytes` is the length of `full`.
pub fn inspect(full: &str) -> Result<Inspect, BoardError> {
    let doc = serial::from_json(full)?;
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
    Ok(Inspect { text, meta })
}

/// The SVG of page `page` (0-based) of a full envelope, `None` when the page is empty or does not exist.
pub fn preview(full: &str, page: usize) -> Result<Option<String>, BoardError> {
    let doc = serial::from_json(full)?;
    Ok(doc
        .pages
        .get(page)
        .and_then(|p| inkboard_core::svg::export_approx(&doc, p)))
}

/// The SVG of every page of a full envelope, parsed once; `None` for an empty page.
pub fn previews(full: &str) -> Result<Vec<Option<String>>, BoardError> {
    let doc = serial::from_json(full)?;
    Ok(doc
        .pages
        .iter()
        .map(|p| inkboard_core::svg::export_approx(&doc, p))
        .collect())
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
    use inkboard_core::model::{BoxGeom, ImageObj, Label, Object, Page, Table};
    use inkboard_core::style::{Align, Style};
    use inkboard_core::text::RichText;

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
    }
}
