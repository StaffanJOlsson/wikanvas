//! Identifiers: UUIDv7 ids, 10-character base62 url_ids and title slugs (owner: W1).
//!
//! D4: every primary key is a UUIDv7 (TEXT in SQLite); items and collections also carry a `url_id`, 10 random base62
//! characters, used only in URLs (`/doc/<slug>-<url_id>`). [`slugify`] is the slug rule the web client uses
//! (`web/src/lib/urls.ts`), so a server-built URL or file name (`<slug>.md`) matches the SPA's.

use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

/// The length of a `url_id` (D4).
pub const URL_ID_LEN: usize = 10;

/// The longest slug [`slugify`] returns.
pub const MAX_SLUG_LEN: usize = 60;

/// The base62 alphabet, in ASCII order (`0-9A-Za-z`), shared with the sort keys of [`crate::rank`].
pub const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// A new UUIDv7 (time-ordered) id.
pub fn new_uuid() -> Uuid {
    Uuid::now_v7()
}

/// `n` random base62 characters, uniformly distributed (bytes ≥ 248 are drawn again).
pub fn random_base62(n: usize) -> Result<String, getrandom::Error> {
    let mut out = String::with_capacity(n);
    let mut buf = [0u8; 64];
    while out.len() < n {
        getrandom::fill(&mut buf)?;
        for &b in &buf {
            if out.len() == n {
                break;
            }
            if b < 248 {
                out.push(char::from(BASE62[usize::from(b % 62)]));
            }
        }
    }
    Ok(out)
}

/// A new `url_id`: [`URL_ID_LEN`] random base62 characters (about 59 bits).
pub fn new_url_id() -> Result<String, getrandom::Error> {
    random_base62(URL_ID_LEN)
}

/// Whether `s` has the shape of a `url_id`: exactly [`URL_ID_LEN`] ASCII letters and digits.
pub fn is_url_id(s: &str) -> bool {
    s.len() == URL_ID_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The slug of a title, as the web client builds it: compatibility-decomposed (NFKD) with combining accents
/// (U+0300–U+036F) removed, lowercased, every run of characters other than `a-z0-9` replaced by one `-`, dashes trimmed
/// at both ends, cut to [`MAX_SLUG_LEN`] characters (and trimmed again), `untitled` when nothing is left. "Café déjà
/// vu" → `cafe-deja-vu`. Only cosmetic: routes resolve by the url_id.
pub fn slugify(title: &str) -> String {
    let folded: String = title
        .nfkd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .collect::<String>()
        .to_lowercase();
    let mut slug = String::with_capacity(folded.len().min(MAX_SLUG_LEN * 2));
    let mut dash = false;
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            dash = false;
            slug.push(c);
        } else {
            dash = true;
        }
    }
    // Every character is ASCII now, so byte and character counts agree.
    slug.truncate(MAX_SLUG_LEN);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "untitled".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_v7_and_ordered() {
        let a = new_uuid();
        let b = new_uuid();
        assert_eq!(a.get_version_num(), 7);
        assert!(a < b, "UUIDv7 ids sort by creation time");
    }

    #[test]
    fn url_ids_have_the_alphabet_and_length() {
        let id = new_url_id().unwrap();
        assert!(is_url_id(&id), "{id}");
        assert_ne!(new_url_id().unwrap(), id);
        assert!(!is_url_id("short"));
        assert!(!is_url_id("aB3dE5gH9-"));
        assert!(!is_url_id("aB3dE5gH9kX"));
        assert_eq!(random_base62(0).unwrap(), "");
        assert_eq!(random_base62(300).unwrap().len(), 300);
    }

    #[test]
    fn slugs_match_the_web_rule() {
        for (title, slug) in [
            ("Café déjà vu", "cafe-deja-vu"),
            ("  Hello, World!  ", "hello-world"),
            ("Ärende 42 – plan", "arende-42-plan"),
            ("", "untitled"),
            ("!!!", "untitled"),
            ("日本語", "untitled"),
            ("ﬁle №1", "file-no1"),
            ("İstanbul", "istanbul"),
            ("a--b__c", "a-b-c"),
            ("Straße", "stra-e"),
        ] {
            assert_eq!(slugify(title), slug, "{title:?}");
        }
        let long = "word ".repeat(30);
        let s = slugify(&long);
        assert!(s.len() <= MAX_SLUG_LEN && !s.ends_with('-'), "{s}");
        assert_eq!(s, "word-".repeat(12).trim_end_matches('-'));
    }
}
