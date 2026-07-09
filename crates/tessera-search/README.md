# tessera-search

Hybrid retrieval and ask-with-citations. Retrieval fuses two signals with
Reciprocal Rank Fusion in a single SQL statement: pgvector kNN (semantic
nearness) and Postgres full-text search (exact keyword match, essential because
identifiers like IPs and hashes do not embed meaningfully).

## Place in the workspace

- Depends on: `tessera-core`, `tessera-db`, `tessera-providers`.
- Used by: `tessera-mcp`, `tessera-api`, `tessera-server`.

## Layout

- `lib.rs` - `search()` and `SearchMode` (Hybrid, Semantic, Keyword), the RRF
  fusion query, and the `SearchHit` shape. The vector distance uses the same
  `halfvec` cast as the per-space HNSW index, so the index is actually used.
- `ask.rs` - `ask()`: retrieve, prompt the LLM, and return an `AskAnswer` whose
  every `Citation` resolves to a real retrieved chunk (no evidence in the base is
  answered as such, never hallucinated).
