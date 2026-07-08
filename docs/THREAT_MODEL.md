# Threat model (STRIDE)

This is the living threat model for tessera. It is updated when a feature ships,
a trust boundary moves, or the architecture changes.

## Scope and trust boundaries

tessera is a single-user (plus API tokens for programs/agents) self-hosted
service. The data-flow crosses these trust boundaries:

1. Internet to the reverse proxy (Caddy, TLS termination).
2. Tailnet to the app (`tesserad`).
3. Ingested bytes to the extractors (the widest boundary: arbitrary,
   attacker-controlled content).
4. The app to an outbound URL fetch (SSRF surface, from M2).
5. The app to LLM providers (Ollama, the `claude` CLI), and provider output back
   into stored insights and the rendered UI.
6. The app to Postgres.
7. The app to a future extractor subprocess plugin (from M2).

## STRIDE analysis and mitigations

| # | Threat (STRIDE) | Mitigation | Status |
|---|---|---|---|
| 1 | S: forged/stolen API token | 256-bit token secret, stored as blake3 hash, looked up by plaintext prefix then constant-time hash compare; scopes; revocation | done (M0) |
| 2 | S: session hijack | HttpOnly + SameSite=Lax (+ Secure in prod) cookie; server-side session store; logout truly deletes the session row | done (M0) |
| 3 | T: malicious bytes corrupt state | memory-safe Rust parsers, bounded readers, size limits, `clean_text`; parameterized SQL only (no SQL built from data) | M1/M2 |
| 4 | T: LLM output treated as trusted data | insight JSON schema-validated; every citation must resolve to a real in-context chunk id or the insight is rejected; output never executed | M3 |
| 5 | R: silent data mutation | append-only `audit_log` (principal, action, target, time) for login, token create/revoke, ingest, deletes | done (M0), extended per feature |
| 6 | I: knowledge base content leaks publicly | app binds to the tailnet interface by default; TLS via Caddy; public exposure is an explicit opt-in | done (config) |
| 7 | I: sensitive source sent to an external LLM | per-source processing policy enforced at the single provider-registry choke point | M3 |
| 8 | I: token/secret leakage | only hashes stored at rest; token secret shown once; secrets zeroized in memory with redacted Debug; secrets from env, not committed files; gitleaks in CI | done (M0) |
| 9 | D: bulk ingest resource exhaustion | request body limits, per-token rate limits, bounded job queue with backpressure, provider timeouts + circuit breakers | M1 (limits partial in M0) |
| 10 | D: pgvector index maintenance starves queries | HNSW incremental inserts (no full rebuild), bounded maintenance memory, separate worker connection pool | M1 |
| 11 | E: prompt injection escalates via the claude CLI agent | the CLI is invoked as a pure text function: tools disabled, empty scratch cwd, no env secrets passed; all LLM I/O is treated as data | M3 (highest residual risk) |
| 12 | E: stored XSS via ingested/LLM content in the UI | React default escaping (no raw HTML injection of content), strict CSP, content rendered as text | M1 (frontend) |
| 13 | I/E: SSRF via URL-fetch ingestion | `UrlGuard`: deny non-http(s), resolve then pin to the resolved IP, deny private/link-local/tailnet CIDRs, re-check on redirect, size and time caps | M2 |
| 14 | E: extractor plugin compromise | subprocess with no network, rlimits (cpu/mem/fsize=0), empty cwd, cleared env, stdin/stdout contract only | M2 |
| 15 | E: scope escalation | deny-by-default auth layer, per-route scope check, admin scope required for token management | done (M0) |

## Accepted risks (explicit)

- Local Ollama and Postgres are trusted localhost/tailnet services on a
  single-user host.
- The MCP stdio transport inherits the local user's trust (it already holds the
  DB credentials); no additional auth is layered on stdio.
- Model files pulled from the Ollama/HuggingFace registries are trusted after a
  checksum pin.

## Known gaps / deferred controls

- Login rate limiting (threat #1, brute force against the password) is not yet
  enforced. It is deferred to M1, when the full middleware stack (rate limiting
  via `tower_governor`, request ids, per-token quotas) is built in one place
  rather than bolted onto individual M0 handlers. Until then, the strong argon2id
  cost parameters raise the per-attempt cost, but a network rate limit is the
  proper control and is tracked as an M1 deliverable.
