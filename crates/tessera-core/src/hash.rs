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

#[cfg(test)]
mod tests {
    use super::ContentHash;

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
}
