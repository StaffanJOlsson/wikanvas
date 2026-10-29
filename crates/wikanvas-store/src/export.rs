//! The workspace export tar (owner: W2).
//!
//! D34, §12: `GET /api/v1/export` streams a tar of `manifest.json`, a credential-free copy of the database as
//! `wikanvas.db`, and every blob file as `files/aa/bb/<sha256>`.
//! - [`prepare_copy`] runs `VACUUM INTO '<data_dir>/tmp/export-….db'` on the maintenance connection (never `/tmp`, a
//!   small tmpfs), then opens the copy on its own connection (not through the §5 PRAGMA path) with
//!   `journal_mode=DELETE` and `secure_delete=ON`, deletes every session and API token and clears every password
//!   hash (also the `sqlite_stat1` / `sqlite_stat4` rows of those tables: stat4 samples index keys, so it would keep
//!   token and session hashes), and closes it, so no `-wal` or `-journal` file keeps the deleted rows. Then a second
//!   `VACUUM INTO` writes the file that is exported from that copy's remaining rows, because unused page space of the
//!   first copy can still hold older copies of deleted rows; the first copy is deleted. The returned [`ExportCopy`]
//!   deletes the file when dropped.
//! - [`write_tar`] writes the tar into any `Write`, reading the copy and each file through 256 KiB buffers; the server
//!   hands it a writer that sends 256 KiB chunks through a 4-chunk channel (D25: at most about 1 MiB in flight).

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use wikanvas_core::api::{ErrorCode, Timestamp};

use crate::backup::vacuum_into;
use crate::db::Db;
use crate::error::{Error, Result};

/// The read buffer for the database copy and each file (D25).
pub const CHUNK: usize = 256 * 1024;

/// The export's `manifest.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Manifest {
    /// `wikanvas-export`.
    pub format: &'static str,
    /// The layout version of the tar (1).
    pub format_version: u32,
    pub created_at: String,
    /// The server that wrote it (`wikanvas-server` version).
    pub server_version: String,
    /// `wikanvas_api::API_VERSION`.
    pub api_version: &'static str,
    /// The database schema version (`user_version`).
    pub schema_version: u32,
    /// What was removed from the database copy.
    pub without: [&'static str; 3],
}

impl Manifest {
    /// The manifest of an export written now by `server_version`.
    pub fn new(server_version: &str, schema_version: u32, now: i64) -> Manifest {
        Manifest {
            format: "wikanvas-export",
            format_version: 1,
            created_at: Timestamp(now).to_rfc3339(),
            server_version: server_version.to_owned(),
            api_version: wikanvas_core::api::API_VERSION,
            schema_version,
            without: ["sessions", "api_tokens", "password_hashes"],
        }
    }
}

/// The credential-free database copy; the file is deleted when this drops.
#[derive(Debug)]
pub struct ExportCopy {
    path: PathBuf,
    schema_version: u32,
}

impl ExportCopy {
    /// The copy's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its `user_version`.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

impl Drop for ExportCopy {
    fn drop(&mut self) {
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut p = self.path.clone().into_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(p));
        }
    }
}

/// Makes the credential-free copy (see the module docs) under `tmp_dir`. Blocking: run it in `spawn_blocking`.
pub fn prepare_copy(db: &Db, tmp_dir: &Path) -> Result<ExportCopy> {
    std::fs::create_dir_all(tmp_dir)?;
    let path = tmp_dir.join(format!("export-{}.db", crate::shared::random_base62(12)?));
    let maintenance = db.maintenance()?;
    // From here the guard owns the file: any error below deletes it.
    let copy = ExportCopy {
        path,
        schema_version: 0,
    };
    vacuum_into(&maintenance, &copy.path)?;
    drop(maintenance);
    let conn = Connection::open_with_flags(
        &copy.path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mode: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(Error::new(
            ErrorCode::Internal,
            format!("the export copy stayed in journal mode {mode}"),
        ));
    }
    conn.execute_batch(
        "PRAGMA secure_delete = ON;
         PRAGMA foreign_keys = OFF;
         BEGIN;
         DELETE FROM sessions;
         DELETE FROM api_token_scopes;
         DELETE FROM api_tokens;
         UPDATE users SET password_hash = NULL;",
    )?;
    // `PRAGMA optimize` (ANALYZE) keeps sample index keys in sqlite_stat4, token and session hashes included.
    for stat in ["sqlite_stat1", "sqlite_stat4"] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [stat],
            |r| r.get(0),
        )?;
        if exists {
            conn.execute(
                &format!("DELETE FROM {stat} WHERE tbl IN ('sessions', 'api_tokens', 'api_token_scopes', 'users')"),
                [],
            )?;
        }
    }
    conn.execute_batch("COMMIT;")?;
    // secure_delete zeroes the deleted cells, but older copies of them can stay in unused page space: the first
    // VACUUM INTO rebalances b-trees without clearing it (stat4 samples of token hashes survived there). A second
    // VACUUM INTO writes a file from the remaining rows only; the first copy is deleted when it drops.
    let clean = ExportCopy {
        path: tmp_dir.join(format!("export-{}.db", crate::shared::random_base62(12)?)),
        schema_version: conn.query_row("PRAGMA user_version", [], |r| r.get(0))?,
    };
    vacuum_into(&conn, &clean.path)?;
    conn.close().map_err(|(_, e)| Error::from(e))?;
    drop(copy);
    let conn = Connection::open_with_flags(
        &clean.path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mode: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
    conn.close().map_err(|(_, e)| Error::from(e))?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(Error::new(
            ErrorCode::Internal,
            format!("the export copy stayed in journal mode {mode}"),
        ));
    }
    Ok(clean)
}

/// Writes the tar: `manifest.json`, `wikanvas.db` (the copy), then `files/aa/bb/<sha256>` for every file under
/// `files_dir`. Blocking. Returns the writer after the end-of-archive marker.
pub fn write_tar<W: Write>(
    out: W,
    copy: &ExportCopy,
    files_dir: &Path,
    manifest: &Manifest,
) -> Result<W> {
    let mut tar = tar::Builder::new(out);
    tar.mode(tar::HeaderMode::Deterministic);
    let json = serde_json::to_vec_pretty(manifest)?;
    let mtime = manifest_mtime(manifest);
    append(
        &mut tar,
        "manifest.json",
        json.len() as u64,
        mtime,
        &json[..],
    )?;
    let db = File::open(copy.path())?;
    let len = db.metadata()?.len();
    append(
        &mut tar,
        "wikanvas.db",
        len,
        mtime,
        BufReader::with_capacity(CHUNK, db),
    )?;
    for (rel, path) in blob_files(files_dir)? {
        let file = match File::open(&path) {
            Ok(f) => f,
            // A blob deleted by the GC while the export ran is simply left out.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let len = file.metadata()?.len();
        append(
            &mut tar,
            &format!("files/{rel}"),
            len,
            mtime,
            BufReader::with_capacity(CHUNK, file),
        )?;
    }
    Ok(tar.into_inner()?)
}

fn manifest_mtime(manifest: &Manifest) -> u64 {
    Timestamp::parse_rfc3339(&manifest.created_at)
        .map(|t| (t.millis() / 1000).max(0) as u64)
        .unwrap_or(0)
}

fn append<W: Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    size: u64,
    mtime: u64,
    data: impl std::io::Read,
) -> Result<()> {
    let mut header = tar::Header::new_ustar();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_mtime(mtime);
    header.set_entry_type(tar::EntryType::Regular);
    tar.append_data(&mut header, path, data.take(size))?;
    Ok(())
}

/// Every blob file as (`aa/bb/<sha256>`, path), sorted.
fn blob_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let read = |dir: &Path| -> Result<Vec<(String, PathBuf)>> {
        match std::fs::read_dir(dir) {
            Ok(entries) => Ok(entries
                .flatten()
                .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    };
    for (a, a_path) in read(root)? {
        if !a_path.is_dir() {
            continue;
        }
        for (b, b_path) in read(&a_path)? {
            if !b_path.is_dir() {
                continue;
            }
            for (name, path) in read(&b_path)? {
                if crate::blobs::Sha256::parse(&name).is_ok() && path.is_file() {
                    out.push((format!("{a}/{b}/{name}"), path));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}
