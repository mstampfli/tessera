//! Identifier helpers.
//!
//! Every primary key in tessera is a `UUIDv7`: time-ordered, so ids sort by
//! creation and support keyset pagination (`WHERE id > $cursor ORDER BY id`)
//! without a separate timestamp column, and random enough to be unguessable.

use uuid::Uuid;

/// Mint a fresh time-ordered id for a new row.
#[must_use]
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::new_id;

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        // UUIDv7 encodes a millisecond timestamp in the high bits, so ids minted
        // later never sort before earlier ones.
        assert!(b >= a, "uuidv7 ids should be monotonically non-decreasing");
        assert_eq!(a.get_version_num(), 7);
    }
}
