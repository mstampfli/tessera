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
use tessera_core::ContentHash;

/// A handle to the on-disk content store.
#[derive(Clone)]
pub struct CasStore {
    root: PathBuf,
}

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

    /// Store bytes, returning the content hash and size. Idempotent: if the
    /// content is already present, this does no write and returns the same hash.
    /// The write is atomic (temp file then rename) so a crash mid-write never
    /// leaves a partial object under the final key.
    pub async fn write_bytes(&self, bytes: &[u8]) -> Result<(ContentHash, u64), Error> {
        let hash = ContentHash::of(bytes);
        let final_path = self.path_for(&hash);
        let size = bytes.len() as u64;

        if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
            return Ok((hash, size));
        }

        let dir = final_path.parent().expect("path_for always has a parent");
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| io_err("create shard dir", e))?;

        // Write to a unique temp file in the same directory, then rename. The
        // temp name embeds the hash plus this task's id to avoid collisions
        // between concurrent writers of the same content.
        let tmp = dir.join(format!(".tmp-{}-{}", hash.to_hex(), unique_suffix()));
        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|e| io_err("write temp", e))?;
        match tokio::fs::rename(&tmp, &final_path).await {
            Ok(()) => Ok((hash, size)),
            Err(e) => {
                // Another writer may have won the race; if the final object now
                // exists, that is success. Otherwise surface the error.
                tokio::fs::remove_file(&tmp).await.ok();
                if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
                    Ok((hash, size))
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
