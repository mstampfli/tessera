# tessera

A self-hosted knowledge base and correlation engine. Feed it bulk data of any
kind; dedicated models embed it, extract entities, find correlations, group
related material, and surface actionable, evidence-cited insights. Built to be
driven by both people (an intuitive web UI) and programs (a REST API and an MCP
server for AI agents).

A tessera is a single tile of a mosaic. The engine assembles many small,
disconnected tiles of data into a picture you can act on.

> Status: early. Milestone M0 (foundation) is in place: the workspace, the full
> database schema, configuration, the auth model (API tokens and sessions), and
> the operator CLI. Ingestion and search land in M1. See the roadmap below.

## Why

Most tools stop at storing and searching data; the correlation, the "what
connects to what", and the "so what should I do" are left to a human staring at
a database browser. tessera does that work: it treats ingestion as cheap, does
the heavy analysis incrementally in the background, and presents ranked
conclusions with the evidence attached to every claim.

The engine is general purpose. The flagship showcase is security and OSINT
(indicators, logs, threat feeds, CVEs), where "actionable" is concrete: block
this address, hunt this hash, patch this CVE, all cited back to the source.

## Architecture

One Rust binary, `tesserad`, runs the HTTP API, the MCP server, and the pipeline
workers on a single async runtime. Postgres (with pgvector) is the only stateful
service: it holds vectors, full-text indexes, the job queue, the entity graph,
and sessions. Raw ingested bytes live in an on-disk content-addressed store keyed
by their hash, which is also the idempotency key.

Every model call (embed, extract entities, synthesize insights) goes through a
pluggable provider layer, so the implementation (in-process ONNX, a local Ollama
server, the `claude` CLI, a future remote API) is a swap point, not a rewrite.

The pipeline is incremental by construction: every stage is a job keyed to one
document, chunk batch, or cluster. Nothing ever rescans the whole corpus.

```
crates/
  tessera-core        domain types, ids, error taxonomy, secret primitives, config
  tessera-db          connection pools, migrations, repositories, the job queue
  tessera-extract     content sniffing, chunking, extractors, the plugin host
  tessera-providers   embedding + generation provider traits and implementations
  tessera-pipeline    the incremental stage workers
  tessera-search      hybrid retrieval and ask-with-citations
  tessera-api         the axum HTTP API and auth
  tessera-mcp         the MCP server for AI agents
  tessera-server      tesserad: the binary and operator CLI
```

## Quickstart (development)

Requires a recent Rust toolchain and Docker.

```sh
# 1. Start Postgres (pgvector).
docker compose up -d

# 2. Point the app at it and apply migrations.
export DATABASE_URL=postgres://tessera:tessera@127.0.0.1:5432/tessera
cargo run --bin tesserad -- migrate

# 3. Create the primary user and an API token.
export TESSERA_ADMIN_PASSWORD='choose-a-strong-password'
cargo run --bin tesserad -- user create --user marc
cargo run --bin tesserad -- token new --user marc --name laptop --scopes read,ingest,admin

# 4. Run the server.
cargo run --bin tesserad -- serve

# 5. Verify auth end to end.
curl -s -H "Authorization: Bearer <token from step 3>" \
  http://127.0.0.1:8080/v1/auth/me
```

`tesserad doctor` prints the resolved configuration and checks that Postgres and
the content store are reachable.

## Security posture

- The Rust core owns all authentication. API tokens are 256-bit secrets stored
  only as a hash; the user password is argon2id. Authorization is deny-by-default
  and scoped.
- All ingested bytes are treated as untrusted: memory-safe parsers, bounded
  readers, and size limits. Model output is treated as data, never executed, and
  every insight citation must resolve to a real source chunk.
- Deployed on a private tailnet behind a TLS reverse proxy; the service binds to
  the tailnet interface, never a public one, by default.

See `docs/THREAT_MODEL.md` for the STRIDE analysis.

## Roadmap

- M0 foundation: workspace, schema, config, auth, CLI. (done)
- M1 ingest and search: content-addressed ingestion, extractors, the job queue,
  embeddings, hybrid search, ask-with-citations, and the web UI shell.
- M2 entities and correlation: the security extractor pack, the entity graph,
  and the correlation edges.
- M3 clustering and insights: incremental clustering and cited insight cards.
- M4 agents and deployment: the MCP server and the production deployment.

## License

Dual-licensed under either of Apache License, Version 2.0 or the MIT license at
your option.
