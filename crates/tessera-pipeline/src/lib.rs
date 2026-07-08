//! Incremental pipeline workers.
//!
//! Every stage is a job keyed to one document or chunk batch; nothing scans the
//! corpus. Workers claim jobs with `SKIP LOCKED`, heartbeat their lease, and are
//! idempotent by construction, so at-least-once delivery is safe: a killed and
//! restarted worker converges to the same state rather than duplicating work.

mod context;
mod stages;
mod worker;

pub use context::PipelineContext;
pub use worker::{run_pipeline, PipelineHandle};

/// Job kind: normalize + chunk a freshly ingested document, then fan out
/// embedding and entity-extraction jobs for its chunks.
pub const KIND_PROCESS_DOCUMENT: &str = "process_document";
/// Job kind: embed a batch of chunk ids into the active space.
pub const KIND_EMBED_CHUNKS: &str = "embed_chunks";
/// Job kind: extract security entities from a document's chunks and recompute
/// its correlation edges.
pub const KIND_EXTRACT_ENTITIES: &str = "extract_entities";
