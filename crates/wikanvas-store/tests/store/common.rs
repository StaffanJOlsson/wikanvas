//! Shared test setup: a temp-file database with migrations applied, opened through Db (owner: W1).
//!
//! `TestDb::new()` makes a temp data directory, opens `wikanvas.db` in it through [`Db`] (WAL, the §5 PRAGMAs, the real
//! writer and reader pools), runs [`migrate::run`], and roots a [`BlobStore`] at the same directory. Tests are
//! `#[tokio::test]` because `Db::write` and `Db::read` are async. The directory is removed when the `TestDb` drops.

use std::path::Path;

use uuid::Uuid;
use wikanvas_core::api::{Collection, ItemKind, ItemSummary, NewCollection, NewItem, Role};
use wikanvas_store::{BlobStore, Db, DbOptions, ItemRow, Viewer, migrate};

/// A fixed "now": 2026-10-28T09:30:00Z.
pub const NOW: i64 = 1_793_179_800_000;
pub const MINUTE: i64 = 60_000;
pub const DAY: i64 = 86_400_000;

pub struct TestDb {
    pub db: Db,
    pub blobs: BlobStore,
    dir: tempfile::TempDir,
}

impl TestDb {
    /// A new, migrated database in a temp directory.
    pub fn new() -> TestDb {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(dir.path().join("wikanvas.db"), DbOptions::default()).expect("open db");
        let migrated = migrate::run(&db, dir.path(), NOW).expect("migrate");
        assert_eq!((migrated.from, migrated.backup.as_ref()), (0, None));
        assert_eq!(migrated.to, migrate::latest());
        let blobs = BlobStore::new(dir.path().to_path_buf());
        TestDb { db, blobs, dir }
    }

    /// The data directory (holds `wikanvas.db`, `files/`, `tmp/`, `backups/`).
    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    /// Inserts the owner (raw SQL, as the server's harness does) and returns its id.
    pub async fn owner(&self) -> String {
        self.user("owner", "owner@wikanvas.invalid").await
    }

    /// Inserts a user with `role` and returns its id.
    pub async fn user(&self, role: &'static str, email: &'static str) -> String {
        let id = Uuid::now_v7().to_string();
        let ret = id.clone();
        self.db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO users (id, email, name, password_hash, role, created_at, updated_at)
                     VALUES (?1, ?2, 'Test User', NULL, ?3, ?4, ?4)",
                    rusqlite::params![id, email, role, NOW],
                )?;
                Ok(())
            })
            .await
            .expect("insert user");
        ret
    }

    /// Creates a collection named `name` through `collections::create`, as the owner `actor` would.
    pub async fn collection(&self, actor: &str, name: &str) -> Collection {
        let actor = viewer(actor);
        let new = NewCollection {
            name: name.to_owned(),
            ..Default::default()
        };
        self.db
            .write(move |tx| wikanvas_store::collections::create(tx, &actor, &new, NOW))
            .await
            .expect("create collection")
    }

    /// Creates an item through `items::create` (light background for boards).
    pub async fn item(
        &self,
        actor: &str,
        collection: Uuid,
        parent: Option<Uuid>,
        kind: ItemKind,
        title: &str,
    ) -> ItemSummary {
        let actor = actor.to_owned();
        let new = NewItem {
            collection_id: collection,
            parent_id: parent,
            kind,
            title: Some(title.to_owned()),
            position: None,
        };
        self.db
            .write(move |tx| wikanvas_store::items::create(tx, &actor, &new, false, NOW))
            .await
            .expect("create item")
    }

    /// Runs a read query on a reader.
    pub async fn query<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        self.db.read(move |c| Ok(f(c)?)).await.expect("read query")
    }
}

impl TestDb {
    /// Runs `f` in a writer transaction.
    pub async fn w<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> wikanvas_store::Result<T> + Send + 'static,
    ) -> wikanvas_store::Result<T> {
        self.db.write(f).await
    }

    /// Runs `f` on a reader.
    pub async fn r<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> wikanvas_store::Result<T> + Send + 'static,
    ) -> wikanvas_store::Result<T> {
        self.db.read(f).await
    }

    /// The `items` row of `id`.
    pub async fn row(&self, id: Uuid) -> ItemRow {
        let id = id.to_string();
        self.r(move |c| wikanvas_store::shared::item_row(c, &id))
            .await
            .expect("item row")
    }

    /// The titles of a collection's tree, in tree order.
    pub async fn tree_titles(&self, owner: &str, collection: Uuid) -> Vec<String> {
        let (v, c) = (viewer(owner), collection.to_string());
        self.r(move |conn| wikanvas_store::tree::collection_tree(conn, &v, &c))
            .await
            .expect("tree")
            .items
            .into_iter()
            .map(|i| i.title)
            .collect()
    }

    /// The collection's `tree_version`.
    pub async fn tree_version(&self, collection: Uuid) -> i64 {
        count(
            self,
            "SELECT tree_version FROM collections WHERE id = ?1",
            collection.to_string(),
        )
        .await
    }
}

/// The owner as a viewer.
pub fn viewer(user: &str) -> Viewer {
    Viewer {
        user_id: user.to_owned(),
        role: Role::Owner,
    }
}

/// `SELECT <expr>` with one text parameter, as an i64 (counts, flags).
pub async fn count(t: &TestDb, sql: &'static str, param: String) -> i64 {
    t.query(move |c| c.query_row(sql, [param], |r| r.get(0)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wikanvas_core::api::ErrorCode;

    #[tokio::test]
    async fn harness_db_is_migrated_with_the_pragmas() {
        let t = TestDb::new();
        assert!(t.data_dir().join("wikanvas.db").exists());
        let (mode, fk, version, query_only) = t
            .db
            .write(|tx| {
                Ok(tx.query_row(
                    "SELECT (SELECT journal_mode FROM pragma_journal_mode), (SELECT foreign_keys FROM pragma_foreign_keys),
                       (SELECT user_version FROM pragma_user_version), (SELECT query_only FROM pragma_query_only)",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            (mode.as_str(), fk, version, query_only),
            ("wal", 1, i64::from(migrate::latest()), 0)
        );
        let (reader_query_only, busy, trusted): (i64, i64, i64) = t
            .query(|c| {
                c.query_row(
                    "SELECT (SELECT query_only FROM pragma_query_only), (SELECT timeout FROM pragma_busy_timeout),
                       (SELECT trusted_schema FROM pragma_trusted_schema)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .await;
        assert_eq!((reader_query_only, busy, trusted), (1, 5000, 0));
        let maintenance = t.db.maintenance().unwrap();
        let mq: i64 = maintenance
            .query_row("PRAGMA query_only", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mq, 0);
    }

    #[tokio::test]
    async fn readers_cannot_write_and_failed_writes_roll_back() {
        let t = TestDb::new();
        let e =
            t.db.read(|c| {
                c.execute("INSERT INTO settings (key, value) VALUES ('x', '1')", [])?;
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(e.is(ErrorCode::Internal), "{e:?}");
        let e =
            t.db.write(|tx| {
                tx.execute("INSERT INTO settings (key, value) VALUES ('x', '1')", [])?;
                Err::<(), _>(wikanvas_store::Error::validation(
                    "x",
                    "fail after the insert",
                ))
            })
            .await
            .unwrap_err();
        assert!(e.is(ErrorCode::Validation));
        assert_eq!(
            count(
                &t,
                "SELECT count(*) FROM settings WHERE key = ?1",
                "x".into()
            )
            .await,
            0
        );
        t.db.write(|tx| {
            tx.execute("INSERT INTO settings (key, value) VALUES ('x', '1')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            count(
                &t,
                "SELECT count(*) FROM settings WHERE key = ?1",
                "x".into()
            )
            .await,
            1
        );
    }

    #[tokio::test]
    async fn concurrent_writes_serialize_on_the_single_writer() {
        let t = TestDb::new();
        let mut tasks = Vec::new();
        for i in 0..20 {
            let db = t.db.clone();
            tasks.push(tokio::spawn(async move {
                db.write(move |tx| {
                    let n: i64 = tx.query_row(
                        "SELECT count(*) FROM settings WHERE key LIKE 'k%'",
                        [],
                        |r| r.get(0),
                    )?;
                    tx.execute(
                        "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                        rusqlite::params![format!("k{i}"), n.to_string()],
                    )?;
                    Ok(())
                })
                .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        // Every insert saw all earlier ones: the values are 0..20 exactly once.
        let mut values: Vec<i64> = t
            .query(|c| {
                c.prepare("SELECT value FROM settings WHERE key LIKE 'k%'")?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .map(|v| v.map(|s| s.parse::<i64>().unwrap()))
                    .collect()
            })
            .await;
        values.sort();
        assert_eq!(values, (0..20).collect::<Vec<_>>());
    }
}
