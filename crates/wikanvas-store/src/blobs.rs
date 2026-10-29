//! The content-addressed blob store under /data/files (owner: W2).
//!
//! D26: blob files live at `<root>/files/<sha[0:2]>/<sha[2:4]>/<sha256>`, staging files under `<root>/tmp/`, where
//! `root` is the data directory (`/data`), so a staged file is renamed into place on the same file system. Write path:
//! [`BlobStore::stage`] (or [`BlobStore::stage_stream`] for uploads) writes, hashes, fsyncs and sniffs outside the
//! writer; [`BlobStore::commit`] renames into place (or drops the staged file and refreshes the mtime when the blob
//! already exists) and `INSERT OR IGNORE`s the `blobs` row, inside the writer transaction. A staged file that is
//! dropped without a commit is deleted. Files without a row are swept by the GC after 1 h ([`crate::gc`]).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256 as Sha256Hasher};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

/// A validated sha256: 64 lowercase hex characters (the `blobs` primary key and file name).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256(String);

/// A string that is not 64 lowercase hex characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidSha256;

impl fmt::Display for InvalidSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a sha256 (64 lowercase hex characters)")
    }
}

impl std::error::Error for InvalidSha256 {}

impl Sha256 {
    /// Validates `s`: exactly 64 characters of `0-9a-f`.
    pub fn parse(s: &str) -> std::result::Result<Sha256, InvalidSha256> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Sha256(s.to_owned()))
        } else {
            Err(InvalidSha256)
        }
    }

    /// The sha256 of `bytes`.
    pub fn of(bytes: &[u8]) -> Sha256 {
        Sha256::from_digest(&Sha256Hasher::digest(bytes).into())
    }

    /// From a raw 32-byte digest.
    pub fn from_digest(digest: &[u8; 32]) -> Sha256 {
        let mut s = String::with_capacity(64);
        for b in digest {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 0xf)]));
        }
        Sha256(s)
    }

    /// The hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Sha256 {
    type Err = InvalidSha256;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Sha256::parse(s)
    }
}

impl AsRef<str> for Sha256 {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A blob as stored: what [`BlobStore::commit`] returns. `mime` is sniffed from the bytes, never taken from the
/// client; `width` and `height` come from an image header only (imagesize) and are `None` for anything else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobRef {
    pub sha256: Sha256,
    pub size: u64,
    pub mime: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A file staged under `<root>/tmp/`, hashed, fsynced and sniffed, waiting for [`BlobStore::commit`]. Dropping it
/// uncommitted deletes the file.
#[derive(Debug)]
pub struct Staged {
    tmp: Option<PathBuf>,
    blob: BlobRef,
}

impl Staged {
    /// What the blob will be once committed.
    pub fn blob(&self) -> &BlobRef {
        &self.blob
    }

    /// Its sha256.
    pub fn sha256(&self) -> &Sha256 {
        &self.blob.sha256
    }

    /// Its size in bytes.
    pub fn size(&self) -> u64 {
        self.blob.size
    }

    /// The sniffed media type.
    pub fn mime(&self) -> &str {
        &self.blob.mime
    }

    /// Image width and height from the header, if it is an image.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        Some((self.blob.width?, self.blob.height?))
    }

    /// The staged file.
    pub fn path(&self) -> &Path {
        self.tmp.as_deref().unwrap_or(Path::new(""))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(tmp) = self.tmp.take() {
            let _ = fs::remove_file(tmp);
        }
    }
}

/// The blob store (D26). Cheap to clone; usually held as `Arc<BlobStore>` in the server state.
#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// A store rooted at the data directory (`WIKANVAS_DATA_DIR`, `/data` in the image): blobs go under `root/files`,
    /// staging files under `root/tmp`. Directories are created on first use.
    pub fn new(root: PathBuf) -> Self {
        BlobStore { root }
    }

    /// The data directory this store is rooted at.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The staging directory, `root/tmp`.
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// The blob directory, `root/files`.
    pub fn files_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    /// Writes `bytes` to a new staging file, fsyncs it, hashes and sniffs it. Call it outside the writer.
    pub fn stage(&self, bytes: &[u8]) -> std::io::Result<Staged> {
        let (tmp, mut file) = self.new_tmp()?;
        // From here on `staged` owns the file: an error below deletes it.
        let mut staged = Staged {
            tmp: Some(tmp),
            blob: BlobRef {
                sha256: Sha256::of(bytes),
                size: bytes.len() as u64,
                mime: String::new(),
                width: None,
                height: None,
            },
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let mime = sniff(bytes);
        let dims = if is_raster(mime) {
            imagesize::blob_size(bytes).ok()
        } else {
            None
        };
        staged.blob.mime = mime.to_owned();
        set_dims(&mut staged.blob, dims);
        Ok(staged)
    }

    /// Streams an upload into a staging file, hashing as it goes, never holding more than one chunk. More than
    /// `limit` bytes gives 413 `payload_too_large` (`details.limit_bytes`) and deletes the partial file; a stream
    /// error is passed on as `internal`.
    pub async fn stage_stream(
        &self,
        mut body: impl Stream<Item = std::io::Result<Bytes>> + Unpin,
        limit: u64,
    ) -> Result<Staged> {
        let (tmp, file) = self.new_tmp()?;
        let mut staged = Staged {
            tmp: Some(tmp),
            blob: BlobRef {
                sha256: Sha256(String::new()),
                size: 0,
                mime: String::new(),
                width: None,
                height: None,
            },
        };
        let mut file = tokio::fs::File::from_std(file);
        let mut hasher = Sha256Hasher::new();
        let mut head: Vec<u8> = Vec::with_capacity(SNIFF_LEN);
        let mut size: u64 = 0;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| Error::internal("reading the upload", e))?;
            size += chunk.len() as u64;
            if size > limit {
                return Err(Error::too_large(limit));
            }
            if head.len() < SNIFF_LEN {
                let take = (SNIFF_LEN - head.len()).min(chunk.len());
                head.extend_from_slice(&chunk[..take]);
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        let mime = sniff(&head);
        staged.blob.sha256 = Sha256::from_digest(&hasher.finalize().into());
        staged.blob.size = size;
        staged.blob.mime = mime.to_owned();
        if is_raster(mime) {
            let path = staged.path().to_path_buf();
            let dims = tokio::task::spawn_blocking(move || imagesize::size(path).ok())
                .await
                .map_err(|e| Error::internal("reading the image header", e))?;
            set_dims(&mut staged.blob, dims);
        }
        Ok(staged)
    }

    /// Inside the writer transaction: moves the staged file into place (when the blob file already exists, drops
    /// the staged file and refreshes the existing file's mtime, so the orphan sweep leaves it alone), then
    /// `INSERT OR IGNORE`s the `blobs` row. Returns the row as stored.
    pub fn commit(&self, tx: &Transaction<'_>, mut staged: Staged, now: i64) -> Result<BlobRef> {
        let dest = self.path(&staged.blob.sha256);
        let tmp = staged.tmp.take().ok_or_else(|| {
            Error::new(
                wikanvas_core::api::ErrorCode::Internal,
                "blob already committed",
            )
        })?;
        if dest.exists() {
            let _ = fs::remove_file(&tmp);
            File::options()
                .append(true)
                .open(&dest)?
                .set_modified(std::time::SystemTime::now())?;
        } else {
            let dir = dest.parent().expect("blob paths have a parent");
            if let Err(e) = fs::create_dir_all(dir).and_then(|()| fs::rename(&tmp, &dest)) {
                let _ = fs::remove_file(&tmp);
                return Err(e.into());
            }
            if let Ok(d) = File::open(dir) {
                let _ = d.sync_all();
            }
        }
        let b = &staged.blob;
        tx.execute(
            "INSERT OR IGNORE INTO blobs (sha256, size, mime, width, height, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                b.sha256.as_str(),
                b.size as i64,
                b.mime,
                b.width,
                b.height,
                now
            ],
        )?;
        let stored = tx
            .query_row(
                "SELECT size, mime, width, height FROM blobs WHERE sha256 = ?1",
                [b.sha256.as_str()],
                |r| {
                    Ok(BlobRef {
                        sha256: b.sha256.clone(),
                        size: r.get::<_, i64>(0)? as u64,
                        mime: r.get(1)?,
                        width: r.get(2)?,
                        height: r.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::new(
                    wikanvas_core::api::ErrorCode::Internal,
                    "blob row missing after insert",
                )
            })?;
        Ok(stored)
    }

    /// [`BlobStore::stage`] then [`BlobStore::commit`], for small blobs written inside the writer (the seed).
    pub fn put(&self, tx: &Transaction<'_>, bytes: &[u8], now: i64) -> Result<BlobRef> {
        let staged = self.stage(bytes)?;
        self.commit(tx, staged, now)
    }

    /// The bytes of a blob.
    pub fn read(&self, sha256: &Sha256) -> std::io::Result<Vec<u8>> {
        fs::read(self.path(sha256))
    }

    /// The file of a blob: `root/files/<sha[0:2]>/<sha[2:4]>/<sha256>`.
    pub fn path(&self, sha256: &Sha256) -> PathBuf {
        let s = sha256.as_str();
        self.files_dir().join(&s[0..2]).join(&s[2..4]).join(s)
    }

    fn new_tmp(&self) -> std::io::Result<(PathBuf, File)> {
        let dir = self.tmp_dir();
        fs::create_dir_all(&dir)?;
        let name = crate::shared::random_base62(16).map_err(std::io::Error::other)?;
        let path = dir.join(format!("blob-{name}.tmp"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((path, file))
    }
}

fn set_dims(blob: &mut BlobRef, dims: Option<imagesize::ImageSize>) {
    if let Some(d) = dims {
        blob.width = u32::try_from(d.width).ok();
        blob.height = u32::try_from(d.height).ok();
        if blob.width.is_none() || blob.height.is_none() {
            blob.width = None;
            blob.height = None;
        }
    }
}

/// Bytes of the start of a file that [`sniff`] looks at.
pub const SNIFF_LEN: usize = 4096;

fn is_raster(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml"
}

/// The media type of a file from its first bytes (magic numbers; never the client's word): the web image formats
/// (with AVIF and HEIF through imagesize), SVG, PDF, zip, gzip, common audio and video containers, UTF-8 text, else
/// `application/octet-stream`.
pub fn sniff(head: &[u8]) -> &'static str {
    use imagesize::ImageType;
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return "image/jpeg";
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        return "image/webp";
    }
    // ISO base media files: imagesize calls every `ftyp` HEIF, so the major brand decides.
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        match &head[8..12] {
            b"avif" | b"avis" => return "image/avif",
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => {
                return "image/heif";
            }
            b"M4A " => return "audio/mp4",
            b"qt  " => return "video/quicktime",
            _ => return "video/mp4",
        }
    }
    if let Ok(t) = imagesize::image_type(head) {
        let m = match t {
            ImageType::Png => Some("image/png"),
            ImageType::Jpeg => Some("image/jpeg"),
            ImageType::Gif => Some("image/gif"),
            ImageType::Webp => Some("image/webp"),
            ImageType::Bmp => Some("image/bmp"),
            ImageType::Ico => Some("image/x-icon"),
            ImageType::Tiff => Some("image/tiff"),
            ImageType::Jxl => Some("image/jxl"),
            ImageType::Heif(imagesize::Compression::Av1) => Some("image/avif"),
            ImageType::Heif(_) => Some("image/heif"),
            _ => None,
        };
        if let Some(m) = m {
            return m;
        }
    }
    if head.starts_with(b"%PDF-") {
        return "application/pdf";
    }
    if head.starts_with(b"PK\x03\x04") {
        return "application/zip";
    }
    if head.starts_with(&[0x1f, 0x8b]) {
        return "application/gzip";
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WAVE" {
        return "audio/wav";
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return "video/webm";
    }
    if head.starts_with(b"OggS") {
        return "audio/ogg";
    }
    if head.starts_with(b"fLaC") {
        return "audio/flac";
    }
    if head.starts_with(b"ID3") || (head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0) {
        return "audio/mpeg";
    }
    let text = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    // A multi-byte character may be cut at the end of the sniffed window.
    let valid = match std::str::from_utf8(text) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > text.len(),
    };
    if valid && !text.contains(&0) {
        let start = text.trim_ascii_start();
        let lower: Vec<u8> = start
            .iter()
            .take(1024)
            .map(u8::to_ascii_lowercase)
            .collect();
        if lower.starts_with(b"<svg")
            || ((lower.starts_with(b"<?xml") || lower.starts_with(b"<!doctype svg"))
                && lower.windows(4).any(|w| w == b"<svg"))
        {
            return "image/svg+xml";
        }
        if !text.is_empty() {
            return "text/plain";
        }
    }
    "application/octet-stream"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_parse_and_format() {
        let s = Sha256::of(b"abc");
        assert_eq!(
            s.as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Sha256::parse(s.as_str()), Ok(s.clone()));
        assert!(Sha256::parse(&s.as_str().to_uppercase()).is_err());
        assert!(Sha256::parse("abc").is_err());
        assert!(Sha256::parse(&"g".repeat(64)).is_err());
    }

    #[test]
    fn sniff_magic_numbers() {
        assert_eq!(
            sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0"),
            "image/png"
        );
        assert_eq!(sniff(b"GIF89a\x01\x00\x01\x00"), "image/gif");
        assert_eq!(sniff(b"%PDF-1.7\n"), "application/pdf");
        assert_eq!(
            sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            "image/svg+xml"
        );
        assert_eq!(sniff(b"<?xml version=\"1.0\"?>\n<svg/>"), "image/svg+xml");
        assert_eq!(sniff(b"hello, world\n"), "text/plain");
        assert_eq!(sniff(b"\0\0\0\x18ftypmp42\0\0\0\0"), "video/mp4");
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0\0\0"), "image/avif");
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0"), "image/heif");
        assert_eq!(sniff(b"OggS\0\x02"), "audio/ogg");
        assert_eq!(sniff(b"ID3\x04\0"), "audio/mpeg");
        assert_eq!(sniff(b"\0\x01\x02binary"), "application/octet-stream");
        assert_eq!(sniff(b""), "application/octet-stream");
    }
}
