//! Shared, dependency-light foundation for tessera: domain ids, the error
//! taxonomy, secret-handling primitives, the normalized extractor event format,
//! and the layered configuration loader.
//!
//! This crate performs no I/O. Everything above it in the workspace DAG builds
//! on these types so that ids, errors, and secret handling have exactly one
//! definition (house rule: one shared mechanism, never two drifting paths).

pub mod config;
pub mod error;
pub mod extract_event;
pub mod hash;
pub mod ids;
pub mod secret;

pub use error::{Error, ErrorKind, Result};
pub use extract_event::ExtractEvent;
pub use hash::{ContentHash, ContentHasher};
pub use ids::new_id;
