//! The single error taxonomy for tessera.
//!
//! Every fallible operation in the workspace returns [`Error`]. The API layer
//! maps [`ErrorKind`] to an RFC 9457 problem+json response with a stable `type`
//! slug; internal detail is logged, never leaked to 5xx bodies.

use std::fmt;

/// Convenience alias used throughout the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A coarse classification that drives HTTP status mapping and log severity.
///
/// Keep this list small and stable; the slugs are part of the public API
/// contract for programmatic clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The request was malformed or failed validation (400).
    Invalid,
    /// Authentication is required or the supplied credential is bad (401).
    Unauthorized,
    /// The principal is known but lacks the required scope (403).
    Forbidden,
    /// The addressed resource does not exist (404).
    NotFound,
    /// The request conflicts with current state, e.g. a unique violation (409).
    Conflict,
    /// The payload exceeded a configured limit (413).
    TooLarge,
    /// A rate limit or quota was hit (429).
    RateLimited,
    /// An upstream operation timed out (504).
    Timeout,
    /// A model/provider call failed (502).
    Provider,
    /// Content extraction failed on untrusted input (422).
    Extract,
    /// A database operation failed (500).
    Db,
    /// A filesystem or I/O operation failed (500).
    Io,
    /// An unclassified internal fault (500).
    Internal,
}

impl ErrorKind {
    /// Stable machine-readable slug used as the problem+json `type` suffix.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            ErrorKind::Invalid => "invalid",
            ErrorKind::Unauthorized => "unauthorized",
            ErrorKind::Forbidden => "forbidden",
            ErrorKind::NotFound => "not-found",
            ErrorKind::Conflict => "conflict",
            ErrorKind::TooLarge => "too-large",
            ErrorKind::RateLimited => "rate-limited",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Provider => "provider",
            ErrorKind::Extract => "extract",
            ErrorKind::Db => "db",
            ErrorKind::Io => "io",
            ErrorKind::Internal => "internal",
        }
    }

    /// The HTTP status this kind maps to.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            ErrorKind::Invalid => 400,
            ErrorKind::Unauthorized => 401,
            ErrorKind::Forbidden => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::TooLarge => 413,
            ErrorKind::RateLimited => 429,
            ErrorKind::Extract => 422,
            ErrorKind::Provider => 502,
            ErrorKind::Timeout => 504,
            ErrorKind::Db | ErrorKind::Io | ErrorKind::Internal => 500,
        }
    }

    /// Whether the detail message is safe to return to the client. Only true for
    /// client-fault (4xx) kinds; server faults keep detail in logs only.
    #[must_use]
    pub const fn detail_is_public(self) -> bool {
        (self.http_status() / 100) == 4
    }
}

/// The workspace error type: a [`ErrorKind`] plus a human-readable message.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    // Ergonomic constructors for the common kinds.
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, m)
    }
    pub fn unauthorized(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unauthorized, m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Forbidden, m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, m)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.slug(), self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|b| b.as_ref() as &(dyn std::error::Error + 'static))
    }
}
