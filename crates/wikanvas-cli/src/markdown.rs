//! `doc put --attach-images`: the local images a Markdown file references (owner: W8)
//!
//! §7.6 "Guards": only images referenced by relative paths that canonicalize (symlinks resolved) to a file under the
//! Markdown file's directory or `--base-dir`, whose magic bytes are png, jpeg, gif, webp or avif, and whose size is
//! within `max_upload_mb`, are uploaded; every other local reference is skipped with an `attach_skipped` warning.
//! URLs (`https:`, `data:`, `//host`), fragments and `/api/v1/attachments/…` are not local files and stay as they
//! are. Inline images (`![alt](dest "title")`) outside code blocks and code spans are recognized.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One inline image destination in the Markdown, with its byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    pub start: usize,
    pub end: usize,
    pub dest: String,
}

/// The inline image destinations outside fenced code blocks and code spans, in order.
pub fn find_images(md: &str) -> Vec<ImageRef> {
    let mut found = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        if indent <= 3
            && let Some(&c) = trimmed.as_bytes().first()
            && (c == b'`' || c == b'~')
        {
            let n = trimmed.bytes().take_while(|&b| b == c).count();
            if n >= 3 {
                match fence {
                    None => {
                        fence = Some((c, n));
                        continue;
                    }
                    Some((fc, fnum)) if fc == c && n >= fnum && trimmed[n..].trim().is_empty() => {
                        fence = None;
                        continue;
                    }
                    Some(_) => {}
                }
            }
        }
        if fence.is_some() {
            continue;
        }
        scan_line(line, start, &mut found);
    }
    found
}

fn scan_line(line: &str, base: usize, found: &mut Vec<ImageRef>) {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            // A code span: skip to the closing run of the same length.
            let n = b[i..].iter().take_while(|&&c| c == b'`').count();
            let ticks = &line[i..i + n];
            match line[i + n..].find(ticks) {
                Some(close) => i += n + close + n,
                None => i += n,
            }
            continue;
        }
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b'!'
            && b.get(i + 1) == Some(&b'[')
            && let Some(found_ref) = parse_image(line, i + 2, base)
        {
            i = found_ref.end - base;
            found.push(found_ref);
            continue;
        }
        i += 1;
    }
}

/// Parses `alt](dest …)` starting after `![`.
fn parse_image(line: &str, from: usize, base: usize) -> Option<ImageRef> {
    let b = line.as_bytes();
    let mut depth = 0usize;
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' if depth == 0 => break,
            b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if b.get(i) != Some(&b']') || b.get(i + 1) != Some(&b'(') {
        return None;
    }
    let mut j = i + 2;
    while b.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
        j += 1;
    }
    if b.get(j) == Some(&b'<') {
        let close = line[j + 1..].find('>')? + j + 1;
        return Some(ImageRef {
            start: base + j + 1,
            end: base + close,
            dest: line[j + 1..close].to_owned(),
        });
    }
    let start = j;
    let mut parens = 0usize;
    while let Some(&c) = b.get(j) {
        match c {
            b'\\' => j += 1,
            b'(' => parens += 1,
            b')' if parens == 0 => break,
            b')' => parens -= 1,
            c if c.is_ascii_whitespace() => break,
            _ => {}
        }
        j += 1;
    }
    let end = j.min(b.len());
    (end > start).then(|| ImageRef {
        start: base + start,
        end: base + end,
        dest: line[start..end].to_owned(),
    })
}

/// What to do with a destination.
#[derive(Debug, PartialEq, Eq)]
pub enum Local {
    /// Not a local file (a URL, an attachment, a fragment): left as it is, silently.
    External,
    /// A local reference that is not uploaded, and why.
    Skip(String),
    /// Upload this file (canonical path).
    Upload(PathBuf),
}

fn has_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.split_once(':').is_some_and(|(scheme, _)| {
            scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The image type from the magic bytes: png, jpeg, gif, webp or avif.
pub fn image_kind(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("webp")
    } else if head.len() >= 12
        && &head[4..8] == b"ftyp"
        && head[8..head.len().min(64)]
            .chunks(4)
            .any(|brand| brand == b"avif" || brand == b"avis")
    {
        Some("avif")
    } else {
        None
    }
}

/// Classifies one destination against the base directory (already canonical) and the size limit.
pub fn classify(dest: &str, base: &Path, max_bytes: u64) -> Local {
    let dest = dest.trim();
    if dest.is_empty()
        || dest.starts_with('#')
        || dest.starts_with("//")
        || has_scheme(dest)
        || dest.starts_with("/api/v1/attachments/")
    {
        return Local::External;
    }
    if dest.starts_with('/') {
        return Local::Skip(
            "an absolute path (only paths relative to the base directory are uploaded)".into(),
        );
    }
    let path_part = dest.split(['?', '#']).next().unwrap_or_default();
    let relative = percent_decode(path_part);
    let joined = base.join(&relative);
    let canonical = match fs::canonicalize(&joined) {
        Ok(p) => p,
        Err(e) => return Local::Skip(format!("not found: {e}")),
    };
    if !canonical.starts_with(base) {
        return Local::Skip("outside the base directory (after resolving symlinks)".into());
    }
    let meta = match fs::metadata(&canonical) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Local::Skip("not a regular file".into()),
        Err(e) => return Local::Skip(format!("cannot read: {e}")),
    };
    if meta.len() > max_bytes {
        return Local::Skip(format!("larger than the upload limit of {max_bytes} bytes"));
    }
    let mut head = [0u8; 64];
    let n = match fs::File::open(&canonical).and_then(|mut f| {
        let mut total = 0;
        while total < head.len() {
            match f.read(&mut head[total..])? {
                0 => break,
                n => total += n,
            }
        }
        Ok(total)
    }) {
        Ok(n) => n,
        Err(e) => return Local::Skip(format!("cannot read: {e}")),
    };
    if image_kind(&head[..n]).is_none() {
        return Local::Skip("not an image (png, jpeg, gif, webp or avif)".into());
    }
    Local::Upload(canonical)
}

/// Replaces each range with its new destination (ranges in order, not overlapping).
pub fn rewrite(md: &str, replacements: &[(ImageRef, String)]) -> String {
    let mut out = String::with_capacity(md.len());
    let mut at = 0;
    for (r, url) in replacements {
        out.push_str(&md[at..r.start]);
        out.push_str(url);
        at = r.end;
    }
    out.push_str(&md[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_inline_images_outside_code() {
        let md = "# T\n![a](one.png) and ![b [x]](<two words.png> \"t\")\n`![c](no.png)`\n```\n![d](no.png)\n```\n\
                  ![e](https://x/y.png) ![f](dir/(p).png)\n";
        let dests: Vec<String> = find_images(md).into_iter().map(|r| r.dest).collect();
        assert_eq!(
            dests,
            ["one.png", "two words.png", "https://x/y.png", "dir/(p).png"]
        );
        let refs = find_images(md);
        assert_eq!(&md[refs[0].start..refs[0].end], "one.png");
        let out = rewrite(md, &[(refs[0].clone(), "/api/v1/attachments/1".into())]);
        assert!(out.contains("![a](/api/v1/attachments/1) and"), "{out}");
    }

    #[test]
    fn magic_bytes() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpeg"));
        assert_eq!(image_kind(b"GIF89a.."), Some("gif"));
        assert_eq!(image_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(
            image_kind(b"\0\0\0\x1cftypavif\0\0\0\0avifmif1"),
            Some("avif")
        );
        assert_eq!(image_kind(b"<svg"), None);
        assert_eq!(image_kind(b""), None);
    }

    #[test]
    fn classification() {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        fs::write(base.join("ok.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        fs::write(base.join("text.png"), b"not an image").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::os::unix::fs::symlink(outside.path().join("x.png"), base.join("link.png")).unwrap();
        assert_eq!(
            classify("ok.png", &base, 1000),
            Local::Upload(base.join("ok.png"))
        );
        assert_eq!(
            classify("./ok%2Epng", &base, 1000),
            Local::Upload(base.join("ok.png"))
        );
        for local in [
            "../x.png",
            "/etc/passwd",
            "link.png",
            "text.png",
            "missing.png",
        ] {
            assert!(
                matches!(classify(local, &base, 1000), Local::Skip(_)),
                "{local}"
            );
        }
        assert!(matches!(classify("ok.png", &base, 3), Local::Skip(_)));
        for not_local in [
            "https://x/y.png",
            "data:image/png;base64,xx",
            "//h/x.png",
            "#a",
            "/api/v1/attachments/x",
        ] {
            assert_eq!(
                classify(not_local, &base, 1000),
                Local::External,
                "{not_local}"
            );
        }
    }
}
