# tessera-server

The `tesserad` binary and operator CLI. It wires the configured providers, pools,
content store, and embedding space into the application state, then runs whichever
command was asked for. This is the only crate that depends on all the others.

## Place in the workspace

- Depends on: every other crate.
- Used by: nothing (it is the workspace root of the DAG, the executable).

## Layout

- `main.rs` - the entry point: parse the CLI, load config, set up tracing, run.
- `cli.rs` - the `clap` command definitions and their implementations.

## Commands

- `serve` - run the HTTP API and the pipeline workers on one runtime.
- `mcp-stdio` - run the MCP server over stdio (for local AI agents).
- `migrate` - apply database migrations and exit.
- `doctor` - print the resolved config and check Postgres, the CAS, and the model
  providers are reachable.
- `recorrelate` - backfill entity embeddings and rebuild all global semantic
  correlation edges (run once after enabling correlation on an existing corpus).
- `recluster` - HDBSCAN density recluster of all chunk embeddings.
- `token new|list|revoke`, `user create|...` - operator management of API tokens
  and users.
