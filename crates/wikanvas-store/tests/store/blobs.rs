//! Blob store and garbage collection tests (owner: W2)
//!
//! Staging and commit (D26), streamed staging with its limit, dedupe, sniffing, the free-space floor (D33), and the
//! attachment and blob GC with the orphan sweep (§5, D26, D36).

use bytes::Bytes;
use wikanvas_core::api::ErrorCode;
use wikanvas_store::{BlobStore, Sha256, shared};

use crate::common::{NOW, TestDb, count};

/// A 3×2 RGBA PNG (header and IHDR are what the sniffer and imagesize read).
fn png() -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    v.extend_from_slice(&3u32.to_be_bytes());
    v.extend_from_slice(&2u32.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0\0\0\0\0");
    v.extend_from_slice(b"\0\0\0\0IEND\xaeB`\x82");
    v
}

fn tmp_files(blobs: &BlobStore) -> usize {
    std::fs::read_dir(blobs.tmp_dir())
        .map(|d| d.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn stage_then_commit_places_file_and_row() {
    let t = TestDb::new();
    let staged = t.blobs.stage(&png()).unwrap();
    assert_eq!(staged.mime(), "image/png");
    assert_eq!(staged.dimensions(), Some((3, 2)));
    assert_eq!(staged.sha256(), &Sha256::of(&png()));
    assert!(staged.path().starts_with(t.blobs.tmp_dir()));
    assert_eq!(tmp_files(&t.blobs), 1);
    let blobs = t.blobs.clone();
    let blob =
        t.db.write(move |tx| blobs.commit(tx, staged, NOW))
            .await
            .unwrap();
    assert_eq!(
        (blob.size, blob.mime.as_str(), blob.width, blob.height),
        (png().len() as u64, "image/png", Some(3), Some(2))
    );
    let path = t.blobs.path(&blob.sha256);
    let s = blob.sha256.as_str();
    assert_eq!(
        path,
        t.data_dir()
            .join("files")
            .join(&s[..2])
            .join(&s[2..4])
            .join(s)
    );
    assert_eq!(t.blobs.read(&blob.sha256).unwrap(), png());
    assert_eq!(tmp_files(&t.blobs), 0);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM blobs WHERE sha256 = ?1",
            s.to_owned()
        )
        .await,
        1
    );
}

#[tokio::test]
async fn commit_of_existing_blob_dedupes_and_drop_deletes_staged_file() {
    let t = TestDb::new();
    let blobs = t.blobs.clone();
    let (a, b) =
        t.db.write(move |tx| {
            Ok((
                blobs.put(tx, b"same bytes", NOW)?,
                blobs.put(tx, b"same bytes", NOW + 1)?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.mime, "text/plain");
    assert_eq!(tmp_files(&t.blobs), 0);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM blobs WHERE sha256 = ?1",
            a.sha256.to_string()
        )
        .await,
        1
    );
    // A staged file dropped without a commit (a no-op save) disappears.
    let staged = t.blobs.stage(b"never committed").unwrap();
    let path = staged.path().to_path_buf();
    assert!(path.exists());
    drop(staged);
    assert!(!path.exists());
    // A rolled-back commit leaves the file for the orphan sweep and no row.
    let staged = t.blobs.stage(b"rolled back").unwrap();
    let sha = staged.sha256().clone();
    let blobs = t.blobs.clone();
    let e =
        t.db.write(move |tx| {
            blobs.commit(tx, staged, NOW)?;
            Err::<(), _>(wikanvas_store::Error::validation("x", "abort"))
        })
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
    assert!(t.blobs.path(&sha).exists());
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM blobs WHERE sha256 = ?1",
            sha.to_string()
        )
        .await,
        0
    );
}

#[tokio::test]
async fn stage_stream_hashes_and_enforces_the_limit() {
    let t = TestDb::new();
    let chunks = || {
        futures_util::stream::iter(
            [
                &b"<svg xmlns='http://www.w3.org/2000/svg'>"[..],
                b"<rect/>",
                b"</svg>",
            ]
            .map(|c| Ok::<_, std::io::Error>(Bytes::from_static(c))),
        )
    };
    let whole = b"<svg xmlns='http://www.w3.org/2000/svg'><rect/></svg>";
    let staged = t.blobs.stage_stream(chunks(), 1024).await.unwrap();
    assert_eq!(staged.sha256(), &Sha256::of(whole));
    assert_eq!(
        (staged.size(), staged.mime(), staged.dimensions()),
        (whole.len() as u64, "image/svg+xml", None)
    );
    drop(staged);
    let e = t.blobs.stage_stream(chunks(), 10).await.unwrap_err();
    assert!(e.is(ErrorCode::PayloadTooLarge));
    assert_eq!(e.details, Some(serde_json::json!({"limit_bytes": 10})));
    assert_eq!(tmp_files(&t.blobs), 0, "the partial file is deleted");
    let failing = futures_util::stream::iter([
        Ok(Bytes::from_static(b"abc")),
        Err(std::io::Error::other("client went away")),
    ]);
    let e = t.blobs.stage_stream(failing, 1024).await.unwrap_err();
    assert!(e.is(ErrorCode::Internal));
    assert_eq!(tmp_files(&t.blobs), 0);
    // A PNG streamed in two chunks gets its dimensions from the file.
    let p = png();
    let (head, tail) = p.split_at(10);
    let two = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::copy_from_slice(head)),
        Ok(Bytes::copy_from_slice(tail)),
    ]);
    let staged = t.blobs.stage_stream(two, 1024).await.unwrap();
    assert_eq!(
        (staged.mime(), staged.dimensions()),
        ("image/png", Some((3, 2)))
    );
}

#[tokio::test]
async fn sha256_paths_are_validated() {
    assert!(Sha256::parse("../../etc/passwd").is_err());
    assert!("ABCDEF".parse::<Sha256>().is_err());
    let s = Sha256::of(b"x");
    assert_eq!(s.as_str().len(), 64);
}

#[tokio::test]
async fn free_space_floor() {
    let t = TestDb::new();
    // A floor of 0 MiB always passes; the default floor is capped at 10% of the file system.
    shared::check_free_space(t.data_dir(), 0).unwrap();
    let e = shared::check_free_space(&t.data_dir().join("missing"), 0).unwrap_err();
    assert!(e.is(ErrorCode::Internal));
}

/// Commits `bytes` as a blob and returns its sha256.
async fn put(t: &TestDb, bytes: &'static [u8]) -> Sha256 {
    let blobs = t.blobs.clone();
    t.db.write(move |tx| Ok(blobs.put(tx, bytes, NOW)?.sha256))
        .await
        .unwrap()
}

async fn blob_rows(t: &TestDb) -> Vec<String> {
    t.query(|c| {
        c.prepare("SELECT sha256 FROM blobs ORDER BY sha256")?
            .query_map([], |r| r.get(0))?
            .collect()
    })
    .await
}

/// Backdates a file's mtime by `secs`.
fn age(path: &std::path::Path, secs: u64) {
    let file = std::fs::File::options().append(true).open(path).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs))
        .unwrap();
}

#[tokio::test]
async fn gc_keeps_referenced_blobs_including_revision_assets() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t
        .item(&owner, c.id, None, wikanvas_core::api::ItemKind::Doc, "d")
        .await;
    let board = t
        .item(&owner, c.id, None, wikanvas_core::api::ItemKind::Board, "b")
        .await;
    let attached = put(&t, b"attachment bytes").await;
    let asset = put(&t, b"board asset bytes").await;
    let avatar = put(&t, b"avatar bytes").await;
    let revision = put(&t, b"revision-only asset").await;
    let orphan = put(&t, b"nobody refers to me").await;
    let (a, s, v, r) = (
        attached.to_string(),
        asset.to_string(),
        avatar.to_string(),
        revision.to_string(),
    );
    let (doc_id, board_id, owner_id) = (doc.id.to_string(), board.id.to_string(), owner.clone());
    t.db.write(move |tx| {
        tx.execute(
            "INSERT INTO attachments (id, item_id, sha256, filename, mime, created_by, created_at)
             VALUES ('att-1', ?1, ?2, 'a.png', 'image/png', ?3, ?4)",
            rusqlite::params![doc_id, a, owner_id, NOW],
        )?;
        tx.execute(
            "INSERT INTO board_assets (item_id, asset_id, sha256, mime) VALUES (?1, 'asset-1', ?2, 'image/png')",
            rusqlite::params![board_id, s],
        )?;
        tx.execute("UPDATE users SET avatar_sha256 = ?1", [&v])?;
        tx.execute(
            "INSERT INTO revisions (id, item_id, reason, format, format_version, data, assets, title, byte_size,
                                    created_at)
             VALUES ('rev-1', ?1, 'manual', 'inkboard', 2, x'00', json_object('gone-asset', ?2), 'b', 1, ?3)",
            rusqlite::params![board_id, r, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let report = wikanvas_store::gc::blobs(&t.db, &t.blobs).await.unwrap();
    assert_eq!(report.blobs, 1);
    let mut expect: Vec<String> = [&attached, &asset, &avatar, &revision]
        .iter()
        .map(|s| s.to_string())
        .collect();
    expect.sort();
    assert_eq!(blob_rows(&t).await, expect);
    assert!(
        !t.blobs.path(&orphan).exists(),
        "the unreferenced file is unlinked"
    );
    for kept in [&attached, &asset, &avatar, &revision] {
        assert!(t.blobs.path(kept).exists());
    }
    // A file with no row is swept once it is older than an hour; a young one stays.
    let stray = Sha256::of(b"stray");
    let stray_path = t.blobs.path(&stray);
    std::fs::create_dir_all(stray_path.parent().unwrap()).unwrap();
    std::fs::write(&stray_path, b"stray").unwrap();
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .orphan_files,
        0
    );
    age(&stray_path, 3601);
    age(&t.blobs.path(&attached), 7200);
    let report = wikanvas_store::gc::blobs(&t.db, &t.blobs).await.unwrap();
    assert_eq!((report.blobs, report.orphan_files), (0, 1));
    assert!(!stray_path.exists() && t.blobs.path(&attached).exists());
    // Stale staging files go after a day.
    let tmp = t.blobs.tmp_dir().join("blob-stale.tmp");
    std::fs::create_dir_all(t.blobs.tmp_dir()).unwrap();
    std::fs::write(&tmp, b"x").unwrap();
    age(&tmp, 25 * 3600);
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .stale_tmp,
        1
    );
}

#[tokio::test]
async fn gc_between_stage_and_commit_never_leaves_row_without_file() {
    let t = TestDb::new();
    // An unreferenced blob with its file, and the same bytes staged for a new save.
    let sha = put(&t, b"shared bytes").await;
    let staged = t.blobs.stage(b"shared bytes").unwrap();
    assert_eq!(staged.sha256(), &sha);
    // GC runs between stage and commit: the row and the file go, the staged file stays.
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .blobs,
        1
    );
    assert!(!t.blobs.path(&sha).exists());
    assert!(staged.path().exists());
    // The commit (inside the writer, with the referencing row) puts the file back.
    let blobs = t.blobs.clone();
    t.db.write(move |tx| {
        let blob = blobs.commit(tx, staged, NOW)?;
        tx.execute(
            "INSERT INTO attachments (id, sha256, filename, mime, created_at) VALUES ('a', ?1, 'f', 'text/plain', ?2)",
            rusqlite::params![blob.sha256.as_str(), NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert!(t.blobs.path(&sha).exists());
    assert_eq!(blob_rows(&t).await, [sha.to_string()]);
    // The other order: a commit that dedupes onto an existing file is never undone by a later GC.
    let staged = t.blobs.stage(b"shared bytes").unwrap();
    let blobs = t.blobs.clone();
    t.db.write(move |tx| {
        blobs.commit(tx, staged, NOW)?;
        tx.execute(
            "INSERT INTO attachments (id, sha256, filename, mime, created_at) VALUES ('b', ?1, 'f', 'text/plain', ?2)",
            rusqlite::params![sha.as_str(), NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .blobs,
        0
    );
    for row in blob_rows(&t).await {
        assert!(
            t.blobs.path(&Sha256::parse(&row).unwrap()).exists(),
            "a row without its file"
        );
    }
}

#[tokio::test]
async fn attachment_gc_respects_refs_and_24h_grace() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t
        .item(&owner, c.id, None, wikanvas_core::api::ItemKind::Doc, "d")
        .await;
    let sha = put(&t, b"image").await.to_string();
    let hour = 3_600_000;
    let doc_id = doc.id.to_string();
    t.db.write(move |tx| {
        for (id, created) in [
            ("in-doc", NOW - 48 * hour),
            ("in-revision", NOW - 48 * hour),
            ("logo", NOW - 48 * hour),
            ("young", NOW - 23 * hour),
            ("old", NOW - 25 * hour),
        ] {
            tx.execute(
                "INSERT INTO attachments (id, sha256, filename, mime, created_at) VALUES (?1, ?2, 'f', 'image/png', ?3)",
                rusqlite::params![id, sha, created],
            )?;
        }
        tx.execute(
            "INSERT INTO item_attachments (item_id, attachment_id) VALUES (?1, 'in-doc')",
            [&doc_id],
        )?;
        tx.execute(
            "INSERT INTO revisions (id, item_id, reason, format, format_version, data, attachment_ids, title,
                                    byte_size, created_at)
             VALUES ('rev-1', ?1, 'manual', 'yjs', 1, x'00', '[\"in-revision\"]', 'd', 1, ?2)",
            rusqlite::params![doc_id, NOW],
        )?;
        wikanvas_store::settings::set(tx, wikanvas_store::settings::WORKSPACE_LOGO, &serde_json::json!("logo"))?;
        Ok(())
    })
    .await
    .unwrap();
    let deleted =
        t.db.write(|tx| wikanvas_store::gc::attachments(tx, NOW))
            .await
            .unwrap();
    assert_eq!(deleted, 1);
    let mut left: Vec<String> = t
        .query(|c| {
            c.prepare("SELECT id FROM attachments")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    left.sort();
    assert_eq!(left, ["in-doc", "in-revision", "logo", "young"]);
    // The blob stays while any attachment refers to it.
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .blobs,
        0
    );
    t.db.write(|tx| {
        tx.execute("DELETE FROM attachments", [])?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        wikanvas_store::gc::blobs(&t.db, &t.blobs)
            .await
            .unwrap()
            .blobs,
        1
    );
}
