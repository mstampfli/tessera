# tessera-pipeline

The incremental stage workers. Every stage is a job keyed to one document, chunk
batch, or cluster; nothing ever scans the whole corpus. Workers claim jobs with
SKIP LOCKED, heartbeat their lease, and are idempotent by construction, so a
killed and restarted worker converges rather than duplicating work.

## Place in the workspace

- Depends on: `tessera-core`, `tessera-db`, `tessera-extract`, `tessera-providers`.
- Used by: `tessera-mcp`, `tessera-api`, `tessera-server`.

## Layout

- `worker.rs` - `run_pipeline` / `PipelineHandle`: the claim-lease-run-heartbeat
  loop and the stage dispatch.
- `ingest.rs` - `ingest_bytes`: the write path (CAS + document + first job) all
  callers share.
- `stages.rs` - the per-stage handlers (process document, embed, extract
  entities, assign clusters, correlate entities, detect communities).
- `synth.rs` - insight synthesis for a dirty cluster (LLM output parsed strictly
  as data, every citation checked against the in-context chunks).
- `recluster.rs` - the authoritative HDBSCAN density recluster.
- `context.rs` - `PipelineContext`, the shared handles a stage handler needs.

The `KIND_*` job-kind constants (the single source of truth for the stage names)
live in `lib.rs`.
