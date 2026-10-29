//! Connection pools: one writer, four readers, the maintenance connection, and the PRAGMAs (owner: W1).
//!
//! D2: one writer pool of size 1 whose closures run in a `BEGIN IMMEDIATE` transaction, a reader pool (default 4) with
//! `query_only=ON`, and an on-demand maintenance connection with the same PRAGMAs but writable, used only for
//! `VACUUM INTO` (backup, export) and migrations, never for app writes. Closures run on tokio's blocking pool
//! (deadpool-sqlite `interact`), so `write` and `read` are `async` and may be awaited from handlers directly.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deadpool_sqlite::{Config, Hook, HookError, Pool, Runtime};
use wikanvas_core::api::ErrorCode;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::error::{Error, Result};

/// Options of [`Db::open`].
#[derive(Clone, Debug)]
pub struct DbOptions {
    /// `synchronous=FULL` instead of `NORMAL` (`WIKANVAS_SYNC=FULL`).
    pub synchronous_full: bool,
    /// Size of the reader pool.
    pub readers: usize,
}

impl Default for DbOptions {
    fn default() -> Self {
        DbOptions {
            synchronous_full: false,
            readers: 4,
        }
    }
}

/// The database: a writer pool of one, a reader pool, and the path for maintenance connections. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    options: DbOptions,
    writer: Pool,
    readers: Pool,
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.inner.path)
            .field("options", &self.inner.options)
            .finish_non_exhaustive()
    }
}

/// Which connection the PRAGMAs are for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Writer,
    Reader,
    Maintenance,
}

/// The §5 PRAGMAs, in an order that works on a reader: `optimize` may write statistics, so `query_only` comes last.
fn apply_pragmas(conn: &Connection, role: Role, synchronous_full: bool) -> rusqlite::Result<()> {
    let synchronous = if synchronous_full { "FULL" } else { "NORMAL" };
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = {synchronous};
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -8000;
         PRAGMA journal_size_limit = 67108864;
         PRAGMA trusted_schema = OFF;
         PRAGMA optimize = 0x10002;"
    ))?;
    if role == Role::Reader {
        conn.execute_batch("PRAGMA query_only = ON;")?;
    }
    Ok(())
}

fn pool(path: &Path, size: usize, role: Role, synchronous_full: bool) -> Result<Pool> {
    let config = Config::new(path);
    let builder = config
        .builder(Runtime::Tokio1)
        .map_err(|e| Error::internal("database pool configuration", e))?;
    builder
        .max_size(size)
        .post_create(Hook::async_fn(move |conn, _metrics| {
            Box::pin(async move {
                conn.interact(move |c| apply_pragmas(c, role, synchronous_full))
                    .await
                    .map_err(|e| HookError::Message(e.to_string().into()))?
                    .map_err(HookError::Backend)
            })
        }))
        .build()
        .map_err(|e| Error::internal("database pool", e))
}

impl Db {
    /// Opens the database file at `path` (created if missing). Connections are made lazily, on first use, inside the
    /// tokio runtime; run [`crate::migrate::run`] before serving.
    pub fn open(path: impl AsRef<Path>, options: DbOptions) -> Result<Db> {
        let path = path.as_ref().to_path_buf();
        let writer = pool(&path, 1, Role::Writer, options.synchronous_full)?;
        let readers = pool(
            &path,
            options.readers.max(1),
            Role::Reader,
            options.synchronous_full,
        )?;
        Ok(Db {
            inner: Arc::new(Inner {
                path,
                options,
                writer,
                readers,
            }),
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Runs `f` in one `BEGIN IMMEDIATE` transaction on the single writer; commits when `f` returns `Ok`, rolls back
    /// otherwise. A panic inside `f` is resumed on the caller.
    pub async fn write<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.with_writer(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let out = f(&tx)?;
            tx.commit()?;
            Ok(out)
        })
        .await
    }

    /// Runs `f` on a reader (`query_only`) inside one deferred transaction, so every query in `f` sees the same
    /// snapshot.
    pub async fn read<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self
            .inner
            .readers
            .get()
            .await
            .map_err(|e| Error::internal("database reader", e))?;
        interact(&conn, move |conn| {
            let tx = conn.transaction()?;
            let out = f(&tx)?;
            tx.commit()?;
            Ok(out)
        })
        .await
    }

    /// Runs `f` with the writer connection itself, outside any transaction, while holding the writer: for work that
    /// commits and then continues under the writer (blob GC unlinks files after its commit, D26). Prefer
    /// [`Db::write`].
    pub async fn with_writer<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self
            .inner
            .writer
            .get()
            .await
            .map_err(|e| Error::internal("database writer", e))?;
        interact(&conn, f).await
    }

    /// A new connection with the §5 PRAGMAs and no `query_only`, for `VACUUM INTO` and migrations only (D2). It is
    /// not pooled; drop it when done.
    pub fn maintenance(&self) -> Result<Connection> {
        let conn = Connection::open(&self.inner.path)?;
        apply_pragmas(
            &conn,
            Role::Maintenance,
            self.inner.options.synchronous_full,
        )?;
        Ok(conn)
    }
}

async fn interact<T, F>(conn: &deadpool_sqlite::Object, f: F) -> Result<T>
where
    F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    match conn.interact(f).await {
        Ok(out) => out,
        Err(deadpool_sqlite::InteractError::Panic(payload)) => std::panic::resume_unwind(payload),
        Err(e) => Err(Error::new(
            ErrorCode::Internal,
            format!("database task: {e}"),
        )),
    }
}
