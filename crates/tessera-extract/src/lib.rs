//! Extraction: turn untrusted raw bytes into a title plus normalized chunks.
//!
//! The [`Extractor`] trait is the seam for the future subprocess plugin host
//! (M2). The M1 built-in path is [`normalize`], which sniffs the content type
//! and dispatches to the per-format extractors. Every extractor treats its input
//! as hostile: bounded work, lossy UTF-8 decoding, and no execution of content.

pub mod chunk;
pub mod dates;
pub mod extractors;
pub mod plugin;
pub mod security;
pub mod sniff;

pub use chunk::PreparedChunk;
pub use extractors::{normalize, Prepared};
pub use sniff::sniff;

use tessera_core::ExtractEvent;

/// How confidently an extractor claims a given input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrength {
    /// The content is unambiguously this type (magic bytes matched).
    Definite,
    /// Plausibly this type (heuristic match); used only if nothing is Definite.
    Maybe,
    /// Not this type.
    No,
}

/// The result of content-type sniffing: never trust the client-declared type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SniffedType {
    /// Best-guess IANA media type.
    pub media_type: String,
    /// A short label the extractor registry dispatches on, e.g. `json`, `log`.
    pub label: String,
}

/// A failure while extracting untrusted content.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("input exceeded a safety limit: {0}")]
    LimitExceeded(String),
    #[error("malformed input: {0}")]
    Malformed(String),
    #[error("extraction failed: {0}")]
    Other(String),
}

/// Turns bytes of a known-sniffed type into normalized events.
///
/// The real trait streams events for bounded memory on huge inputs; M0 pins the
/// synchronous shape and the batch contract, which M1 refines to a stream.
pub trait Extractor: Send + Sync {
    /// Stable extractor id, e.g. `plain_text`.
    fn id(&self) -> &'static str;

    /// How well this extractor matches the sniffed type.
    fn accepts(&self, sniff: &SniffedType) -> MatchStrength;

    /// Extract all events from the input bytes.
    fn extract(&self, input: &[u8]) -> Result<Vec<ExtractEvent>, ExtractError>;
}
