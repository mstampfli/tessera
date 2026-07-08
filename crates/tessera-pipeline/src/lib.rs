//! Incremental pipeline workers.
//!
//! Every stage (normalize, chunk, embed, extract entities, correlate, cluster,
//! synthesize insights) is a job keyed to a specific document, chunk batch, or
//! cluster. Nothing ever scans the whole corpus. Workers claim jobs from the
//! Postgres queue with `SELECT ... FOR UPDATE SKIP LOCKED`, heartbeat their
//! lease, and are idempotent by construction so at-least-once delivery is safe.
//!
//! Stage handlers land in M1 (normalize/chunk/embed) onward. M0 is a placeholder
//! so the workspace DAG is complete.

/// Placeholder marker retained until the first stage handler lands in M1.
#[must_use]
pub const fn planned() -> &'static str {
    "pipeline stages land in M1"
}
