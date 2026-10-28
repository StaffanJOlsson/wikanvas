//! Shared shapes and enums used across the API: timestamps, kinds, roles, positions, users (owner: W1).
//!
//! Wire rules (ARCHITECTURE §7.1, D5): snake_case JSON fields; UUIDs as strings; times as RFC 3339 UTC strings
//! ([`Timestamp`]); response fields that have no value are sent as `null`, never omitted; optional request fields may
//! be omitted (and, where `null` means something, a `null` is kept apart from an omitted field: [`double_option`]).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

/// Names of the HTTP headers that are part of the API contract.
pub mod header {
    /// The API version (`quire_api::API_VERSION`) on every `/api/v1` response.
    pub const API_VERSION: &str = "Quire-Api-Version";
    /// The request id, also in every error body's `request_id`.
    pub const REQUEST_ID: &str = "X-Request-Id";
    /// Codes of the conversion warnings of a Markdown GET, comma-separated.
    pub const CONVERSION_WARNINGS: &str = "Quire-Conversion-Warnings";
    /// The board engine version of the writer of a board PUT (optional for API clients, D51).
    pub const ENGINE: &str = "X-Quire-Engine";
}

/// The error of a `FromStr` for one of the API's string enums.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownValue {
    /// The enum, e.g. `ItemKind`.
    pub kind: &'static str,
    /// The rejected input.
    pub value: String,
}

impl fmt::Display for UnknownValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {}: {:?}", self.kind, self.value)
    }
}

impl std::error::Error for UnknownValue {}

/// Declares a unit-only enum that travels as a snake_case string: serde, ts-rs and utoipa derives, `ALL`, `as_str`,
/// `Display` and `FromStr` (the store reads these values from TEXT columns).
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $wire:tt, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
            ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS, ::utoipa::ToSchema,
        )]
        #[ts(export)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $wire)] $variant, )+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The wire (and database) spelling.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $wire, )+
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::common::UnknownValue;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $wire => Ok($name::$variant), )+
                    _ => Err($crate::common::UnknownValue { kind: stringify!($name), value: s.to_owned() }),
                }
            }
        }
    };
}
pub(crate) use string_enum;

string_enum! {
    /// The kind of an item (lookup table `item_kinds`).
    pub enum ItemKind {
        Doc = "doc",
        Board = "board",
    }
}

string_enum! {
    /// A user's role (lookup table `user_roles`). M1 has a single owner.
    pub enum Role {
        Owner = "owner",
        Admin = "admin",
        Member = "member",
        Viewer = "viewer",
    }
}

string_enum! {
    /// A user's colour theme.
    pub enum Theme {
        Light = "light",
        Dark = "dark",
        System = "system",
    }
}

string_enum! {
    /// How a collection orders its items in the tree. Non-manual orders are computed at read time.
    pub enum ChildOrder {
        Manual = "manual",
        Title = "title",
        Updated = "updated",
        Created = "created",
    }
}

/// A user, as every response shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub theme: Theme,
}

/// A position among siblings (or collections, or stars). The server computes the sort key (D6). With both ids
/// omitted (or `null`) the row goes last.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Position {
    /// Place the row right after this sibling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<Uuid>,
    /// Place the row right before this sibling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<Uuid>,
}

/// A point in time. Unix milliseconds in Rust and SQL (D5); on the wire an RFC 3339 UTC string with millisecond
/// precision, e.g. `2026-10-28T09:30:00.000Z`. Parsing accepts any RFC 3339 offset and fraction (truncated to
/// milliseconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, TS, ToSchema)]
#[ts(export, type = "string")]
#[schema(value_type = String, format = DateTime)]
pub struct Timestamp(pub i64);

/// An RFC 3339 string that [`Timestamp`] cannot parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimestampError(pub String);

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an RFC 3339 timestamp: {:?}", self.0)
    }
}

impl std::error::Error for TimestampError {}

const MS_PER_DAY: i64 = 86_400_000;
/// 0000-01-01T00:00:00Z and 9999-12-31T23:59:59.999Z: the range RFC 3339 can spell.
const MIN_MS: i64 = -62_167_219_200_000;
const MAX_MS: i64 = 253_402_300_799_999;

impl Timestamp {
    /// From unix milliseconds.
    pub const fn from_millis(ms: i64) -> Self {
        Timestamp(ms)
    }

    /// Unix milliseconds.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// `YYYY-MM-DDTHH:MM:SS.mmmZ`. Values outside years 0000–9999 are clamped to that range.
    pub fn to_rfc3339(self) -> String {
        let ms = self.0.clamp(MIN_MS, MAX_MS);
        let days = ms.div_euclid(MS_PER_DAY);
        let rem = ms.rem_euclid(MS_PER_DAY);
        let (y, m, d) = civil_from_days(days);
        let (h, mi, s, milli) = (
            rem / 3_600_000,
            rem / 60_000 % 60,
            rem / 1000 % 60,
            rem % 1000,
        );
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
    }

    /// Parses `YYYY-MM-DD(T|t| )HH:MM:SS[.fraction](Z|z|±HH:MM)`.
    pub fn parse_rfc3339(s: &str) -> Result<Self, TimestampError> {
        parse_rfc3339(s.as_bytes()).ok_or_else(|| TimestampError(s.to_owned()))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl FromStr for Timestamp {
    type Err = TimestampError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Timestamp::parse_rfc3339(s)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Timestamp;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 timestamp")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Timestamp, E> {
                Timestamp::parse_rfc3339(v).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (H. Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// (year, month, day) → days since 1970-01-01 (H. Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

fn parse_rfc3339(b: &[u8]) -> Option<Timestamp> {
    fn num(b: &[u8]) -> Option<u32> {
        if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
            return None;
        }
        b.iter().try_fold(0u32, |acc, &c| {
            acc.checked_mul(10)?.checked_add(u32::from(c - b'0'))
        })
    }
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d) = (num(&b[0..4])?, num(&b[5..7])?, num(&b[8..10])?);
    let (h, mi, s) = (num(&b[11..13])?, num(&b[14..16])?, num(&b[17..19])?);
    if !(1..=12).contains(&mo)
        || d == 0
        || d > days_in_month(i64::from(y), mo)
        || h > 23
        || mi > 59
        || s > 60
    {
        return None;
    }
    let mut rest = &b[19..];
    let mut millis = 0u32;
    if let Some(frac) = rest.strip_prefix(b".") {
        let len = frac.iter().take_while(|c| c.is_ascii_digit()).count();
        if len == 0 {
            return None;
        }
        let mut digits = [b'0'; 3];
        for (slot, &c) in digits.iter_mut().zip(&frac[..len]) {
            *slot = c;
        }
        millis = num(&digits)?;
        rest = &frac[len..];
    }
    let offset_min: i64 = match rest {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), oh1, oh2, b':', om1, om2] => {
            let (oh, om) = (num(&[*oh1, *oh2])?, num(&[*om1, *om2])?);
            if oh > 23 || om > 59 {
                return None;
            }
            let total = i64::from(oh * 60 + om);
            if *sign == b'-' { -total } else { total }
        }
        _ => return None,
    };
    let days = days_from_civil(i64::from(y), mo, d);
    let secs = days * 86_400 + i64::from(h * 3600 + mi * 60 + s) - offset_min * 60;
    Some(Timestamp(secs * 1000 + i64::from(millis)))
}

/// `#[serde(with = "double_option")]` for PATCH fields where an omitted field ("leave unchanged") and `null`
/// ("clear it") differ: omitted → `None`, `null` → `Some(None)`, a value → `Some(Some(v))`. Use it together with
/// `#[serde(default, skip_serializing_if = "Option::is_none")]`.
pub mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_formats_utc_with_millis() {
        assert_eq!(Timestamp(0).to_rfc3339(), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            Timestamp(1_761_643_800_123).to_rfc3339(),
            "2025-10-28T09:30:00.123Z"
        );
        assert_eq!(Timestamp(-1).to_rfc3339(), "1969-12-31T23:59:59.999Z");
        assert_eq!(
            Timestamp(951_782_400_000).to_rfc3339(),
            "2000-02-29T00:00:00.000Z"
        );
    }

    #[test]
    fn timestamp_parses_offsets_and_fractions() {
        let t = |s: &str| Timestamp::parse_rfc3339(s).map(Timestamp::millis);
        assert_eq!(t("2025-10-28T09:30:00.123Z"), Ok(1_761_643_800_123));
        assert_eq!(t("2025-10-28T10:30:00.123456+01:00"), Ok(1_761_643_800_123));
        assert_eq!(t("2025-10-28t08:30:00-01:00"), Ok(1_761_643_800_000));
        assert_eq!(t("2025-10-28 09:30:00.1z"), Ok(1_761_643_800_100));
        for bad in [
            "",
            "2025-10-28",
            "2025-13-01T00:00:00Z",
            "2025-02-29T00:00:00Z",
            "2025-10-28T09:30:00",
            "2025-10-28T09:30:00.Z",
            "2025-10-28T24:00:00Z",
            "2025-10-28T09:30:00+0100",
            "+2025-10-28T09:30:00Z",
        ] {
            assert!(t(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn timestamp_roundtrips_through_json() {
        for ms in [
            0,
            1,
            -1,
            1_761_643_800_123,
            MIN_MS,
            MAX_MS,
            4_102_444_800_000,
        ] {
            let json = serde_json::to_string(&Timestamp(ms)).unwrap();
            assert_eq!(
                serde_json::from_str::<Timestamp>(&json).unwrap(),
                Timestamp(ms),
                "{json}"
            );
        }
    }

    #[test]
    fn string_enums_roundtrip() {
        for k in ItemKind::ALL {
            assert_eq!(k.as_str().parse::<ItemKind>(), Ok(*k));
            assert_eq!(serde_json::to_string(k).unwrap(), format!("\"{k}\""));
        }
        assert_eq!(ChildOrder::ALL.len(), 4);
        assert!("Doc".parse::<ItemKind>().is_err());
    }

    #[test]
    fn double_option_keeps_null_apart_from_absent() {
        #[derive(Deserialize, Serialize, Default)]
        struct Patch {
            #[serde(
                default,
                skip_serializing_if = "Option::is_none",
                with = "double_option"
            )]
            icon: Option<Option<String>>,
        }
        assert_eq!(serde_json::from_str::<Patch>("{}").unwrap().icon, None);
        assert_eq!(
            serde_json::from_str::<Patch>(r#"{"icon":null}"#)
                .unwrap()
                .icon,
            Some(None)
        );
        assert_eq!(
            serde_json::from_str::<Patch>(r#"{"icon":"x"}"#)
                .unwrap()
                .icon,
            Some(Some("x".into()))
        );
        assert_eq!(
            serde_json::to_string(&Patch { icon: Some(None) }).unwrap(),
            r#"{"icon":null}"#
        );
        assert_eq!(serde_json::to_string(&Patch::default()).unwrap(), "{}");
    }
}
