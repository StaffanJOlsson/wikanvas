//! Backups on the maintenance connection, including the pre-migration backup (owner: W2).
//!
//! W0b provides the pre-migration backup (§12): `VACUUM INTO '<data_dir>/backups/pre-migrate-v{N}-{ts}.db'` on the
//! maintenance connection (D2: a `query_only` reader would fail with SQLITE_READONLY), then a `quick_check` of the
//! copy. Scheduled and on-demand backups (`quire-YYYYMMDDTHHMMZ.db`, `QUIRE_BACKUP_KEEP`) are W2's.

use std::path::{Path, PathBuf};

use quire_core::api::{ErrorCode, Timestamp};
use rusqlite::{Connection, OpenFlags};

use crate::db::Db;
use crate::error::{Error, Result};

/// The backup directory under the data directory.
pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

/// `YYYYMMDDTHHMMZ` (UTC) for backup file names.
pub fn file_timestamp(now: i64) -> String {
    let iso = Timestamp(now).to_rfc3339(); // YYYY-MM-DDTHH:MM:SS.mmmZ
    format!(
        "{}{}{}T{}{}Z",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16]
    )
}

/// Copies the database before migrating from schema version `version`: `VACUUM INTO
/// '<data_dir>/backups/pre-migrate-v{version}-{YYYYMMDDTHHMMZ}.db'` on [`Db::maintenance`], then `quick_check` on the
/// copy. A name that is taken gets a `-2`, `-3`, … suffix. Returns the path of the copy. Pre-migration backups are not
/// counted against `QUIRE_BACKUP_KEEP`.
pub fn pre_migration(db: &Db, data_dir: &Path, version: u32, now: i64) -> Result<PathBuf> {
    let dir = backups_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let stem = format!("pre-migrate-v{version}-{}", file_timestamp(now));
    let mut path = dir.join(format!("{stem}.db"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.db"));
        n += 1;
    }
    let conn = db.maintenance()?;
    vacuum_into(&conn, &path)?;
    drop(conn);
    quick_check(&path)?;
    Ok(path)
}

/// `VACUUM INTO path` on `conn` (which must not be `query_only`).
pub fn vacuum_into(conn: &Connection, path: &Path) -> Result<()> {
    let target = path.to_str().ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            format!("backup path {} is not UTF-8", path.display()),
        )
    })?;
    conn.execute("VACUUM INTO ?1", [target])?;
    Ok(())
}

/// `PRAGMA quick_check` on the database file at `path`, opened read-only; anything but `ok` is `internal`.
pub fn quick_check(path: &Path) -> Result<()> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let result: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if result != "ok" {
        return Err(Error::new(
            ErrorCode::Internal,
            format!("quick_check of {} failed: {result}", path.display()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_timestamp_is_compact_utc() {
        assert_eq!(file_timestamp(1_761_643_800_123), "20251028T0930Z");
        assert_eq!(file_timestamp(0), "19700101T0000Z");
    }
}
