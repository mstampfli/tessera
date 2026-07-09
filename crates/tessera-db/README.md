# tessera-db

The database layer: connection pools, embedded migrations, the content-addressed
store, the job queue, and the typed repositories every higher crate reads and
writes through. No business logic lives here, only persistence.

## Place in the workspace

- Depends on: `tessera-core`.
- Used by: `tessera-pipeline`, `tessera-search`, `tessera-mcp`, `tessera-api`,
  `tessera-server`.

## Layout

- `lib.rs` - the `Db` handle (two pools: `api` for interactive requests, `worker`
  for background jobs, so bulk work cannot starve the API), `migrate`, the
  `listen` LISTEN/NOTIFY bridge, and `map_sqlx` (classifies a unique violation as
  a `Conflict`, a missing row as `NotFound`).
- `cas.rs` - `CasStore`: writes raw bytes under their blake3 hash (in memory via
  `write_bytes`, or streamed and size-capped via `write_streaming`) and
  re-verifies the hash on read (`read_verified`).
- `queue.rs` - the Postgres job queue: `enqueue` (transactional with the data
  write), `claim` (SKIP LOCKED + lease), `heartbeat`, `complete`, `fail` (with
  backoff), `reap_expired`, `depth`, `notify`.
- `louvain.rs` - weighted Louvain community detection over the entity graph.
- `repos/` - one module per aggregate: `users`, `sessions`, `api_tokens`,
  `sources`, `documents`, `chunks`, `embeddings`, `entities`, `communities`,
  `clusters`, `insights`, `ask_history`, `audit`.

The CAS and queue are safe primitives; see `../../docs/PRIMITIVES.md`.
