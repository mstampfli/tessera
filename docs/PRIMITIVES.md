# Safe primitives

Recurring concerns (hashing, secret handling, bounded I/O, canonicalization) are
built once here as named, safe-by-construction, tested primitives and reused
everywhere. Reach for the primitive; the hand-rolled equivalent is
review-rejected once the primitive exists. Anchors are names, so grep them.

## Identity and hashing

### `ContentHash` (tessera-core, `hash.rs`)
A blake3 hash of a byte slice. It is both the content-addressed storage key and
the ingestion idempotency key, so identical bytes ingested twice converge on one
object. `of`, `from_slice`, `as_bytes`, `to_hex`.
Instead of: hashing ad hoc, or keying stored content by filename or a fresh uuid.

### `new_id()` (tessera-core, `ids.rs`)
A uuidv7: time-ordered, so ids sort by creation time and support keyset
pagination without a separate timestamp column.
Instead of: uuidv4 (random, fragments the index) or a database serial.

## Secrets and authentication (tessera-core, `secret.rs`)

### `Secret32`
A 256-bit secret with a redacted `Debug` (it cannot be logged by accident) and
`hash()` returning its blake3 digest for storage.
Instead of: holding secret material in a `String`.

### `generate_api_token` / `parse_api_token`
The `tessera_<prefix>_<secret>` token format. The prefix is stored in the clear
for an O(1) indexed lookup; only `blake3(secret)` is stored and compared. Returns
`NewApiToken` (shown once) and `PresentedToken`.
Instead of: storing the raw token, or scanning the table to find a token.

### `hashes_equal`
Constant-time comparison (via `subtle`). The only permitted way to compare a
token or cookie hash.
Instead of: `a == b` on secret material, which leaks a timing oracle.

### `generate_session` / `hash_session_cookie`
An opaque 32-byte session secret, blake3 at rest, 30-day sliding.
Instead of: a JWT or any guessable session identifier.

### `hash_password` / `verify_password`
argon2id with OWASP parameters.
Instead of: any fast hash for passwords.

## Content-addressed store (tessera-db, `cas.rs`)

### `CasStore::write_bytes` -> `(ContentHash, len)`
Hashes the bytes, writes to a unique temp file, and atomically renames into the
hash-sharded path. A concurrent writer of the same content is a safe no-op.
Instead of: writing ingested bytes to a chosen path.

### `CasStore::read_verified`
Re-hashes the bytes on read and hard-errors if they no longer match the key
(on-disk corruption or tampering).
Instead of: reading the object and trusting it.

## Job queue (tessera-db, `queue.rs`)

`enqueue` (transactional with the data write, with a dedupe key), `claim`
(SKIP LOCKED plus a lease), `heartbeat`, `complete`, `fail` (with backoff),
`reap_expired` (recovers a crashed worker's job), `depth`. Handlers are
idempotent by construction, so at-least-once delivery is safe: a killed and
restarted worker converges rather than duplicating work.
Instead of: a second queue mechanism, or a claim with no lease (a crashed worker
would strand the job forever).

## Untrusted input (tessera-extract)

### `sniff` -> `SniffedType` (`sniff.rs`)
Content type from magic bytes first; the client-declared media type is only a
weak hint, never trusted.
Instead of: dispatching on the request's declared content type.

### `refang` (`security.rs`)
Normalizes defanged indicators (`hxxp`, `[.]`, `(dot)`) before matching, so
`1[.]2[.]3[.]4` and `hxxps://evil[.]com` are caught.
Instead of: matching raw text and silently missing defanged IOCs.

### `extract` -> `EntityMatch` (`security.rs`)
Each match carries the canonical value (one normalization per kind: a lowercased
registrable domain via the public-suffix list, an uppercased CVE, a
colon-delimited lowercased MAC) that the storage layer dedups on.
Instead of: canonicalizing an entity value at each call site, which drifts.

## SSRF-guarded fetch (tessera-api, `url_guard.rs`)

### `url_guard::fetch(url)` -> `Fetched`
http(s) only; resolves the host and rejects if any resolved address is private,
loopback, link-local, or in the tailnet CGNAT range; re-validates on every
redirect hop; caps response size and time.
Instead of: building an ad-hoc reqwest client for a user-supplied URL.
Residual risk (documented in the module): a TOCTOU window between the DNS check
and the connection, since the connection is not yet pinned to the checked IP;
resolve-then-pin is the planned follow-up. The private-range denial still blocks
the common SSRF targets.

## Errors (tessera-core, `error.rs`)

`Error` / `ErrorKind` / `Result` are the one taxonomy every crate returns.
`tessera_db::map_sqlx` classifies database errors (a unique violation becomes
`Conflict`, a missing row `NotFound`); `tessera_api::error::ApiError` maps a kind
to its HTTP status.
Instead of: `anyhow`/stringly-typed errors across crate boundaries, or ad-hoc
status mapping in a handler.

## LLM JSON (tessera-providers, `json.rs`)

### `generate_json<T>` -> `(T, model)`
The one way to get a typed JSON value out of a model. It owns the fragile
mechanics every caller would otherwise re-derive: models routinely wrap the
object in prose or ``` fences, so it slices the outermost `{...}` before
deserializing, and it retries once (with a corrective instruction appended to the
system prompt) if the first reply does not parse. Only a parse failure is
retried; a provider/transport error propagates immediately, because backend
failover is the chain's job. Returns the value plus the model id that produced it.
Instead of: hand-rolling the extract-JSON-from-prose and retry-once loop at each
call site.
It is NOT a schema validator: the caller keeps its own prompt (which describes the
fields) and its own semantic validation of the parsed values (enum membership,
numeric ranges).

## Enforced by convention (not a dedicated type)

Some concerns are held by convention rather than a wrapper type. They work, but
they are patterns to follow by hand, not types that make the illegal state
unrepresentable:

- Keyset pagination on the uuidv7 id (the `sources` and `documents` repos):
  newest-first, seek by id, never `OFFSET`.
- Undirected edges stored once with `src < dst` (`entity_edges`), upserting the
  weight on conflict.
- Per-space vector dimensionality recorded in `embedding_spaces` and applied via
  the `halfvec(dim)` cast in the index and in every query, so storage and search
  share one dimension; a model swap registers a new space rather than mutating
  the column.

## Known gaps vs the plan (findings, to be reconciled)

Writing this catalog was an audit. The design plan called for these as named,
tested primitives; the current code inlines or skips them. Recorded here so they
are not mistaken for done, and tracked for a follow-up pass:

1. **Ingestion buffers rather than streams.** `CasStore::write_bytes` takes a full
   `&[u8]`, and multipart upload reads the whole field with `field.bytes()`,
   whereas the plan specified streaming into the CAS while hashing. It is bounded
   by the 64 MiB global body cap, so it is safe, just not the streaming design.
2. **No `clean_text` or bounded-reader primitives, and no write-side dim check.**
   Text cleaning is inline `from_utf8_lossy`, bounding is ad-hoc `.take(N)`, and
   `embeddings::insert_batch` accepts vectors without checking their length
   against `space.dim` (a wrong-dimension vector fails later at index or query
   time, not at the write).

(The plan's third gap, a shared `generate_json<T>` LLM-JSON helper, is now built;
see the LLM JSON section above. It landed as a lean typed-parse-plus-retry
primitive over the caller's own prompt, rather than the plan's schema-in-prompt
design, because a hand-tuned domain prompt guides a small local model better than
a rendered JSON schema would.)
