//! The content-addressed store (CAS) for raw ingested bytes.
//!
//! Raw bytes never go in Postgres; they live on disk keyed by their blake3 hash.
//! The hash is the storage key, so writing the same content twice is a no-op
//! (idempotency for free), and reads re-verify the hash so on-disk corruption or
//! tampering is caught rather than served.
//!
//! Layout: `<root>/<first two hex chars>/<full hex>` to keep any one directory
//! from growing unbounded.

use std::path::{Path, PathBuf};

use tessera_core::error::{Error, ErrorKind};
use tessera_core::{ContentHash, ContentHasher};
use tokio::io::{AsyncRead, AsyncReadExt};

/// A handle to the on-disk content store.
#[derive(Clone)]
pub struct CasStore {
    root: PathBuf,
}

/// The result of a streaming write: the content hash, the total byte count, and
/// the first bytes of the content, so the caller can sniff the type without
/// re-reading the object from disk.
#[derive(Debug)]
pub struct Stored {
    pub hash: ContentHash,
    pub size: u64,
    pub head: Vec<u8>,
}

/// Read buffer size for streaming writes.
const STREAM_CHUNK: usize = 64 * 1024;

fn io_err(context: &str, e: std::io::Error) -> Error {
    Error::new(ErrorKind::Io, format!("cas {context}: {e}"))
}

impl CasStore {
    /// Open (creating the root if needed).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| io_err("create root", e))?;
        Ok(Self { root })
    }

    /// Absolute path where `hash` is (or would be) stored.
    #[must_use]
    pub fn path_for(&self, hash: &ContentHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[0..2]).join(&hex)
    }

    /// Whether content with this hash is already stored.
    pub async fn exists(&self, hash: &ContentHash) -> bool {
        tokio::fs::try_exists(self.path_for(hash))
            .await
            .unwrap_or(false)
    }

    /// Store bytes already held in memory, returning the content hash and size.
    /// Idempotent: if the content is already present, this does no write and
    /// returns the same hash. For content arriving as a stream (an upload), use
    /// [`write_streaming`](Self::write_streaming) so the whole body is never
    /// buffered.
    pub async fn write_bytes(&self, bytes: &[u8]) -> Result<(ContentHash, u64), Error> {
        let hash = ContentHash::of(bytes);
        let size = bytes.len() as u64;

        if self.exists(&hash).await {
            return Ok((hash, size));
        }
        let tmp = self.temp_path();
        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|e| io_err("write temp", e))?;
        self.commit(&tmp, &hash).await?;
        Ok((hash, size))
    }

    /// Store content read from `reader`, hashing while writing so the whole body
    /// is never buffered in memory. Aborts with `TooLarge` if the content exceeds
    /// `max_bytes`. Returns the hash, the size, and the first `head_cap` bytes
    /// (for the caller to sniff). Idempotent and atomic, like `write_bytes`.
    pub async fn write_streaming<R: AsyncRead + Unpin>(
        &self,
        mut reader: R,
        max_bytes: u64,
        head_cap: usize,
    ) -> Result<Stored, Error> {
        use tokio::io::AsyncWriteExt as _;

        let tmp = self.temp_path();
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| io_err("create temp", e))?;

        let mut hasher = ContentHasher::new();
        let mut size: u64 = 0;
        let mut head: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; STREAM_CHUNK];

        loop {
            let n = match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tokio::fs::remove_file(&tmp).await.ok();
                    return Err(io_err("read stream", e));
                }
            };
            size += n as u64;
            if size > max_bytes {
                tokio::fs::remove_file(&tmp).await.ok();
                return Err(Error::new(
                    ErrorKind::TooLarge,
                    "content exceeds size limit",
                ));
            }
            hasher.update(&buf[..n]);
            if head.len() < head_cap {
                let take = (head_cap - head.len()).min(n);
                head.extend_from_slice(&buf[..take]);
            }
            if let Err(e) = file.write_all(&buf[..n]).await {
                tokio::fs::remove_file(&tmp).await.ok();
                return Err(io_err("write temp", e));
            }
        }
        if let Err(e) = file.flush().await {
            tokio::fs::remove_file(&tmp).await.ok();
            return Err(io_err("flush temp", e));
        }
        drop(file);

        let hash = hasher.finalize();
        if self.exists(&hash).await {
            tokio::fs::remove_file(&tmp).await.ok();
        } else {
            self.commit(&tmp, &hash).await?;
        }
        Ok(Stored { hash, size, head })
    }

    /// A unique temp path in the store root (same filesystem as the final object,
    /// so the rename in `commit` is atomic).
    fn temp_path(&self) -> PathBuf {
        self.root.join(format!(".tmp-{}", unique_suffix()))
    }

    /// Move a written temp file into place under its content hash. Handles the
    /// race where another writer stored the same content first (the temp is
    /// discarded and the existing object wins). Any failure removes the temp.
    async fn commit(&self, tmp: &Path, hash: &ContentHash) -> Result<(), Error> {
        let final_path = self.path_for(hash);
        let dir = final_path.parent().expect("path_for always has a parent");
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            tokio::fs::remove_file(tmp).await.ok();
            return Err(io_err("create shard dir", e));
        }
        match tokio::fs::rename(tmp, &final_path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                tokio::fs::remove_file(tmp).await.ok();
                if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
                    Ok(())
                } else {
                    Err(io_err("rename into place", e))
                }
            }
        }
    }

    /// Read content by hash, verifying the bytes still hash to the key. A
    /// mismatch means on-disk corruption or tampering and is a hard error.
    pub async fn read_verified(&self, hash: &ContentHash) -> Result<Vec<u8>, Error> {
        let path = self.path_for(hash);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::new(ErrorKind::NotFound, "content not in store"));
            }
            Err(e) => return Err(io_err("read", e)),
        };
        if ContentHash::of(&bytes) != *hash {
            return Err(Error::new(
                ErrorKind::Internal,
                "content hash mismatch on read (corruption or tampering)",
            ));
        }
        Ok(bytes)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// A process-unique suffix for temp filenames, derived from a fresh uuidv7 so we
/// avoid `Math.random`-style nondeterminism concerns and stay collision-free.
fn unique_suffix() -> String {
    tessera_core::new_id().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::CasStore;
    use tessera_core::ContentHash;

    #[tokio::test]
    async fn write_read_roundtrip_and_dedup() {
        let dir = std::env::temp_dir().join(format!(
            "tessera-cas-test-{}",
            tessera_core::new_id().simple()
        ));
        let cas = CasStore::open(&dir).unwrap();

        let (h1, n1) = cas.write_bytes(b"the quick brown fox").await.unwrap();
        assert_eq!(n1, 19);
        assert!(cas.exists(&h1).await);

        // Idempotent second write returns the same hash and does not error.
        let (h2, _) = cas.write_bytes(b"the quick brown fox").await.unwrap();
        assert_eq!(h1, h2);

        let read = cas.read_verified(&h1).await.unwrap();
        assert_eq!(read, b"the quick brown fox");

        // Unknown hash reads as NotFound.
        let missing = ContentHash::of(b"never stored");
        assert!(cas.read_verified(&missing).await.is_err());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn streaming_write_matches_hashes_caps_and_dedups() {
        let dir = std::env::temp_dir().join(format!(
            "tessera-cas-stream-{}",
            tessera_core::new_id().simple()
        ));
        let cas = CasStore::open(&dir).unwrap();
        let content = b"streamed content that is reasonably long for a head cap";

        // Streaming and one-shot writes agree on the hash, and the head is the
        // requested prefix.
        let stored = cas
            .write_streaming(&content[..], 1_000_000, 8)
            .await
            .unwrap();
        assert_eq!(stored.size, content.len() as u64);
        assert_eq!(stored.hash, ContentHash::of(content));
        assert_eq!(stored.head, &content[..8]);
        assert_eq!(cas.read_verified(&stored.hash).await.unwrap(), content);

        // head_cap beyond the content yields the whole content.
        let small = cas.write_streaming(&b"hi"[..], 1000, 64).await.unwrap();
        assert_eq!(small.head, b"hi");

        // Dedup: streaming content already stored by write_bytes is a no-op that
        // returns the same hash.
        let (h, _) = cas.write_bytes(content).await.unwrap();
        let again = cas
            .write_streaming(&content[..], 1_000_000, 8)
            .await
            .unwrap();
        assert_eq!(again.hash, h);

        // The size cap aborts before storing, and leaves no object under the key.
        let err = cas.write_streaming(&content[..], 8, 8).await.unwrap_err();
        assert!(
            err.message().contains("size limit"),
            "got: {}",
            err.message()
        );

        // No stray temp files remain in the root.
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(e) = rd.next_entry().await.unwrap() {
            let name = e.file_name();
            assert!(
                !name.to_string_lossy().starts_with(".tmp-"),
                "leftover temp file: {name:?}"
            );
        }

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn read_detects_corruption() {
        let dir = std::env::temp_dir().join(format!(
            "tessera-cas-corrupt-{}",
            tessera_core::new_id().simple()
        ));
        let cas = CasStore::open(&dir).unwrap();
        let (h, _) = cas.write_bytes(b"trustworthy content").await.unwrap();

        // Corrupt the stored file in place; the read must reject it.
        tokio::fs::write(cas.path_for(&h), b"tampered content!!!")
            .await
            .unwrap();
        let err = cas.read_verified(&h).await.unwrap_err();
        assert!(err.message().contains("mismatch"), "got: {}", err.message());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
