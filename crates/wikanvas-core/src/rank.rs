//! Fractional-index base62 sort keys, generated on the server (D6; owner: W1).
//!
//! A port of the CC0 `fractional-indexing` algorithm (David Greenspan's "Implementing Fractional Indexing", in the
//! version of rocicorp/fractional-indexing: `generateKeyBetween` and `generateNKeysBetween`) over the base62 alphabet
//! `0-9A-Za-z`. A key is an "integer part" (a head letter that encodes its length, then digits) plus an optional
//! fraction without trailing zeros; keys compare as plain byte strings, which is how SQLite's BINARY collation orders
//! `sort_key`. Clients never compute keys: they send `{after_id, before_id}` and the store calls [`key_between`].
//!
//! Inserting repeatedly at the same spot lengthens keys by about one character per six insertions. When a key grows
//! past [`MAX_KEY_LEN`] the store rewrites the whole sibling group with [`rebalance`] (§5 "Sort keys").

use crate::ids::BASE62;

/// Keys longer than this make the store rebalance their sibling group (§5).
pub const MAX_KEY_LEN: usize = 64;

/// The smallest integer part; no key may equal it (nothing could sort before it).
const SMALLEST_INT: &str = "A00000000000000000000000000";

/// Why a key could not be made.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RankError {
    /// A stored key is not a valid key (bad head, bad digit, trailing zero, too short).
    #[error("invalid sort key {0:?}")]
    Invalid(String),
    /// The lower bound is not below the upper bound: the sibling group needs a rebalance.
    #[error("sort keys {0:?} >= {1:?}: the sibling group needs a rebalance")]
    OutOfOrder(String, String),
    /// The integer range is used up (more than 10^47 keys appended at one end).
    #[error("the sort key range is exhausted")]
    Exhausted,
}

/// Whether a key is long enough that its sibling group should be rebalanced.
pub fn needs_rebalance(key: &str) -> bool {
    key.len() > MAX_KEY_LEN
}

/// A key strictly between `a` and `b` (`None`: unbounded on that side). `a < b` is required when both are given.
pub fn key_between(a: Option<&str>, b: Option<&str>) -> Result<String, RankError> {
    if let Some(a) = a {
        validate(a)?;
    }
    if let Some(b) = b {
        validate(b)?;
    }
    match (a, b) {
        (Some(a), Some(b)) if a >= b => Err(RankError::OutOfOrder(a.to_owned(), b.to_owned())),
        (None, None) => Ok(format!("a{}", char::from(BASE62[0]))),
        (None, Some(b)) => {
            let ib = int_part(b)?;
            let fb = &b.as_bytes()[ib.len()..];
            if ib == SMALLEST_INT {
                return with_mid(ib, &[], Some(fb));
            }
            if ib.len() < b.len() {
                return Ok(ib.to_owned());
            }
            decrement(ib)?.ok_or(RankError::Exhausted)
        }
        (Some(a), None) => {
            let ia = int_part(a)?;
            let fa = &a.as_bytes()[ia.len()..];
            match increment(ia)? {
                Some(i) => Ok(i),
                None => with_mid(ia, fa, None),
            }
        }
        (Some(a), Some(b)) => {
            let ia = int_part(a)?;
            let fa = &a.as_bytes()[ia.len()..];
            let ib = int_part(b)?;
            let fb = &b.as_bytes()[ib.len()..];
            if ia == ib {
                return with_mid(ia, fa, Some(fb));
            }
            let i = increment(ia)?.ok_or(RankError::Exhausted)?;
            if i.as_str() < b {
                return Ok(i);
            }
            with_mid(ia, fa, None)
        }
    }
}

/// `n` keys in increasing order, all strictly between `a` and `b` (`generateNKeysBetween`).
pub fn n_keys_between(
    a: Option<&str>,
    b: Option<&str>,
    n: usize,
) -> Result<Vec<String>, RankError> {
    match n {
        0 => Ok(Vec::new()),
        1 => Ok(vec![key_between(a, b)?]),
        _ => match (a, b) {
            (_, None) => {
                let mut out = Vec::with_capacity(n);
                let mut c = key_between(a, None)?;
                for _ in 1..n {
                    let next = key_between(Some(&c), None)?;
                    out.push(std::mem::replace(&mut c, next));
                }
                out.push(c);
                Ok(out)
            }
            (None, Some(b)) => {
                let mut out = Vec::with_capacity(n);
                let mut c = key_between(None, Some(b))?;
                for _ in 1..n {
                    let next = key_between(None, Some(&c))?;
                    out.push(std::mem::replace(&mut c, next));
                }
                out.push(c);
                out.reverse();
                Ok(out)
            }
            (Some(a), Some(b)) => {
                let mid = n / 2;
                let c = key_between(Some(a), Some(b))?;
                let mut out = n_keys_between(Some(a), Some(&c), mid)?;
                out.push(c.clone());
                out.extend(n_keys_between(Some(&c), Some(b), n - mid - 1)?);
                Ok(out)
            }
        },
    }
}

/// `n` fresh, short, increasing keys for a whole group (the rebalance of §5): `a0`, `a1`, … `az`, `b00`, …
pub fn rebalance(n: usize) -> Vec<String> {
    n_keys_between(None, None, n).expect("keys from an empty range never fail")
}

/// Checks that `key` is a valid key.
pub fn validate(key: &str) -> Result<(), RankError> {
    let invalid = || RankError::Invalid(key.to_owned());
    if key == SMALLEST_INT {
        return Err(invalid());
    }
    let i = int_part(key)?;
    let f = &key.as_bytes()[i.len()..];
    if f.last() == Some(&BASE62[0]) {
        return Err(invalid());
    }
    if key.as_bytes()[1..].iter().any(|&c| digit(c).is_none()) {
        return Err(invalid());
    }
    Ok(())
}

fn digit(c: u8) -> Option<usize> {
    BASE62.iter().position(|&d| d == c)
}

fn digit_of(key: &[u8], c: u8) -> Result<usize, RankError> {
    digit(c).ok_or_else(|| RankError::Invalid(String::from_utf8_lossy(key).into_owned()))
}

fn int_len(key: &str) -> Result<usize, RankError> {
    match key.as_bytes().first() {
        Some(&h @ b'a'..=b'z') => Ok(usize::from(h - b'a') + 2),
        Some(&h @ b'A'..=b'Z') => Ok(usize::from(b'Z' - h) + 2),
        _ => Err(RankError::Invalid(key.to_owned())),
    }
}

fn int_part(key: &str) -> Result<&str, RankError> {
    let len = int_len(key)?;
    key.get(..len)
        .ok_or_else(|| RankError::Invalid(key.to_owned()))
}

/// A digit string strictly between the fractions `a` and `b` (`b == None`: unbounded above).
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Result<Vec<u8>, RankError> {
    if let Some(b) = b {
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(BASE62[0]) == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            let rest_a = a.get(n..).unwrap_or(&[]);
            out.extend(midpoint(rest_a, Some(&b[n..]))?);
            return Ok(out);
        }
    }
    let da = match a.first() {
        Some(&c) => digit_of(a, c)?,
        None => 0,
    };
    let db = match b {
        Some(b) => {
            let &c = b.first().ok_or_else(|| {
                RankError::OutOfOrder(String::from_utf8_lossy(a).into_owned(), String::new())
            })?;
            digit_of(b, c)?
        }
        None => BASE62.len(),
    };
    if db < da {
        return Err(RankError::OutOfOrder(
            String::from_utf8_lossy(a).into_owned(),
            String::from_utf8_lossy(b.unwrap_or(&[])).into_owned(),
        ));
    }
    if db - da > 1 {
        Ok(vec![BASE62[(da + db).div_ceil(2)]])
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        Ok(vec![b[0]])
    } else {
        let mut out = vec![BASE62[da]];
        out.extend(midpoint(a.get(1..).unwrap_or(&[]), None)?);
        Ok(out)
    }
}

fn increment(x: &str) -> Result<Option<String>, RankError> {
    let b = x.as_bytes();
    let head = b[0];
    let mut digs = b[1..].to_vec();
    let mut carry = true;
    for d in digs.iter_mut().rev() {
        if !carry {
            break;
        }
        let v = digit_of(b, *d)? + 1;
        if v == BASE62.len() {
            *d = BASE62[0];
        } else {
            *d = BASE62[v];
            carry = false;
        }
    }
    if carry {
        if head == b'Z' {
            return Ok(Some(format!("a{}", char::from(BASE62[0]))));
        }
        if head == b'z' {
            return Ok(None);
        }
        let h = head + 1;
        if h > b'a' {
            digs.push(BASE62[0]);
        } else {
            digs.pop();
        }
        return Ok(Some(ascii(std::iter::once(h).chain(digs))));
    }
    Ok(Some(ascii(std::iter::once(head).chain(digs))))
}

fn decrement(x: &str) -> Result<Option<String>, RankError> {
    let b = x.as_bytes();
    let head = b[0];
    let mut digs = b[1..].to_vec();
    let mut borrow = true;
    for d in digs.iter_mut().rev() {
        if !borrow {
            break;
        }
        let v = digit_of(b, *d)?;
        if v == 0 {
            *d = BASE62[BASE62.len() - 1];
        } else {
            *d = BASE62[v - 1];
            borrow = false;
        }
    }
    if borrow {
        if head == b'a' {
            return Ok(Some(format!("Z{}", char::from(BASE62[BASE62.len() - 1]))));
        }
        if head == b'A' {
            return Ok(None);
        }
        let h = head - 1;
        if h < b'Z' {
            digs.push(BASE62[BASE62.len() - 1]);
        } else {
            digs.pop();
        }
        return Ok(Some(ascii(std::iter::once(h).chain(digs))));
    }
    Ok(Some(ascii(std::iter::once(head).chain(digs))))
}

fn with_mid(int: &str, frac: &[u8], hi: Option<&[u8]>) -> Result<String, RankError> {
    let mut out = int.as_bytes().to_vec();
    out.extend(midpoint(frac, hi)?);
    Ok(ascii(out))
}

fn ascii(bytes: impl IntoIterator<Item = u8>) -> String {
    String::from_utf8(bytes.into_iter().collect()).expect("sort keys are ASCII")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn kb(a: Option<&str>, b: Option<&str>) -> String {
        key_between(a, b).unwrap()
    }

    #[test]
    fn matches_fractional_indexing_vectors() {
        // The test vectors of rocicorp/fractional-indexing (CC0), base62.
        assert_eq!(kb(None, None), "a0");
        assert_eq!(kb(None, Some("a0")), "Zz");
        assert_eq!(kb(None, Some("Zz")), "Zy");
        assert_eq!(kb(Some("a0"), None), "a1");
        assert_eq!(kb(Some("a1"), None), "a2");
        assert_eq!(kb(Some("a0"), Some("a1")), "a0V");
        assert_eq!(kb(Some("a1"), Some("a2")), "a1V");
        assert_eq!(kb(Some("a0V"), Some("a1")), "a0l");
        assert_eq!(kb(Some("Zz"), Some("a0")), "ZzV");
        assert_eq!(kb(Some("Zz"), Some("a1")), "a0");
        assert_eq!(kb(None, Some("Y00")), "Xzzz");
        assert_eq!(kb(Some("bzz"), None), "c000");
        assert_eq!(kb(Some("a0"), Some("a0V")), "a0G");
        assert_eq!(kb(Some("a0"), Some("a0G")), "a08");
        assert_eq!(kb(Some("b125"), Some("b129")), "b127");
        assert_eq!(kb(Some("a0"), Some("a1V")), "a1");
        assert_eq!(kb(Some("Zz"), Some("a01")), "a0");
        assert_eq!(kb(None, Some("a0V")), "a0");
        assert_eq!(kb(None, Some("b999")), "b99");
        assert_eq!(
            kb(None, Some("A000000000000000000000000001")),
            "A000000000000000000000000000V"
        );
        assert_eq!(
            kb(Some("zzzzzzzzzzzzzzzzzzzzzzzzzzy"), None),
            "zzzzzzzzzzzzzzzzzzzzzzzzzzz"
        );
        assert_eq!(
            kb(Some("zzzzzzzzzzzzzzzzzzzzzzzzzzz"), None),
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzV"
        );
        // The invalid inputs of the same suite.
        for (a, b) in [
            (None, Some("A00000000000000000000000000")),
            (Some("a00"), None),
            (Some("a00"), Some("a1")),
            (Some("0"), Some("1")),
            (Some("a1"), Some("a0")),
        ] {
            assert!(key_between(a, b).is_err(), "{a:?} {b:?}");
        }
        assert!(matches!(
            key_between(Some("a1"), Some("a1")),
            Err(RankError::OutOfOrder(..))
        ));
        assert!(matches!(
            key_between(Some("a!"), None),
            Err(RankError::Invalid(_))
        ));
        // generateNKeysBetween.
        assert_eq!(
            n_keys_between(None, None, 5).unwrap(),
            ["a0", "a1", "a2", "a3", "a4"]
        );
        assert_eq!(
            n_keys_between(Some("a4"), None, 10).unwrap(),
            ["a5", "a6", "a7", "a8", "a9", "aA", "aB", "aC", "aD", "aE"]
        );
        assert_eq!(
            n_keys_between(None, Some("a0"), 5).unwrap(),
            ["Zv", "Zw", "Zx", "Zy", "Zz"]
        );
        let between = n_keys_between(Some("a0"), Some("a2"), 20).unwrap();
        assert_eq!(between.len(), 20);
        assert!(between.windows(2).all(|w| w[0] < w[1]));
        assert!(
            "a0" < between[0].as_str() && between[19].as_str() < "a2",
            "{between:?}"
        );
        assert!(
            between.contains(&"a1".to_owned()),
            "the first split is the plain midpoint"
        );
    }

    #[test]
    fn rebalance_gives_short_increasing_keys() {
        let keys = rebalance(5000);
        assert_eq!(keys.len(), 5000);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| k.len() <= 4 && validate(k).is_ok()));
        assert!(rebalance(0).is_empty());
    }

    #[test]
    fn repeated_inserts_at_one_spot_need_a_rebalance_eventually() {
        let (lo, mut hi) = ("a0".to_owned(), "a1".to_owned());
        let mut steps = 0;
        while !needs_rebalance(&hi) {
            hi = kb(Some(&lo), Some(&hi));
            steps += 1;
        }
        assert!(
            steps > 300,
            "{steps} inserts before a key passed {MAX_KEY_LEN} characters"
        );
    }

    fn insert_positions() -> impl Strategy<Value = Vec<(prop::sample::Index, bool)>> {
        prop::collection::vec((any::<prop::sample::Index>(), any::<bool>()), 1..200)
    }

    proptest! {
        #[test]
        fn between_is_strictly_between(ops in insert_positions()) {
            // Random insertions (at random gaps, ends included), sometimes a run of several keys at once.
            let mut keys: Vec<String> = Vec::new();
            for (at, many) in ops {
                let i = at.index(keys.len() + 1);
                let lo = i.checked_sub(1).map(|j| keys[j].clone());
                let hi = keys.get(i).cloned();
                let new = if many {
                    n_keys_between(lo.as_deref(), hi.as_deref(), 3).unwrap()
                } else {
                    vec![key_between(lo.as_deref(), hi.as_deref()).unwrap()]
                };
                for k in &new {
                    prop_assert!(validate(k).is_ok(), "{}", k);
                    prop_assert!(lo.as_deref().is_none_or(|lo| lo < k.as_str()), "{:?} < {}", lo, k);
                    prop_assert!(hi.as_deref().is_none_or(|hi| k.as_str() < hi), "{} < {:?}", k, hi);
                }
                prop_assert!(new.windows(2).all(|w| w[0] < w[1]));
                keys.splice(i..i, new);
            }
            prop_assert!(keys.windows(2).all(|w| w[0] < w[1]), "{:?}", keys);
        }
    }
}
