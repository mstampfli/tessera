# Contributing to tessera

This is the how-to-work-in-it guide. For what tessera is, read `README.md`; for
how it works (the codemap, the dependency DAG, and the invariants), read
`docs/ARCHITECTURE.md`; for the safe primitives to reuse instead of hand-rolling,
read `docs/PRIMITIVES.md`.

## Prerequisites

- A recent stable Rust toolchain (edition 2021, minimum 1.95) with `rustfmt` and
  `clippy`.
- Docker, for the pgvector Postgres the app and the tests run against.
- Node 20, for the `web/` frontend.
- Optional: a local Ollama for the model providers. Without it, build the backend
  with in-process ONNX embeddings (`--features fastembed`) or point the providers
  at a remote endpoint. `tesserad doctor` reports what is reachable.

## Repository layout

- `crates/` - the Rust workspace (see `docs/ARCHITECTURE.md` for the DAG). Each
  crate has its own README stating its job and place in the DAG.
- `web/` - the Next.js frontend (a pure client over the core's `/api`).
- `docs/` - architecture, threat model, primitives, and this domain's tool notes.
- `scripts/`, `deploy/`, `plugins/` - backup/restore and seed scripts, the deploy
  snippets, and example extractor plugins.

## The dev loop

The `justfile` is the task runner (`just` lists the targets). It pins the real
Docker socket and a default `DATABASE_URL`.

```sh
just db-up      # start the pgvector Postgres (waits until it is ready)
just migrate    # apply migrations
just serve      # run the API + pipeline workers (cargo run --bin tesserad -- serve)
just db-down    # stop Postgres (keeps the volume)
```

Default connection string (override by exporting your own):
`postgres://tessera:tessera@127.0.0.1:5432/tessera`.

First run also needs a user and a token (see the README quickstart): `tesserad
user create` then `tesserad token new`.

### Inner loop vs the full gate

- Fast, while editing: `cargo check -p <crate>` and `cargo clippy -p <crate>`.
- Before every commit or PR, run the full local gate, which mirrors CI exactly:

```sh
just check      # cargo fmt --all --check
                # cargo clippy --workspace --all-targets -- -D warnings
                # cargo test --workspace
```

`cargo test --workspace` talks to a real Postgres, so `just db-up` must have run
first. `just fmt` formats the whole workspace.

If you touched `web/`, also run its checks:

```sh
cd web && npm ci
npm run typecheck
npx next lint --dir src
npm run build
```

## What CI enforces (all must be green)

`.github/workflows/ci.yml` runs, and breaks the build on failure (`RUSTFLAGS`
is `-D warnings`):

- **rust** - `fmt --check`, `clippy -D warnings`, and `cargo test --workspace`
  against a `pgvector/pgvector:pg17` service container.
- **web** - `typecheck`, `next lint`, `build`, and `npm audit --audit-level=high`.
- **supply-chain** - `cargo-deny check advisories licenses bans sources`.
- **secrets** - `gitleaks` over the full history.
- **sbom** - CycloneDX SBOMs for the Rust workspace and the web app.

## Conventions

### Respect the crate DAG

Dependencies point one way (`docs/ARCHITECTURE.md`). Do not add an upward or
cyclic dependency. `tessera-core` performs no I/O. `unsafe_code` is forbidden
workspace-wide; the one documented exception is the plugin host's `pre_exec`.

### Reuse the primitives

Before hand-rolling a content hash, a secret comparison, a bounded read, a CAS
write, a queue interaction, or a URL fetch, reach for the named primitive in
`docs/PRIMITIVES.md`. Hand-rolled equivalents are review-rejected once the
primitive exists.

### To add a typical thing

- **An HTTP endpoint:** a handler in `crates/tessera-api/src/routes/<resource>.rs`,
  wired in `routes/mod.rs`. Keep it thin: authenticate, validate, call a service
  function, render. If agents should reach it too, add a delegating tool in
  `crates/tessera-mcp/src/tools.rs` so the two surfaces cannot drift.
- **An extractor:** implement it in `crates/tessera-extract/src/extractors.rs`,
  register it, and add a detection rule in `sniff.rs`. Treat input as hostile.
- **A model provider:** implement `EmbeddingProvider` or `LlmProvider` in
  `crates/tessera-providers`, then construct it in `build.rs`.
- **A pipeline stage:** add a `KIND_*` constant in `tessera-pipeline/src/lib.rs`,
  a handler in `stages.rs`, and dispatch in `worker.rs`. Keep the handler
  idempotent so at-least-once delivery is safe.
- **A schema change:** a new numbered migration in
  `crates/tessera-db/migrations/`, plus the repo function in `repos/`. Migrations
  are embedded and applied by `tesserad migrate`.

### Commits and docs

- Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`), imperative subject, small and focused, plain ASCII.
- When a change alters structure, a flow, an invariant, or the dev loop, update
  the affected doc (the crate README, `docs/ARCHITECTURE.md`, or
  `docs/PRIMITIVES.md`) in the same commit. A stale map is worse than none.

## Security

Secrets live only in a gitignored `.env`; never commit them (gitleaks gates
this). Ingested bytes and model output are untrusted by construction: parse
defensively, bind every SQL parameter, and never execute model output. See
`docs/THREAT_MODEL.md`.
