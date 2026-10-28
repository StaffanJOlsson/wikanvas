//! Blob store and garbage collection tests (owner: W2)
//!
//! W0b: staging and commit (D26), streamed staging with its limit, dedupe, sniffing, and the free-space floor (D33).

use bytes::Bytes;
use quire_core::api::ErrorCode;
use quire_store::{BlobStore, Sha256, shared};

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
            Err::<(), _>(quire_store::Error::validation("x", "abort"))
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
