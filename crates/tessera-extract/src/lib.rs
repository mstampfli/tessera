//! Extraction: turn untrusted raw bytes into a stream of normalized
//! [`tessera_core::ExtractEvent`]s.
//!
//! M0 establishes the [`Extractor`] seam and the content-sniffing type. The
//! built-in extractors (text, markdown, json, csv, log, html, pdf), the shared
//! chunker, the security entity pack, and the sandboxed subprocess plugin host
//! land in M1 and M2. Every extractor treats its input as hostile: bounded
//! readers, per-event size caps, and one shared `clean_text` primitive.

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
