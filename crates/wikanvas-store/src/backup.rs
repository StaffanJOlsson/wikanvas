//! Backups on the maintenance connection, including the pre-migration backup (owner: W2).
//!
//! §12: every backup is `VACUUM INTO` on the maintenance connection (D2: a `query_only` reader would fail with
//! SQLITE_READONLY), then a `quick_check` of the copy.
//! - [`pre_migration`]: `<data_dir>/backups/pre-migrate-v{N}-{ts}.db`, kept separately.
//! - [`create`]: scheduled and on-demand backups. The copy is written as `backups/.tmp-…`, checked, then renamed to
//!   `wikanvas-YYYYMMDDTHHMMZ.db`; afterwards only the newest `keep` `wikanvas-*.db` files remain ([`prune`]). It holds
//!   an exclusive lock on `backups/.lock` throughout, so the admin command and a running server never back up at the
//!   same time (429 `rate_limited` with `reason: busy` for the second).
//! - [`to_path`]: `wikanvas-server admin backup --out PATH`, not counted against `keep`.
//!
//! In one process the callers also keep backups single-flight, and they check the free space first (D33).

use std::path::{Path, PathBuf};

use wikanvas_core::api::{ErrorCode, Timestamp};
use rusqlite::{Connection, OpenFlags};

use crate::db::Db;
use crate::error::{Error, Result};

/// The backup directory under the data directory.
pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

/// The lock file of [`create`] in the backup directory.
pub const LOCK_FILE: &str = ".lock";

/// Takes the exclusive lock on `dir/.lock` without waiting; 429 `rate_limited` (`reason: busy`) while another
/// process (or another open of the file) holds it. The lock is released when the returned file is dropped.
pub fn lock(dir: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::new(
            ErrorCode::RateLimited,
            "another backup is running",
        )
        .with_details(&serde_json::json!({"reason": "busy"}))),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
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
/// counted against `WIKANVAS_BACKUP_KEEP`.
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

/// A scheduled or on-demand backup: `VACUUM INTO backups/.tmp-<random>.db`, `quick_check`, rename to
/// `backups/wikanvas-YYYYMMDDTHHMMZ.db` (`-2`, `-3`, … when the name is taken), then [`prune`] to the newest
/// `keep.max(1)` copies, all under [`lock`] (429 `busy` when another process is backing up). Returns the new file. A
/// failed check deletes the copy.
pub fn create(db: &Db, data_dir: &Path, keep: u32, now: i64) -> Result<PathBuf> {
    let dir = backups_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let _lock = lock(&dir)?;
    let tmp = dir.join(format!(".tmp-{}.db", crate::shared::random_base62(12)?));
    let result: Result<PathBuf> = (|| {
        let conn = db.maintenance()?;
        vacuum_into(&conn, &tmp)?;
        drop(conn);
        quick_check(&tmp)?;
        let stem = format!("wikanvas-{}", file_timestamp(now));
        let mut path = dir.join(format!("{stem}.db"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem}-{n}.db"));
            n += 1;
        }
        std::fs::rename(&tmp, &path)?;
        if let Ok(d) = std::fs::File::open(&dir) {
            let _ = d.sync_all();
        }
        Ok(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    let path = result?;
    prune(&dir, keep.max(1))?;
    Ok(path)
}

/// `wikanvas-server admin backup --out PATH`: `VACUUM INTO path` (which must not exist) and `quick_check`. Not counted
/// against `WIKANVAS_BACKUP_KEEP`.
pub fn to_path(db: &Db, path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::validation(
            "out",
            format!("{} exists; choose a new file", path.display()),
        ));
    }
    let conn = db.maintenance()?;
    vacuum_into(&conn, path)?;
    drop(conn);
    if let Err(e) = quick_check(path) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// The `wikanvas-*.db` backups in `dir`, oldest first (by name, which sorts by time).
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("wikanvas-") && n.ends_with(".db"))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    found.sort_by_key(|p| backup_key(p));
    Ok(found)
}

/// Sort key of a backup name: the timestamp, then the `-n` suffix as a number.
fn backup_key(path: &Path) -> (String, u32) {
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
    let rest = name.strip_prefix("wikanvas-").unwrap_or(name);
    match rest.split_once('-') {
        Some((ts, n)) => (ts.to_owned(), n.parse().unwrap_or(0)),
        None => (rest.to_owned(), 1),
    }
}

/// Deletes the oldest `wikanvas-*.db` files in `dir` until at most `keep` remain (pre-migration backups are never
/// touched); returns the deleted paths. A file that is already gone counts as deleted.
pub fn prune(dir: &Path, keep: u32) -> Result<Vec<PathBuf>> {
    let all = list(dir)?;
    let excess = all.len().saturating_sub(keep as usize);
    let mut removed = Vec::new();
    for path in all.into_iter().take(excess) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        removed.push(path);
    }
    Ok(removed)
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
