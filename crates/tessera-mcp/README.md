# tessera-mcp

The MCP server: tessera as an agent-drivable tool. It exposes ingest, search,
ask, list-insights, entity-neighborhood, and job-status as MCP tools, each a thin
delegate to the same service layer the REST API uses, so the two surfaces cannot
drift.

The JSON-RPC 2.0 protocol is implemented directly rather than through an SDK, so
the server is dependency-light and fully under our control.

## Place in the workspace

- Depends on: `tessera-core`, `tessera-db`, `tessera-providers`, `tessera-search`,
  `tessera-pipeline`.
- Used by: `tessera-api` (the HTTP `/mcp` transport), `tessera-server` (the stdio
  transport).

## Layout

- `lib.rs` - `McpState` (the service handles), `run_stdio` (the line-delimited
  JSON-RPC loop for `tesserad mcp-stdio`), and `dispatch_request` (one request,
  reused by the HTTP transport in `tessera-api`).
- `tools.rs` - the tool `definitions()` and `call()` dispatch.
