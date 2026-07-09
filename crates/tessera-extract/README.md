# tessera-extract

Turns untrusted raw bytes into a title, normalized text chunks, and extracted
entities. Every extractor treats its input as hostile: bounded work, lossy UTF-8
decoding, and no execution of content.

## Place in the workspace

- Depends on: `tessera-core`.
- Used by: `tessera-pipeline`, `tessera-api`, `tessera-server`.
- Must never: trust the client-declared media type, or execute ingested content.

## Layout

- `sniff.rs` - `sniff()` returns a `SniffedType` from magic bytes first, the
  declared type only as a weak hint.
- `extractors.rs` - `normalize()` dispatches on the sniffed type to the
  per-format extractors (plain text, markdown, JSON/NDJSON, CSV, logs, HTML).
- `chunk.rs` - the content-aware chunker producing `PreparedChunk`s. Every chunk
  is built through `PreparedChunk::new`, which applies `clean_text`, so all chunks
  are control-character-clean by construction.
- `text.rs` - `clean_text`, the control-character stripper for decoded content.
- `security.rs` - the security pack: `refang()` (defang normalization) and
  `extract()` returning `EntityMatch`es (IPs, domains, URLs, emails, hashes, CVEs,
  MACs, ASNs), each carrying the canonical value the storage layer dedups on.
- `dates.rs` - event-time parsing for temporal correlation.
- `plugin.rs` - the sandboxed subprocess plugin host (NDJSON `ExtractEvent` over
  stdio) for out-of-language extractors.

`refang`, the canonicalization, and `clean_text` are safe primitives; see
`../../docs/PRIMITIVES.md`.
