//! The personal access token format (D42): `qpat_` + 40 base62 random characters + 6 base62 characters of the
//! CRC32 of those 40, 51 characters in all. The checksum lets the CLI reject a mistyped token without a request and
//! lets the server refuse a malformed one without a database lookup. No randomness here: the caller draws the bytes
//! (`getrandom`) and stores only `sha256(token)` and the 12-character [`display_prefix`] (owner: W2).
//!
//! Invite links (D148) carry a token of the same shape with the prefix `qinv_` ([`encode_invite`],
//! [`check_invite`]), so an invite can never pass for an API token, and the server stores only its sha256 too.

use std::fmt;

/// Every token starts with this; `is_wikanvas_bearer` (server) tests `Authorization: Bearer qpat_…`.
pub const PREFIX: &str = "qpat_";
/// Random base62 characters after the prefix (about 238 bits).
pub const SECRET_LEN: usize = 40;
/// Base62 characters of the CRC32 checksum.
pub const CHECKSUM_LEN: usize = 6;
/// The whole token.
pub const TOKEN_LEN: usize = PREFIX.len() + SECRET_LEN + CHECKSUM_LEN;
/// The stored and listed prefix: `qpat_` + 7 characters.
pub const DISPLAY_PREFIX_LEN: usize = 12;
/// Random bytes [`encode`] takes.
pub const RANDOM_BYTES: usize = 32;
/// Every invite token starts with this (D148): the last path segment of `<public URL>/invite/<token>`.
pub const INVITE_PREFIX: &str = "qinv_";

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Why a string is not a well-formed token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenFormatError {
    /// It does not start with `qpat_`.
    Prefix,
    /// It is not 51 characters long.
    Length,
    /// A character after the prefix is not base62.
    Alphabet,
    /// The last 6 characters are not the CRC32 of the 40 before them.
    Checksum,
}

impl fmt::Display for TokenFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TokenFormatError::Prefix => "a Wikanvas token starts with qpat_",
            TokenFormatError::Length => "a Wikanvas token is 51 characters long",
            TokenFormatError::Alphabet => {
                "a Wikanvas token holds only the characters 0-9, A-Z and a-z after qpat_"
            }
            TokenFormatError::Checksum => {
                "the token's checksum does not match (mistyped or truncated?)"
            }
        })
    }
}

impl std::error::Error for TokenFormatError {}

/// Builds a token from 32 random bytes: the bytes, read as one big-endian number, give the 40 secret characters
/// (the number modulo 62⁴⁰; the bias is below 2⁻¹⁷), then the checksum is appended.
pub fn encode(random: &[u8; RANDOM_BYTES]) -> String {
    encode_with(PREFIX, random)
}

/// An invite token from 32 random bytes: [`encode`] with the prefix `qinv_` (51 characters).
pub fn encode_invite(random: &[u8; RANDOM_BYTES]) -> String {
    encode_with(INVITE_PREFIX, random)
}

fn encode_with(prefix: &str, random: &[u8; RANDOM_BYTES]) -> String {
    let mut n = *random;
    let mut secret = [0u8; SECRET_LEN];
    for slot in secret.iter_mut().rev() {
        let mut rem = 0u32;
        for byte in n.iter_mut() {
            let acc = (rem << 8) | u32::from(*byte);
            *byte = (acc / 62) as u8;
            rem = acc % 62;
        }
        *slot = ALPHABET[rem as usize];
    }
    let mut token = String::with_capacity(TOKEN_LEN);
    token.push_str(prefix);
    token.extend(secret.iter().map(|&b| char::from(b)));
    token.push_str(&checksum(&secret));
    token
}

/// Checks prefix, length, alphabet and checksum (in that order). Nothing about whether the token exists.
pub fn check(token: &str) -> Result<(), TokenFormatError> {
    check_with(PREFIX, token)
}

/// [`check`] for an invite token (`qinv_`).
pub fn check_invite(token: &str) -> Result<(), TokenFormatError> {
    check_with(INVITE_PREFIX, token)
}

fn check_with(prefix: &str, token: &str) -> Result<(), TokenFormatError> {
    let rest = token.strip_prefix(prefix).ok_or(TokenFormatError::Prefix)?;
    if token.len() != TOKEN_LEN {
        return Err(TokenFormatError::Length);
    }
    if !rest.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(TokenFormatError::Alphabet);
    }
    let (secret, sum) = rest.split_at(SECRET_LEN);
    if checksum(secret.as_bytes()) != sum {
        return Err(TokenFormatError::Checksum);
    }
    Ok(())
}

/// The first 12 characters, stored as `api_tokens.prefix` and shown in lists. For a string shorter than that (never a
/// valid token), the whole string.
pub fn display_prefix(token: &str) -> &str {
    token.get(..DISPLAY_PREFIX_LEN).unwrap_or(token)
}

/// CRC32 of the secret characters, as 6 base62 digits, most significant first (62⁶ > 2³²).
fn checksum(secret: &[u8]) -> String {
    let mut v = crc32fast::hash(secret);
    let mut out = [b'0'; CHECKSUM_LEN];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(v % 62) as usize];
        v /= 62;
    }
    out.iter().map(|&b| char::from(b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seed: u8) -> [u8; RANDOM_BYTES] {
        let mut b = [0u8; RANDOM_BYTES];
        for (i, x) in b.iter_mut().enumerate() {
            *x = seed
                .wrapping_mul(31)
                .wrapping_add((i as u8).wrapping_mul(97))
                ^ 0x5a;
        }
        b
    }

    #[test]
    fn encode_has_the_d42_format() {
        for seed in 0..=255u8 {
            let t = encode(&sample(seed));
            assert_eq!(t.len(), TOKEN_LEN);
            assert_eq!(TOKEN_LEN, 51);
            assert!(t.starts_with("qpat_"));
            assert!(t[5..].bytes().all(|b| b.is_ascii_alphanumeric()), "{t}");
            assert_eq!(check(&t), Ok(()), "{t}");
            assert_eq!(display_prefix(&t).len(), 12);
            assert_eq!(display_prefix(&t), &t[..12]);
        }
    }

    #[test]
    fn encode_is_deterministic_and_spreads() {
        assert_eq!(encode(&sample(1)), encode(&sample(1)));
        assert_ne!(encode(&sample(1)), encode(&sample(2)));
        // All-zero bytes give the all-'0' secret; its checksum is the CRC32 of forty '0' characters.
        let zero = encode(&[0; RANDOM_BYTES]);
        assert_eq!(&zero[5..45], "0".repeat(40));
        assert_eq!(zero[45..], checksum("0".repeat(40).as_bytes()));
        // The highest byte matters: the secret is the whole number, not just its low bits.
        let mut hi = [0u8; RANDOM_BYTES];
        hi[0] = 1;
        assert_ne!(encode(&hi)[5..45], zero[5..45]);
        // Every alphabet character shows up across a few tokens.
        let all: String = (0..64u8)
            .map(|s| encode(&sample(s))[5..45].to_owned())
            .collect();
        assert!(ALPHABET.iter().all(|c| all.as_bytes().contains(c)));
    }

    #[test]
    fn checksum_is_crc32_in_base62() {
        assert_eq!(checksum(b""), "000000"); // CRC32("") = 0
        let v = crc32fast::hash(b"abc");
        let digits = checksum(b"abc");
        let back = digits.bytes().fold(0u64, |acc, c| {
            acc * 62 + ALPHABET.iter().position(|&a| a == c).unwrap() as u64
        });
        assert_eq!(back, u64::from(v));
    }

    #[test]
    fn check_rejects_malformed_tokens() {
        let t = encode(&sample(7));
        assert_eq!(
            check(&t.replacen("qpat_", "qpak_", 1)),
            Err(TokenFormatError::Prefix)
        );
        assert_eq!(check(&t[5..]), Err(TokenFormatError::Prefix));
        assert_eq!(check(""), Err(TokenFormatError::Prefix));
        assert_eq!(check(&t[..50]), Err(TokenFormatError::Length));
        assert_eq!(check(&format!("{t}0")), Err(TokenFormatError::Length));
        let mut bad = t.clone().into_bytes();
        bad[20] = b'-';
        assert_eq!(
            check(std::str::from_utf8(&bad).unwrap()),
            Err(TokenFormatError::Alphabet)
        );
        // A non-ASCII character of the same byte length is still not base62.
        let mut uni = t[..49].to_owned();
        uni.push('é');
        assert_eq!(check(&uni), Err(TokenFormatError::Alphabet));
        // One changed secret character or checksum character breaks the checksum.
        for pos in [5, 25, 44, 45, 50] {
            let mut b = t.clone().into_bytes();
            b[pos] = if b[pos] == b'A' { b'B' } else { b'A' };
            assert_eq!(
                check(std::str::from_utf8(&b).unwrap()),
                Err(TokenFormatError::Checksum),
                "{pos}"
            );
        }
        // Swapping two different neighbouring characters is caught too.
        let mut swapped = t.clone().into_bytes();
        let i = (5..44).find(|&i| swapped[i] != swapped[i + 1]).unwrap();
        swapped.swap(i, i + 1);
        assert_eq!(
            check(std::str::from_utf8(&swapped).unwrap()),
            Err(TokenFormatError::Checksum)
        );
    }

    #[test]
    fn invite_tokens_have_their_own_prefix() {
        let t = encode_invite(&sample(9));
        assert_eq!(t.len(), TOKEN_LEN);
        assert!(t.starts_with("qinv_"));
        assert_eq!(&t[5..], &encode(&sample(9))[5..]);
        assert_eq!(check_invite(&t), Ok(()));
        // An invite is never an API token, and the other way round.
        assert_eq!(check(&t), Err(TokenFormatError::Prefix));
        assert_eq!(
            check_invite(&encode(&sample(9))),
            Err(TokenFormatError::Prefix)
        );
        let mut b = t.into_bytes();
        b[30] = if b[30] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            check_invite(std::str::from_utf8(&b).unwrap()),
            Err(TokenFormatError::Checksum)
        );
    }

    #[test]
    fn display_prefix_of_short_input() {
        assert_eq!(display_prefix("qpat_"), "qpat_");
    }
}
