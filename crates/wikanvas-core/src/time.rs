//! Time helpers: unix milliseconds (D5; owner: W1).
//!
//! Every time in Wikanvas is an `i64` of unix milliseconds in Rust and SQL, and an RFC 3339 UTC string on the wire
//! (`wikanvas_api::Timestamp`). The retention and window constants of §5 and §7.2 live here so the store, the server
//! and the jobs agree on them.

use std::time::{SystemTime, UNIX_EPOCH};

pub const SECOND_MS: i64 = 1000;
pub const MINUTE_MS: i64 = 60 * SECOND_MS;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;
pub const WEEK_MS: i64 = 7 * DAY_MS;

/// Trash roots (items and collections) are purged this long after they were deleted (§5, §12).
pub const TRASH_RETENTION_MS: i64 = 30 * DAY_MS;

/// `?discard_empty=true` deletes only items created less than this long ago (§7.2).
pub const DISCARD_WINDOW_MS: i64 = DAY_MS;

/// Unix milliseconds now (0 before 1970, `i64::MAX` past year 292 million).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// When something deleted at `deleted_at` is purged: [`TRASH_RETENTION_MS`] later.
pub fn purge_at(deleted_at: i64) -> i64 {
    deleted_at.saturating_add(TRASH_RETENTION_MS)
}

/// The cut-off the purge job passes to `items::purge_expired`: everything deleted before it is purged.
pub fn purge_cutoff(now: i64) -> i64 {
    now.saturating_sub(TRASH_RETENTION_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_purge_dates() {
        assert_eq!(DAY_MS, 86_400_000);
        assert_eq!(WEEK_MS, 604_800_000);
        assert_eq!(purge_at(0), 30 * 86_400_000);
        assert_eq!(purge_cutoff(purge_at(5)), 5);
        assert_eq!(purge_at(i64::MAX), i64::MAX);
        let now = now_ms();
        assert!(now > 1_700_000_000_000, "{now}");
    }
}
