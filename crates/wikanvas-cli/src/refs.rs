//! References to items and collections: a UUID, a `url_id`, or a Quire URL (owner: W8)
//!
//! §7.6 "References": wherever a command takes `<i>` or `<c>`, it may be a UUID, a `url_id` (10 base62
//! characters), or a Quire URL such as `https://wiki.example.org/doc/my-title-AbCdEfGhIj`, from which the CLI takes
//! the `url_id`. Only `GET /items/{id}` and `GET /collections/{id}` accept a `url_id`, so the commands resolve one to
//! its UUID with that GET first.

use crate::error::Failure;

/// A parsed reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    /// A UUID, lowercased.
    Uuid(String),
    /// A `url_id`.
    UrlId(String),
}

/// Whether `s` is a UUID in the 8-4-4-4-12 hex form.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Whether `s` is a `url_id`: exactly 10 ASCII alphanumerics.
pub fn is_url_id(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|c| c.is_ascii_alphanumeric())
}

/// Parses a reference. `what` names it in the error ("item", "collection").
pub fn parse(input: &str, what: &str) -> Result<Ref, Failure> {
    let s = input.trim();
    if is_uuid(s) {
        return Ok(Ref::Uuid(s.to_ascii_lowercase()));
    }
    if is_url_id(s) {
        return Ok(Ref::UrlId(s.to_owned()));
    }
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        let path = rest.split(['?', '#']).next().unwrap_or_default();
        let last = path
            .split('/')
            .skip(1)
            .filter(|seg| !seg.is_empty())
            .last()
            .unwrap_or_default();
        let candidate = last.rsplit('-').next().unwrap_or_default();
        if is_url_id(candidate) {
            return Ok(Ref::UrlId(candidate.to_owned()));
        }
        if is_uuid(last) {
            return Ok(Ref::Uuid(last.to_ascii_lowercase()));
        }
    }
    Err(Failure::usage(format!(
        "{input:?} is not a {what} reference: use a UUID, a url_id (10 letters and digits) or a Quire URL"
    )))
}

/// Parses an id that must be a UUID (attachments, tokens, sessions).
pub fn uuid(input: &str, what: &str) -> Result<String, Failure> {
    let s = input.trim();
    if is_uuid(s) {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(Failure::usage(format!(
            "{input:?} is not a {what} id (a UUID)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0192f3a4-5b6c-7d8e-9f01-23456789abcd";

    #[test]
    fn uuids_url_ids_and_urls() {
        assert_eq!(parse(ID, "item").unwrap(), Ref::Uuid(ID.into()));
        assert_eq!(
            parse(&ID.to_ascii_uppercase(), "item").unwrap(),
            Ref::Uuid(ID.into())
        );
        assert_eq!(
            parse("AbCdEfGhIj", "item").unwrap(),
            Ref::UrlId("AbCdEfGhIj".into())
        );
        for url in [
            "https://wiki.example.org/doc/my-title-AbCdEfGhIj",
            "https://wiki.example.org/doc/my-title-AbCdEfGhIj/",
            "http://localhost:8080/board/untitled-AbCdEfGhIj?x=1#y",
            "https://wiki.example.org/collection/AbCdEfGhIj",
        ] {
            assert_eq!(
                parse(url, "item").unwrap(),
                Ref::UrlId("AbCdEfGhIj".into()),
                "{url}"
            );
        }
        assert_eq!(
            parse(&format!("https://wiki.example.org/x/{ID}"), "item").unwrap(),
            Ref::Uuid(ID.into())
        );
    }

    #[test]
    fn refuses_anything_else() {
        for bad in [
            "",
            "abc",
            "AbCdEfGhI",
            "AbCdEfGhIjK",
            "AbCdEfGh-j",
            "https://wiki.example.org/",
            "https://wiki.example.org/doc/title-short",
            "ftp://x/doc/a-AbCdEfGhIj",
            "0192f3a4-5b6c-7d8e-9f01-23456789abcz",
        ] {
            let err = parse(bad, "item").expect_err(bad);
            assert_eq!(err.exit, quire_api::cli::ExitCode::Usage, "{bad}");
        }
        assert!(uuid("AbCdEfGhIj", "attachment").is_err());
        assert_eq!(uuid(ID, "attachment").unwrap(), ID);
    }
}
