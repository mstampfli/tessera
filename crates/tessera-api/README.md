# tessera-api

The HTTP API surface and shared application state. Handlers are thin: they
authenticate, validate, call the service layer (ingest, search, ask), and render.
The same service functions back the MCP server, so the human surface and the
agent surface cannot drift.

## Place in the workspace

- Depends on: `tessera-core`, `tessera-db`, `tessera-extract`, `tessera-providers`,
  `tessera-search`, `tessera-pipeline`, `tessera-mcp`.
- Used by: `tessera-server`.
- Must never: put business logic in a handler; a route authenticates and
  delegates.

## Layout

- `lib.rs` - `build_router` wires the routes, middleware, and `AppState` (the
  cheaply cloneable handle: db, config, CAS, embedder, LLM, active space, event
  bus, metrics).
- `auth.rs` - the `AuthContext` extractor and scope checks (deny-by-default).
- `error.rs` - `ApiError`, mapping the core `ErrorKind` to an HTTP status.
- `events.rs` - the SSE `EventBus` fed by pipeline LISTEN/NOTIFY.
- `rate_limit.rs`, `metrics.rs`, `url_guard.rs` - the login limiter, Prometheus
  middleware, and the SSRF-guarded fetch for URL ingestion.
- `routes/` - one module per resource: `auth`, `tokens`, `ingest`, `search`,
  `ask`, `entities`, `clusters`, `insights`, `documents`, `sources`, `jobs`,
  `events`, `health`, `mcp` (the HTTP MCP transport).

`url_guard::fetch` is a safe primitive; see `../../docs/PRIMITIVES.md`.
