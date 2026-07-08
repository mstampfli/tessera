# Architecture

tessera is one Rust binary (`tesserad`) that runs the HTTP API, the MCP server,
and the pipeline workers on a single async runtime, backed by Postgres (with
pgvector) as the only stateful service. This document records the load-bearing
design invariants: the rules the implementation must not drift from.

## Invariant 1: algorithms decide what correlates; the LLM only explains it

This is the core of how tessera finds "actionable" information, and it is a hard
rule, not a preference.

An LLM is a conservative correlator. Asked "are these two things related?" it
hedges, because it is trained not to assert what it cannot verify. That is the
wrong tool for discovery and the right tool for description. So tessera never
asks a model whether things correlate. Correlation is decided by cheap,
deterministic, math-based methods that produce a score you threshold and that
never refuse to answer:

- Shared-entity co-occurrence, idf-weighted. Two documents that share a rare hash
  or IP are correlated by arithmetic, not opinion. This is the highest-signal
  method in the security domain and costs almost nothing.
- Embedding kNN over pgvector. Semantic nearness is a cosine number, not a
  judgment.
- Density clustering (HDBSCAN) over the embedding vectors. Grouping by geometry.
- Temporal co-occurrence and graph community detection (Louvain). Pure
  computation over the entity graph.

None of these have epistemic caution, so they surface links a cautious model
would never volunteer. They are also GPU-optional and scale.

The LLM earns its keep strictly downstream, on material the math already grouped:

- Naming and labeling a cluster the algorithm found.
- Narrating why a group hangs together.
- Suggesting next actions ("block this, hunt that").

The prompt is never "is this correlated?" It is always "here is a group of things
that co-occur or cluster together; write the actionable story." That sidesteps
the hedging entirely, and it is both cheaper and better at discovery, because
vector geometry has no epistemic caution.

### The leash: no uncited claims (machine-enforced)

Because the model describes rather than discovers, it must not be allowed to
invent a correlation the math did not find. Every claim in a synthesized insight
must carry a citation marker, and every marker must resolve to a real source
chunk id that was actually in the model's context. The synthesis stage validates
this after generation: any insight containing a citation that does not resolve is
rejected and regenerated. This keeps the model a describer, never a fabricator.
See the synthesis stage (M3) and `docs/THREAT_MODEL.md` threat #4.

## Invariant 2: one shared service layer, never two paths

Every capability (search, ask, ingest, entity lookup) is implemented once in the
service layer. The REST handler and the MCP tool are both thin delegates to the
same function, so the human surface and the agent surface can never diverge in
behavior.

## Invariant 3: everything incremental, nothing rescans the corpus

Every pipeline stage is a job keyed to one document, chunk batch, or cluster. New
data assigns to existing clusters online (nearest centroid); reclustering is
bounded to genuinely new material; a rare full recluster preserves cluster
identity by member overlap so insights do not churn. No stage ever iterates the
whole corpus.

## Invariant 4: all ingested bytes and all model output are untrusted data

Ingested bytes are parsed only by memory-safe, bounded, size-capped code. Model
output is parsed as data (schema-validated), never executed, never interpolated
into SQL or a shell. The `claude` CLI provider is invoked as a pure text function
with its tools disabled.

## Invariant 5: the provider layer is the only place models are called

Embedding, extraction, and synthesis go through capability traits
(`EmbeddingProvider`, `LlmProvider`). Concrete backends (in-process ONNX, Ollama,
the `claude` CLI, future remote APIs) sit behind them with health checks and
fallback chains. Swapping a model is a config change, and per-source data-handling
policy is enforced at this single choke point.

## Data flow

```
ingest bytes -> content-addressed store (blake3) + documents row
             -> classify + chunk
             -> embed (provider)              -> chunk_embeddings (pgvector)
             -> extract entities              -> entities + mentions
  [ALGORITHMS decide correlation]
             -> co-occurrence / kNN / temporal -> entity_edges (scored)
             -> online cluster assign          -> clusters + cluster_members
  [LLM explains what algorithms grouped, on the citation leash]
             -> synthesize                     -> insights + insight_evidence
```

Retrieval for search and ask fuses pgvector kNN, Postgres full-text, and
entity-exact matches with Reciprocal Rank Fusion; answers cite chunks.
