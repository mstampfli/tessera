//! Hybrid retrieval and ask-with-citations.
//!
//! Retrieval fuses three signals in one SQL statement: pgvector kNN (semantic),
//! Postgres full-text (keyword; essential because IOCs and hashes do not embed
//! meaningfully), and entity-exact matches, combined with Reciprocal Rank
//! Fusion. Answers cite specific chunks and never make an uncited claim.
//!
//! The service lands in M1; M0 is a placeholder so the workspace DAG is
//! complete.

/// Placeholder marker retained until the search service lands in M1.
#[must_use]
pub const fn planned() -> &'static str {
    "hybrid search lands in M1"
}
