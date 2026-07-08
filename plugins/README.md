# Extractor plugins

A plugin is any executable that turns raw bytes into normalized extraction
events. It lets tessera ingest formats the built-in Rust extractors do not
handle, without changing the core.

## Contract (schema `tessera.extract.v1`)

- The plugin reads the raw document bytes on **stdin**.
- It writes **NDJSON** to **stdout**: one JSON object per line, each an extraction
  event. Anything on stderr is ignored.
- It exits 0 on success.

Event shapes (the `event` field selects the kind):

```json
{"event":"meta","title":"optional title","attrs":{}}
{"event":"text","text":"a block of prose","section":"optional heading"}
{"event":"record","data":{"any":"structured object"}}
{"event":"entity","entity_kind":"ip","value":"1.2.3.4","confidence":1.0}
{"event":"warn","message":"a non-fatal problem"}
```

## Sandbox

The host runs every plugin under a tight sandbox, because it processes untrusted
content and may itself be untrusted: a cleared environment, an empty working
directory, its own process group, a CPU limit and wall-clock timeout, a memory
cap, no ability to create or grow files (`RLIMIT_FSIZE = 0`), few file
descriptors, and a hard cap on stdout size. A plugin that spins, floods, or tries
to write files is killed and its document is failed.

See `demo_extractor.py` for a minimal, working example.
