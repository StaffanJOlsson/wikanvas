//! Embedded migrations (migrations/NNNN-name/up.sql), run after the pre-migration backup (owner: W1).
//!
//! D3: rusqlite_migration `from_directory` over the `migrations/` directory embedded with `include_dir`; the schema
//! version is SQLite's `user_version`. §12: when migrations are pending on a database that already has a schema,
//! [`run`] first calls [`crate::backup::pre_migration`]; it then migrates in one transaction (on the maintenance
//! connection, before the server takes requests) and runs `PRAGMA foreign_key_check`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use include_dir::{Dir, include_dir};
use quire_core::api::ErrorCode;
use rusqlite_migration::Migrations;

use crate::db::Db;
use crate::error::{Error, Result};

static MIGRATIONS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

static MIGRATIONS: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::from_directory(&MIGRATIONS_DIR).expect("migrations/ is embedded at build time")
});

/// The embedded migrations (for `Migrations::validate` in tests).
pub fn migrations() -> &'static Migrations<'static> {
    &MIGRATIONS
}

/// The schema version the embedded migrations lead to.
pub fn latest() -> u32 {
    MIGRATIONS_DIR.dirs().count() as u32
}

/// What [`run`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migrated {
    /// `user_version` before (0: a new database).
    pub from: u32,
    /// `user_version` after.
    pub to: u32,
    /// The pre-migration backup, when one was made.
    pub backup: Option<PathBuf>,
}

/// Brings the database to the latest schema. A new database (version 0) is migrated without a backup; an existing one
/// with pending migrations is backed up first (`pre-migrate-v{from}-…`). A database newer than this build is refused
/// (`internal`, nothing changed).
pub fn run(db: &Db, data_dir: &Path, now: i64) -> Result<Migrated> {
    let migrations = migrations();
    let mut conn = db.maintenance()?;
    let from: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let pending = migrations.pending_migrations(&conn)?;
    if pending < 0 {
        return Err(Error::new(
            ErrorCode::Internal,
            format!(
                "the database has schema version {from}, newer than this server's {}; use a newer Quire",
                latest()
            ),
        ));
    }
    if pending == 0 {
        return Ok(Migrated {
            from,
            to: from,
            backup: None,
        });
    }
    let backup = if from > 0 {
        Some(crate::backup::pre_migration(db, data_dir, from, now)?)
    } else {
        None
    };
    migrations.to_latest(&mut conn)?;
    let violations: i64 =
        conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if violations > 0 {
        return Err(Error::new(
            ErrorCode::Internal,
            format!("foreign_key_check found {violations} violations after migrating"),
        ));
    }
    let to: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(Migrated { from, to, backup })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_validate() {
        assert_eq!(migrations().validate(), Ok(()));
        assert_eq!(latest(), 1);
    }
}
