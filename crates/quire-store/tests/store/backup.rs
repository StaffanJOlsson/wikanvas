//! Backup tests through the maintenance connection (owner: W2)
//!
//! W0b: the pre-migration backup through the real `Db`, and the migration runner around it.

use quire_core::api::{ErrorCode, ItemKind};
use quire_store::{Db, DbOptions, backup, migrate};

use crate::common::{NOW, TestDb};

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
