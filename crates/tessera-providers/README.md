# tessera-providers

The pluggable AI provider layer. Every model touchpoint (embed, generate) goes
through one of two capability traits, so the concrete backend (in-process ONNX,
Ollama over HTTP, the `claude` CLI, a future remote API) is a swap point, not a
rewrite.

Capabilities are split into two traits rather than one fat trait: a CLI reasoner
cannot embed and an ONNX embedder cannot generate, so a single trait would force
dishonest `unimplemented!()` holes.

## Place in the workspace

- Depends on: `tessera-core`.
- Used by: `tessera-pipeline`, `tessera-search`, `tessera-mcp`, `tessera-api`,
  `tessera-server`.

## Layout

- `lib.rs` - the `EmbeddingProvider` and `LlmProvider` traits plus the shared
  `ProviderHealth`, `ProviderError`, and `EmbeddingSpaceInfo` types.
- `build.rs` - `build_embedder` / `build_llm` construct the configured backend.
- `chain.rs` - `ChainedLlm`, itself an `LlmProvider`, with per-provider timeout
  and a circuit breaker (fall to the next backend on failure).
- `claude_cli.rs` - the `claude` CLI as a subprocess LLM (prompt on stdin, tools
  disabled, empty cwd, no env secrets).
- `ollama.rs` - the Ollama HTTP embedder and LLM.
- `fastembed_embedder.rs` - in-process ONNX embeddings, behind the `fastembed`
  feature (it pulls a large native tree, so the default build uses Ollama).

## Features

- `fastembed` - enable the in-process ONNX embedder. Off by default.
