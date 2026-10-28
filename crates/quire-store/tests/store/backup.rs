//! Backup tests through the maintenance connection (owner: W2)
//!
//! The pre-migration backup and the migration runner around it; scheduled and on-demand backups through the real
//! `Db` (the maintenance connection) with `quick_check`, rotation under `QUIRE_BACKUP_KEEP`, and `--out`.

use quire_core::api::{ErrorCode, ItemKind};
use quire_store::{Db, DbOptions, backup, migrate};

use crate::common::{MINUTE, NOW, TestDb};

#[tokio::test]
async fn pre_migration_vacuum_into_through_db_passes_quick_check() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "kept in the copy")
        .await;
    let path = backup::pre_migration(&t.db, t.data_dir(), 1, NOW).unwrap();
    assert_eq!(
        path,
        t.data_dir()
            .join("backups")
            .join("pre-migrate-v1-20261028T0930Z.db")
    );
    backup::quick_check(&path).unwrap();
    let copy = rusqlite::Connection::open(&path).unwrap();
    let title: String = copy
        .query_row("SELECT title FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "kept in the copy");
    drop(copy);
    // A second backup in the same minute gets its own name.
    let second = backup::pre_migration(&t.db, t.data_dir(), 1, NOW).unwrap();
    assert_eq!(
        second.file_name().unwrap(),
        "pre-migrate-v1-20261028T0930Z-2.db"
    );
}

#[tokio::test]
async fn migrate_is_idempotent_and_refuses_a_newer_database() {
    let t = TestDb::new();
    let again = migrate::run(&t.db, t.data_dir(), NOW).unwrap();
    assert_eq!((again.from, again.to, again.backup), (1, 1, None));
    assert!(!t.data_dir().join("backups").exists());
    migrate::migrations().validate().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("quire.db"), DbOptions::default()).unwrap();
    migrate::run(&db, dir.path(), NOW).unwrap();
    db.maintenance()
        .unwrap()
        .execute_batch("PRAGMA user_version = 99")
        .unwrap();
    let e = migrate::run(&db, dir.path(), NOW).unwrap_err();
    assert!(e.is(ErrorCode::Internal));
    assert!(e.message.contains("newer"), "{}", e.message);
}

#[tokio::test]
async fn vacuum_into_through_db_passes_quick_check() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    t.item(&owner, c.id, None, ItemKind::Doc, "in the backup")
        .await;
    // A writer transaction in flight does not block the copy (WAL), and the copy is the committed state.
    let path = backup::create(&t.db, t.data_dir(), 7, NOW).unwrap();
    assert_eq!(
        path,
        t.data_dir().join("backups").join("quire-20261028T0930Z.db")
    );
    backup::quick_check(&path).unwrap();
    let copy =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let title: String = copy
        .query_row("SELECT title FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "in the backup");
    let version: u32 = copy
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, migrate::latest());
    drop(copy);
    // No temporary file is left, and the name is unique within a minute.
    let second = backup::create(&t.db, t.data_dir(), 7, NOW).unwrap();
    assert_eq!(second.file_name().unwrap(), "quire-20261028T0930Z-2.db");
    let names: Vec<String> = std::fs::read_dir(t.data_dir().join("backups"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    assert!(names.iter().all(|n| !n.starts_with(".tmp-")), "{names:?}");
    // --out writes elsewhere, once.
    let out = t.data_dir().join("manual.db");
    backup::to_path(&t.db, &out).unwrap();
    backup::quick_check(&out).unwrap();
    assert!(
        backup::to_path(&t.db, &out)
            .unwrap_err()
            .is(ErrorCode::Validation)
    );
}

#[tokio::test]
async fn on_demand_counts_against_keep() {
    let t = TestDb::new();
    t.owner().await;
    let pre = backup::pre_migration(&t.db, t.data_dir(), 1, NOW).unwrap();
    let mut made = Vec::new();
    for i in 0..4 {
        made.push(backup::create(&t.db, t.data_dir(), 2, NOW + i * MINUTE).unwrap());
    }
    let left = backup::list(&t.data_dir().join("backups")).unwrap();
    assert_eq!(left, made[2..].to_vec(), "the newest two remain");
    assert!(pre.exists(), "pre-migration backups are kept separately");
    // keep 0 still keeps the one just made.
    backup::create(&t.db, t.data_dir(), 0, NOW + 10 * MINUTE).unwrap();
    assert_eq!(
        backup::list(&t.data_dir().join("backups")).unwrap().len(),
        1
    );
    // Suffixes sort after their base name.
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "quire-20261028T0930Z-10.db",
        "quire-20261028T0930Z-2.db",
        "quire-20261028T0930Z.db",
        "x.db",
    ] {
        std::fs::write(dir.path().join(name), b"").unwrap();
    }
    let names: Vec<String> = backup::list(dir.path())
        .unwrap()
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "quire-20261028T0930Z.db",
            "quire-20261028T0930Z-2.db",
            "quire-20261028T0930Z-10.db"
        ]
    );
}

#[tokio::test]
async fn create_is_busy_while_another_process_holds_the_lock() {
    let t = TestDb::new();
    t.owner().await;
    let dir = t.data_dir().join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    // Another process's backup: a second open of the lock file conflicts like another process would.
    let held = backup::lock(&dir).unwrap();
    let e = backup::create(&t.db, t.data_dir(), 7, NOW).unwrap_err();
    assert!(e.is(ErrorCode::RateLimited), "{e:?}");
    assert_eq!(e.details, Some(serde_json::json!({"reason": "busy"})));
    assert!(backup::list(&dir).unwrap().is_empty(), "nothing was copied");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "only the lock file"
    );
    drop(held);
    backup::create(&t.db, t.data_dir(), 7, NOW).unwrap();
    // The lock is released when create returns.
    drop(backup::lock(&dir).unwrap());
}
