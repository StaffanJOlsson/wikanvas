//! Opaque pagination cursors (D46, ARCHITECTURE §7.1; owner: W1, written complete in W0b).
//!
//! A cursor is the base64url (no padding) form of
//!
//! ```text
//! version (1 = 0x01) ‖ kind (1) ‖ query hash (8) ‖ payload ‖ check (4)
//! kind 0x01 keyset: sort tag (1: 0x01 integer, 0x02 text) ‖ sort value ‖ id length (u16 BE) ‖ id (UTF-8)
//!                   integer: i64 BE; text: length (u32 BE) ‖ UTF-8
//! kind 0x02 offset: offset (u32 BE)
//! check = the first 4 bytes of sha256(everything before it)
//! ```
//!
//! The query hash binds a cursor to the normalized query that produced it ([`query_hash`]), so a cursor reused with
//! other parameters is refused ([`CursorError::QueryMismatch`]); the check rejects truncated, edited or garbled
//! cursors ([`CursorError::Malformed`]). Nothing in a cursor is secret, so it is not signed. Both errors are answered
//! with 422 `validation`, `details.field = "cursor"` ([`CursorError::FIELD`]).
//!
//! Keyset pages continue strictly after `(sort, id)` of the last row of the previous page; relevance-sorted search
//! uses [`Cursor::Offset`] (capped at `wikanvas_api::page::MAX_OFFSET_RESULTS` by the caller).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

const VERSION: u8 = 1;
const KIND_KEYSET: u8 = 1;
const KIND_OFFSET: u8 = 2;
const SORT_INT: u8 = 1;
const SORT_TEXT: u8 = 2;
const HEADER_LEN: usize = 2 + 8;
const CHECK_LEN: usize = 4;

/// The longest cursor string [`decode`] looks at; anything longer is malformed without being decoded. (Text sort
/// values are titles, at most 1000 characters, so real cursors stay far below this.)
pub const MAX_ENCODED_LEN: usize = 8192;

/// Where the next page starts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// After the row with this sort value and id (the id breaks ties).
    Keyset { sort: SortValue, id: String },
    /// After this many results (relevance-sorted search).
    Offset(u32),
}

/// The sort column value of a keyset cursor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SortValue {
    /// Integer columns: timestamps (ms), counts.
    Int(i64),
    /// Text columns: titles, sort keys.
    Text(String),
}

impl Cursor {
    /// A keyset cursor over an integer column.
    pub fn after_int(sort: i64, id: impl Into<String>) -> Self {
        Cursor::Keyset {
            sort: SortValue::Int(sort),
            id: id.into(),
        }
    }

    /// A keyset cursor over a text column.
    pub fn after_text(sort: impl Into<String>, id: impl Into<String>) -> Self {
        Cursor::Keyset {
            sort: SortValue::Text(sort.into()),
            id: id.into(),
        }
    }
}

/// Why a cursor was refused. Both map to 422 `validation` with `details.field = "cursor"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CursorError {
    /// Not a cursor this server wrote: bad base64, wrong length, unknown version or kind, a failed check.
    #[error("the cursor is not valid")]
    Malformed,
    /// A valid cursor, but for other query parameters.
    #[error("the cursor belongs to a different query")]
    QueryMismatch,
}

impl CursorError {
    /// The `details.field` of the 422 answer.
    pub const FIELD: &'static str = "cursor";
}

/// The 8-byte hash that binds cursors to a query: sha256 over the `(name, value)` pairs sorted by name (then
/// value), each length-prefixed. The caller passes what identifies the result set, normalized (defaults filled in,
/// ids canonical), including the list's own name (e.g. `("list", "recent")`) so a cursor of one list is refused by
/// another, and leaves out `cursor` itself (and `limit`, so a client may change its page size between pages).
pub fn query_hash(params: &[(&str, &str)]) -> [u8; 8] {
    let mut sorted: Vec<&(&str, &str)> = params.iter().collect();
    sorted.sort();
    let mut h = Sha256::new();
    h.update(b"wikanvas-cursor-query-v1");
    for (name, value) in sorted {
        h.update((name.len() as u64).to_be_bytes());
        h.update(name.as_bytes());
        h.update((value.len() as u64).to_be_bytes());
        h.update(value.as_bytes());
    }
    first_bytes(&h.finalize())
}

/// Encodes a cursor for the query with this hash.
pub fn encode(c: &Cursor, query_hash: [u8; 8]) -> String {
    let mut buf = Vec::with_capacity(64);
    buf.push(VERSION);
    match c {
        Cursor::Keyset { sort, id } => {
            buf.push(KIND_KEYSET);
            buf.extend_from_slice(&query_hash);
            match sort {
                SortValue::Int(v) => {
                    buf.push(SORT_INT);
                    buf.extend_from_slice(&v.to_be_bytes());
                }
                SortValue::Text(t) => {
                    buf.push(SORT_TEXT);
                    let len = u32::try_from(t.len()).expect("sort value longer than 4 GiB");
                    buf.extend_from_slice(&len.to_be_bytes());
                    buf.extend_from_slice(t.as_bytes());
                }
            }
            let len = u16::try_from(id.len()).expect("cursor id longer than 64 KiB");
            buf.extend_from_slice(&len.to_be_bytes());
            buf.extend_from_slice(id.as_bytes());
        }
        Cursor::Offset(n) => {
            buf.push(KIND_OFFSET);
            buf.extend_from_slice(&query_hash);
            buf.extend_from_slice(&n.to_be_bytes());
        }
    }
    let check: [u8; CHECK_LEN] = first_bytes(&Sha256::digest(&buf));
    buf.extend_from_slice(&check);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Decodes a cursor and checks that it was made for the query with this hash.
pub fn decode(s: &str, query_hash: [u8; 8]) -> Result<Cursor, CursorError> {
    if s.is_empty() || s.len() > MAX_ENCODED_LEN {
        return Err(CursorError::Malformed);
    }
    let raw = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| CursorError::Malformed)?;
    if raw.len() < HEADER_LEN + CHECK_LEN {
        return Err(CursorError::Malformed);
    }
    let (body, check) = raw.split_at(raw.len() - CHECK_LEN);
    if first_bytes::<CHECK_LEN>(&Sha256::digest(body)) != check {
        return Err(CursorError::Malformed);
    }
    if body[0] != VERSION {
        return Err(CursorError::Malformed);
    }
    let kind = body[1];
    if body[2..HEADER_LEN] != query_hash {
        return Err(CursorError::QueryMismatch);
    }
    let mut r = Reader(&body[HEADER_LEN..]);
    let cursor = match kind {
        KIND_KEYSET => {
            let sort = match r.u8()? {
                SORT_INT => SortValue::Int(i64::from_be_bytes(r.array()?)),
                SORT_TEXT => {
                    let len = u32::from_be_bytes(r.array()?) as usize;
                    SortValue::Text(r.utf8(len)?)
                }
                _ => return Err(CursorError::Malformed),
            };
            let len = usize::from(u16::from_be_bytes(r.array()?));
            Cursor::Keyset {
                sort,
                id: r.utf8(len)?,
            }
        }
        KIND_OFFSET => Cursor::Offset(u32::from_be_bytes(r.array()?)),
        _ => return Err(CursorError::Malformed),
    };
    if !r.0.is_empty() {
        return Err(CursorError::Malformed);
    }
    Ok(cursor)
}

fn first_bytes<const N: usize>(digest: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&digest[..N]);
    out
}

/// A bounds-checked reader over the payload.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CursorError> {
        if self.0.len() < n {
            return Err(CursorError::Malformed);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, CursorError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CursorError> {
        Ok(first_bytes(self.take(N)?))
    }

    fn utf8(&mut self, n: usize) -> Result<String, CursorError> {
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CursorError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn h(list: &str) -> [u8; 8] {
        query_hash(&[
            ("list", list),
            ("collection_id", "0199f0a4-9a2b-7c3d-8e4f-123456789abc"),
        ])
    }

    fn samples() -> Vec<Cursor> {
        vec![
            Cursor::after_int(1_761_643_800_123, "0199f0a4-9a2b-7c3d-8e4f-123456789abc"),
            Cursor::after_int(i64::MIN, ""),
            Cursor::after_int(-1, "x"),
            Cursor::after_text(
                "Ärende 𝄞 «quoted» \"x\"",
                "0199f0a4-0000-7000-8000-000000000000",
            ),
            Cursor::after_text("", "id"),
            Cursor::after_text("a0V", "b"),
            Cursor::Offset(0),
            Cursor::Offset(975),
            Cursor::Offset(u32::MAX),
        ]
    }

    #[test]
    fn roundtrip_and_query_binding() {
        for c in samples() {
            let s = encode(&c, h("recent"));
            assert!(
                s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "{s}"
            );
            assert_eq!(decode(&s, h("recent")), Ok(c.clone()));
            // The same cursor under another query hash: refused.
            assert_eq!(
                decode(&s, h("updated")),
                Err(CursorError::QueryMismatch),
                "{c:?}"
            );
            assert_eq!(decode(&s, [0; 8]), Err(CursorError::QueryMismatch));
            // Tampered: any single changed byte of the decoded form fails the check.
            let raw = URL_SAFE_NO_PAD.decode(&s).unwrap();
            for i in 0..raw.len() {
                let mut t = raw.clone();
                t[i] ^= 0x01;
                assert_eq!(
                    decode(&URL_SAFE_NO_PAD.encode(&t), h("recent")),
                    Err(CursorError::Malformed),
                    "{c:?} byte {i}"
                );
            }
            // Tampered in its string form.
            let mut chars: Vec<char> = s.chars().collect();
            let mid = chars.len() / 2;
            chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
            let edited: String = chars.into_iter().collect();
            assert!(decode(&edited, h("recent")).is_err(), "{edited}");
            // Truncated or extended.
            assert_eq!(
                decode(&s[..s.len() - 1], h("recent")),
                Err(CursorError::Malformed)
            );
            assert_eq!(
                decode(&s[..s.len() - 4], h("recent")),
                Err(CursorError::Malformed)
            );
            assert_eq!(
                decode(&format!("{s}AA"), h("recent")),
                Err(CursorError::Malformed)
            );
        }
    }

    #[test]
    fn a_rechecked_forgery_with_another_hash_is_a_query_mismatch() {
        // Nothing is secret: rewriting the hash and recomputing the check gives a well-formed cursor of another query.
        let s = encode(&Cursor::Offset(25), h("recent"));
        let mut raw = URL_SAFE_NO_PAD.decode(&s).unwrap();
        raw.truncate(raw.len() - CHECK_LEN);
        raw[2..10].copy_from_slice(&h("updated"));
        let check: [u8; 4] = first_bytes(&Sha256::digest(&raw));
        raw.extend_from_slice(&check);
        let forged = URL_SAFE_NO_PAD.encode(&raw);
        assert_eq!(decode(&forged, h("updated")), Ok(Cursor::Offset(25)));
        assert_eq!(
            decode(&forged, h("recent")),
            Err(CursorError::QueryMismatch)
        );
    }

    #[test]
    fn garbage_is_malformed() {
        let q = h("recent");
        let valid = encode(&Cursor::Offset(1), q);
        for bad in [
            "",
            "=",
            "AAAA",
            "!!!!",
            "not a cursor",
            &format!("{valid}="),
            &format!("+{}", &valid[1..]),
        ] {
            assert_eq!(decode(bad, q), Err(CursorError::Malformed), "{bad:?}");
        }
        assert_eq!(
            decode(&"A".repeat(MAX_ENCODED_LEN + 1), q),
            Err(CursorError::Malformed)
        );
        // Well-formed envelope, but unknown version, kind or sort tag, invalid UTF-8, or trailing bytes.
        let forge = |body: &[u8]| {
            let mut raw = body.to_vec();
            let check: [u8; 4] = first_bytes(&Sha256::digest(&raw));
            raw.extend_from_slice(&check);
            URL_SAFE_NO_PAD.encode(raw)
        };
        let with = |version: u8, kind: u8, payload: &[u8]| {
            let mut b = vec![version, kind];
            b.extend_from_slice(&q);
            b.extend_from_slice(payload);
            forge(&b)
        };
        assert_eq!(
            decode(&with(1, 2, &7u32.to_be_bytes()), q),
            Ok(Cursor::Offset(7))
        );
        assert_eq!(
            decode(&with(2, 2, &7u32.to_be_bytes()), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 3, &7u32.to_be_bytes()), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 2, &[0, 0, 7]), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 2, &[0, 0, 0, 7, 0]), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 1, &[9, 0, 0]), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 1, &[2, 0, 0, 0, 2, 0xff, 0xfe, 0, 0]), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 1, &[2, 0, 0, 0, 9, b'a']), q),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            decode(&with(1, 1, &[2, 0, 0, 0, 1, b'a', 0, 1, b'b']), q),
            Ok(Cursor::after_text("a", "b"))
        );
    }

    #[test]
    fn query_hash_is_order_independent_and_unambiguous() {
        assert_eq!(
            query_hash(&[("a", "1"), ("b", "2")]),
            query_hash(&[("b", "2"), ("a", "1")])
        );
        assert_ne!(
            query_hash(&[("a", "1"), ("b", "2")]),
            query_hash(&[("a", "1"), ("b", "3")])
        );
        assert_ne!(
            query_hash(&[("a", "1")]),
            query_hash(&[("a", "1"), ("b", "")])
        );
        // Length prefixes keep ("ab","c") apart from ("a","bc").
        assert_ne!(query_hash(&[("ab", "c")]), query_hash(&[("a", "bc")]));
        assert_ne!(query_hash(&[]), query_hash(&[("", "")]));
    }

    #[test]
    fn error_field_is_cursor() {
        assert_eq!(CursorError::FIELD, "cursor");
        assert_eq!(
            CursorError::Malformed.to_string(),
            "the cursor is not valid"
        );
    }

    fn any_cursor() -> impl Strategy<Value = Cursor> {
        prop_oneof![
            (any::<i64>(), ".{0,40}").prop_map(|(v, id)| Cursor::after_int(v, id)),
            (".{0,200}", ".{0,40}").prop_map(|(t, id)| Cursor::after_text(t, id)),
            any::<u32>().prop_map(Cursor::Offset),
        ]
    }

    proptest! {
        #[test]
        fn roundtrips(c in any_cursor(), a in any::<[u8; 8]>(), b in any::<[u8; 8]>()) {
            let s = encode(&c, a);
            prop_assert_eq!(decode(&s, a), Ok(c));
            if a != b {
                prop_assert_eq!(decode(&s, b), Err(CursorError::QueryMismatch));
            }
        }

        #[test]
        fn never_panics_on_input(s in "[A-Za-z0-9_=+/-]{0,120}") {
            let _ = decode(&s, [7; 8]);
        }
    }
}
