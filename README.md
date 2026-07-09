# tessera

A self-hosted knowledge base and correlation engine. Feed it bulk data of any
kind; dedicated models embed it, extract entities, find correlations, group
related material, and surface actionable, evidence-cited insights. Built to be
driven by both people (an intuitive web UI) and programs (a REST API and an MCP
server for AI agents).

A tessera is a single tile of a mosaic. The engine assembles many small,
disconnected tiles of data into a picture you can act on.

> Status: the planned build (M0 through M4) is complete: ingestion, hybrid
> search, ask-with-citations, the security entity graph and correlation edges,
> incremental clustering, cited insight cards, the web UI, and the MCP server all
> work end to end, and the stack is deployable behind a TLS reverse proxy on a
> private tailnet. See the roadmap below.

## Why

Most tools stop at storing and searching data; the correlation, the "what
connects to what", and the "so what should I do" are left to a human staring at
a database browser. tessera does that work: it treats ingestion as cheap, does
the heavy analysis incrementally in the background, and presents ranked
conclusions with the evidence attached to every claim.

The engine is general purpose. The flagship showcase is security and OSINT
(indicators, logs, threat feeds, CVEs), where "actionable" is concrete: block
this address, hunt this hash, patch this CVE, all cited back to the source.
`docs/OSINT_TOOLS.md` catalogs the OSINT sources that feed and corroborate this
domain, and is honest about the categories with no strong free tool.

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

Each crate has a README stating its job and place in the DAG.
`docs/ARCHITECTURE.md` records the load-bearing invariants, `docs/PRIMITIVES.md`
catalogs the safe primitives to reuse, and `CONTRIBUTING.md` has the build, test,
and dev-loop recipe.

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

`tesserad doctor` prints the resolved configuration and checks that Postgres,
the content store, and the model providers are reachable.

Ingestion, search, and ask over the API:

```sh
# Ingest a document (programs use a bearer token; the web UI uses a session).
curl -s -H "Authorization: Bearer <token>" -H 'Content-Type: application/json' \
  -d '{"content":"...","media_type":"text/markdown","title":"report"}' \
  http://127.0.0.1:8080/v1/ingest

# Search (hybrid vector + keyword) and ask with citations.
curl -s -H "Authorization: Bearer <token>" \
  'http://127.0.0.1:8080/v1/search?q=your+query&mode=hybrid'
curl -s -H "Authorization: Bearer <token>" -H 'Content-Type: application/json' \
  -d '{"question":"..."}' http://127.0.0.1:8080/v1/ask
```

### Web UI

The `web/` directory is a Next.js app that talks to the core over a same-origin
`/api` proxy. With the core running (step 4):

```sh
cd web
npm install
npm run dev   # http://localhost:3000, proxies /api to the core on :8080
```

Sign in with the user from step 3, then ingest by dropping files or pasting data,
watch the pipeline progress live, and search or ask over what you have ingested.
Entity and cluster pages lead with a correlation table and offer an opt-in
network graph of the same data, so the campaign structure and the bridge
entities that link separate incidents are visible at a glance.

### For AI agents (MCP)

tessera speaks the Model Context Protocol, so an agent can ingest, search, ask
with citations, list insights, and walk the correlation graph directly. Connect a
local agent over stdio:

```sh
claude mcp add tessera -- tesserad mcp-stdio
```

Remote agents on the tailnet use the same tools over HTTP: a JSON-RPC 2.0
endpoint at `POST /mcp`, authenticated with a bearer token carrying the `mcp`
scope.

Tools: `tessera_ingest`, `tessera_search`, `tessera_ask`,
`tessera_list_insights`, `tessera_get_entity_neighborhood`, `tessera_job_status`.
Each is a thin delegate to the same service layer the REST API and web UI use.

## Deployment

The production stack (Postgres, the backend, and the frontend) runs from
`docker-compose.prod.yml`, behind a TLS reverse proxy on a private tailnet. The
database publishes no host port; the services bind to the host's tailnet IP.

```sh
cp .env.prod.example .env   # set POSTGRES_PASSWORD and APP_BIND
docker compose -f docker-compose.prod.yml up -d --build
```

`deploy/Caddyfile.snippet` routes `/api/*` to the backend and everything else to
the frontend. On a CPU-only host with no Ollama (for example a small VPS), build
the backend with in-process embeddings: `BACKEND_FEATURES=fastembed` and
`EMBEDDER=fastembed` in `.env`.

The backend exposes Prometheus metrics at `/metrics` (HTTP request rates and
latencies, per-stage job counts and durations, and the queue depth) for a
tailnet scraper.

Back up and restore the database and content store:

```sh
scripts/backup.sh                 # writes ./backups/<timestamp>/
scripts/restore.sh backups/<timestamp>
```

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

## Contributing

`CONTRIBUTING.md` has the dev loop, the CI gates, and the conventions for adding
an endpoint, extractor, provider, or pipeline stage. In short: `just db-up`,
`just migrate`, `just serve`, and `just check` before every commit.

## Roadmap

- M0 foundation: workspace, schema, config, auth, CLI. (done)
- M1 ingest and search: content-addressed ingestion, extractors, the job queue,
  embeddings, hybrid search, ask-with-citations, and the web UI. (done)
- M2 entities and correlation: the security extractor pack, the entity graph,
  the correlation edges and neighborhood, URL ingestion with an SSRF guard, and
  the sandboxed extractor plugin host. (done)
- M3 clustering and insights: incremental clustering, cited insight cards, and
  the live triage feed. (done)
- M4 agents and deployment: the MCP server and the production stack. (done)

## License

Dual-licensed under either of Apache License, Version 2.0 or the MIT license at
your option.
