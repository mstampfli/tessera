# tessera-core

The dependency-light foundation the rest of the workspace builds on: domain ids,
the error taxonomy, secret-handling primitives, the normalized extractor event
format, and the layered configuration loader. It performs no I/O.

Because ids, errors, and secret handling are defined exactly once here, every
crate above shares one definition instead of drifting copies.

## Place in the workspace

- Depends on: nothing internal (this is the DAG root).
- Used by: every other crate.

## Layout

- `error.rs` - the `Error` / `ErrorKind` / `Result` taxonomy every crate returns.
- `ids.rs` - `new_id()`, a time-ordered uuidv7 (sortable, keyset-paginatable).
- `hash.rs` - `ContentHash`, the blake3 content hash used as the CAS key and the
  ingestion idempotency key.
- `secret.rs` - API-token and session-secret generation, blake3-at-rest hashing,
  constant-time comparison, and argon2id password hashing. Secrets have a redacted
  `Debug`, so they cannot be logged by accident.
- `extract_event.rs` - `ExtractEvent`, the one normalized event shape both
  in-process and subprocess extractors emit.
- `config.rs` - the figment-based loader (a `tessera.toml` file overlaid with
  `TESSERA__*` environment variables).

The security-relevant types here are documented in `../../docs/PRIMITIVES.md`.
See the root `README.md` for the whole-system picture.
