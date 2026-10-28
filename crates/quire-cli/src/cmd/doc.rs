//! The `quire doc` commands: document content as Markdown or ProseMirror JSON (owner: W8)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::{Ctx, Res, print_json_text};
use crate::args::{DocCmd, DocGet, DocPut};
use crate::error::Failure;
use crate::http::{self, Req};
use crate::markdown::{self, Local};
use crate::output;

/// The upload limit when `GET /config` cannot tell (the compose default, `QUIRE_MAX_UPLOAD_MB=25`).
const DEFAULT_MAX_UPLOAD_MB: u64 = 25;

pub fn run(ctx: &Ctx, cmd: DocCmd) -> Res {
    match cmd {
        DocCmd::Get(a) => get(ctx, a),
        DocCmd::Put(a) => put(ctx, a),
        DocCmd::Schema => ctx.call(Req::get("/api/v1/schema/doc"), None),
    }
}

fn get(ctx: &Ctx, a: DocGet) -> Res {
    let md = a.format == "md";
    if a.title_h1 && !md {
        return Err(Failure::usage("--title-h1 is for --format md"));
    }
    let id = ctx.item_id(&a.item)?;
    let req = if md {
        let r = Req::get(format!("/api/v1/items/{id}/doc/markdown"));
        if a.title_h1 {
            r.query("title", "include")
        } else {
            r
        }
    } else {
        Req::get(format!("/api/v1/items/{id}/doc/prosemirror"))
    };
    let resp = ctx.client()?.send(&req)?;
    let etag = http::header(resp.headers(), "etag");
    let warnings: Vec<String> = http::header(
        resp.headers(),
        quire_api::common::header::CONVERSION_WARNINGS,
    )
    .map(|h| {
        h.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default();
    if !output::json() {
        if let Some(e) = &etag {
            output::note(&format!("etag: {e}"));
        }
        for w in &warnings {
            output::note(&format!("warning: {w}"));
        }
    }
    match a.out {
        Some(path) => {
            let mut file = std::fs::File::create(&path)
                .map_err(|e| Failure::io(&format!("creating {}", path.display()), &e))?;
            let w = http::stream_to(resp, &mut file, &format!("writing {}", path.display()))?;
            if output::json() {
                output::print_json(&json!({
                    "item_id": id, "format": a.format, "etag": etag, "path": path, "bytes": w.bytes,
                    "sha256": w.sha256,
                }))
            } else {
                output::note(&format!("wrote {} bytes to {}", w.bytes, path.display()));
                Ok(())
            }
        }
        None => {
            let text = http::read_text(resp)?;
            if !output::json() {
                return output::stdout_bytes(text.as_bytes());
            }
            if md {
                output::print_json(&json!({
                    "item_id": id, "format": "md", "etag": etag, "content": text, "warnings": warnings,
                }))
            } else {
                let content = RawValue::from_string(text).map_err(|e| {
                    Failure::bad_response(format!("GET …/doc/prosemirror: not JSON: {e}"))
                })?;
                output::print_json(&json!({
                    "item_id": id, "format": "json", "etag": etag, "content": content,
                }))
            }
        }
    }
}

/// The body of a ProseMirror PUT: the file as it is, or a bare `{"type": "doc", …}` wrapped in
/// `{"schema_version": 1, "doc": …}`.
fn prosemirror_body(bytes: Vec<u8>) -> Result<Vec<u8>, Failure> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(rename = "type")]
        ty: Option<String>,
    }
    let probe: Probe = serde_json::from_slice(&bytes)
        .map_err(|e| Failure::usage(format!("--json: not a JSON object: {e}")))?;
    if probe.ty.as_deref() == Some("doc") {
        let mut out = br#"{"schema_version":1,"doc":"#.to_vec();
        out.extend_from_slice(&bytes);
        out.push(b'}');
        Ok(out)
    } else {
        Ok(bytes)
    }
}

fn put(ctx: &Ctx, a: DocPut) -> Res {
    let if_match = super::precondition(&a.precondition)?;
    let id = ctx.item_id(&a.item)?;
    let req = if let Some(spec) = &a.md {
        let bytes = ctx.read_input(spec, "--md")?;
        let mut text = String::from_utf8(bytes)
            .map_err(|_| Failure::usage(format!("--md {spec}: not UTF-8")))?;
        if a.attach_images {
            text = attach_images(ctx, &id, spec, a.base_dir.as_deref(), text)?;
        }
        let mut r = Req::put(format!("/api/v1/items/{id}/doc/markdown"))
            .raw(text.into_bytes(), "text/markdown; charset=utf-8");
        if a.title_h1 {
            r = r.query("title", "from_h1");
        }
        r
    } else {
        let spec = a.json.as_deref().unwrap_or("-");
        let body = prosemirror_body(ctx.read_input(spec, "--json")?)?;
        Req::put(format!("/api/v1/items/{id}/doc/prosemirror")).raw(body, "application/json")
    };
    let mut req = req.header("If-Match", if_match);
    if a.merge {
        req = req.query("merge", "true");
    }
    let text = http::read_text(ctx.client()?.send(&req)?)?;
    if output::json() {
        return print_json_text(&text);
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::bad_response(format!("PUT …/doc: not JSON: {e}")))?;
    output::note(&format!("etag: {}", v["etag"].as_str().unwrap_or("-")));
    for w in v["warnings"].as_array().into_iter().flatten() {
        output::note(&format!(
            "warning: {} (×{}{})",
            w["code"].as_str().unwrap_or("-"),
            w["count"],
            w["path"]
                .as_str()
                .map(|p| format!(", at {p}"))
                .unwrap_or_default()
        ));
    }
    output::print_text(&format!(
        "seq {}, changed: {}, merged: {}",
        v["seq"], v["changed"], v["merged"]
    ))
}

/// The server's upload limit in bytes (`GET /config`), or 25 MiB when it cannot tell.
fn upload_limit(ctx: &Ctx) -> u64 {
    let mb = ctx
        .client()
        .and_then(|c| c.send(&Req::get("/api/v1/config").public()))
        .and_then(http::read_text)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.pointer("/limits/max_upload_mb").and_then(Value::as_u64))
        .unwrap_or(DEFAULT_MAX_UPLOAD_MB);
    mb.saturating_mul(1 << 20)
}

/// `--attach-images`: uploads the local images under the base directory and rewrites their destinations to the
/// attachment URLs; every other local reference is an `attach_skipped` warning (§7.6 "Guards").
fn attach_images(
    ctx: &Ctx,
    item_id: &str,
    spec: &str,
    base_dir: Option<&Path>,
    md: String,
) -> Result<String, Failure> {
    let base: PathBuf = match base_dir {
        Some(d) => d.to_path_buf(),
        None if spec == "-" => {
            return Err(Failure::usage(
                "--attach-images with --md - needs --base-dir DIR",
            ));
        }
        None => Path::new(spec)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf(),
    };
    let base = std::fs::canonicalize(&base)
        .map_err(|e| Failure::usage(format!("the base directory {}: {e}", base.display())))?;
    let refs = markdown::find_images(&md);
    if refs.is_empty() {
        return Ok(md);
    }
    let max = upload_limit(ctx);
    let mut uploaded: HashMap<PathBuf, String> = HashMap::new();
    let mut replacements = Vec::new();
    for r in refs {
        match markdown::classify(&r.dest, &base, max) {
            Local::External => {}
            Local::Skip(reason) => output::warn(
                "attach_skipped",
                &format!("image {}: {reason}", r.dest),
                Some(json!({ "path": r.dest, "reason": reason })),
            ),
            Local::Upload(path) => {
                let url = match uploaded.get(&path) {
                    Some(u) => u.clone(),
                    None => {
                        let bytes = std::fs::read(&path).map_err(|e| {
                            Failure::usage(format!("cannot read {}: {e}", path.display()))
                        })?;
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "image".into());
                        let req = Req::post("/api/v1/attachments")
                            .query("item_id", item_id)
                            .query("filename", name)
                            .raw(bytes, "application/octet-stream")
                            .transfer();
                        let v = ctx.get_value(req)?;
                        let url = v["url"]
                            .as_str()
                            .ok_or_else(|| {
                                Failure::bad_response("POST /attachments: no url in the response")
                            })?
                            .to_owned();
                        uploaded.insert(path, url.clone());
                        url
                    }
                };
                replacements.push((r, url));
            }
        }
    }
    Ok(markdown::rewrite(&md, &replacements))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_docs_are_wrapped() {
        let wrapped = prosemirror_body(br#"{"type":"doc","content":[]}"#.to_vec()).unwrap();
        let v: Value = serde_json::from_slice(&wrapped).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["doc"]["type"], "doc");
        let full = br#"{"schema_version":1,"doc":{"type":"doc"}}"#.to_vec();
        assert_eq!(prosemirror_body(full.clone()).unwrap(), full);
        assert!(prosemirror_body(b"[1]".to_vec()).is_err());
    }
}
