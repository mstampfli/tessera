//! Content hashing. A blake3 digest identifies ingested content: it is the
//! content-addressed store key, the idempotency key, and the dedup anchor, all
//! at once. This module is pure compute (no I/O).

use crate::error::{Error, ErrorKind};

/// A blake3 content digest (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hash a byte slice.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Reconstruct from stored raw bytes (e.g. a `bytea` column).
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::new(ErrorKind::Invalid, "content hash must be 32 bytes"))?;
        Ok(Self(arr))
    }

    /// The raw 32 bytes, for storage.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex encoding, for the on-disk path and display.
    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

/// Incremental content hasher: the streaming twin of [`ContentHash::of`], for
/// content read in chunks (e.g. an upload) so the whole body is never buffered.
/// Feeding the same bytes here, in any chunking, yields the same hash as `of`.
#[derive(Default)]
pub struct ContentHasher(blake3::Hasher);

impl ContentHasher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold the next slice of content into the digest.
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Finalize into a [`ContentHash`]. Does not consume the hasher.
    #[must_use]
    pub fn finalize(&self) -> ContentHash {
        ContentHash(*self.0.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentHash, ContentHasher};

    #[test]
    fn hash_is_stable_and_roundtrips() {
        let a = ContentHash::of(b"hello");
        let b = ContentHash::of(b"hello");
        let c = ContentHash::of(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.to_hex().len(), 64);

        let restored = ContentHash::from_slice(a.as_bytes()).unwrap();
        assert_eq!(a, restored);
        assert!(ContentHash::from_slice(b"too short").is_err());
    }

    #[test]
    fn incremental_matches_oneshot_regardless_of_chunking() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let oneshot = ContentHash::of(data);

        let mut h = ContentHasher::new();
        h.update(&data[..5]);
        h.update(&data[5..20]);
        h.update(&data[20..]);
        assert_eq!(h.finalize(), oneshot);

        // Empty content still matches.
        assert_eq!(ContentHasher::new().finalize(), ContentHash::of(b""));
    }
}
