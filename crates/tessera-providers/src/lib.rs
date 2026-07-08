//! The pluggable AI provider layer.
//!
//! Every model touchpoint in tessera goes through one of two capability traits
//! so that the implementation (in-process ONNX, Ollama over HTTP, the `claude`
//! CLI as a subprocess, a future remote API) is a swap point, not a rewrite.
//! Capabilities are split rather than merged into one fat trait: a CLI reasoner
//! cannot embed and an ONNX embedder cannot generate, so a single trait would
//! force dishonest `unimplemented!()` holes.
//!
//! The seams and their health/error types live here; concrete backends live in
//! the submodules and are constructed via [`build`].

pub mod build;
pub mod chain;
pub mod claude_cli;
#[cfg(feature = "fastembed")]
pub mod fastembed_embedder;
pub mod ollama;

pub use build::{build_embedder, build_llm};

use async_trait::async_trait;

/// Liveness of a provider, surfaced by `/readyz` and `tesserad doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderHealth {
    /// Reachable and ready.
    Up,
    /// Reachable but degraded (e.g. rate-limited), with a reason.
    Degraded(String),
    /// Unreachable, with a reason.
    Down(String),
}

/// A provider call failure. Kept distinct from the core error taxonomy so the
/// chain/circuit-breaker logic can reason about retryability.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("provider is unavailable: {0}")]
    Unavailable(String),
    #[error("provider returned invalid output: {0}")]
    InvalidOutput(String),
    #[error("provider call failed: {0}")]
    Other(String),
}

/// Static description of an embedding vector space, so a stored vector always
/// records which model and dimensionality produced it (enables model swaps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpaceInfo {
    /// Stable space name, e.g. `bge-small-en-v1.5`.
    pub name: String,
    /// Vector dimensionality.
    pub dim: usize,
    /// Distance metric, e.g. `cosine`.
    pub metric: &'static str,
}

/// Whether text is being embedded as a stored document or as a live query; some
/// models require different instruction prefixes for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Document,
    Query,
}

/// Produces embedding vectors for text.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// The space this provider writes into.
    fn space(&self) -> &EmbeddingSpaceInfo;

    /// Embed a batch of texts, returning one vector per input in order.
    async fn embed(
        &self,
        texts: &[String],
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, ProviderError>;

    /// Cheap liveness probe.
    async fn health(&self) -> ProviderHealth;
}

/// A single text-generation request.
#[derive(Debug, Clone)]
pub struct GenRequest {
    /// The rendered prompt. All corpus-derived content in here is untrusted
    /// data; callers must treat the response as data too (validate, never
    /// execute).
    pub prompt: String,
    /// Optional system instruction.
    pub system: Option<String>,
    /// Soft cap on output tokens, if the provider supports it.
    pub max_tokens: Option<u32>,
}

/// A completed generation.
#[derive(Debug, Clone)]
pub struct GenResponse {
    pub text: String,
    /// The concrete model id that served the request, for provenance.
    pub model: String,
}

/// Generates text (summaries, insight synthesis, RAG answers).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable id of this provider, e.g. `claude_cli` or `ollama`.
    fn id(&self) -> &'static str;

    /// Generate a completion.
    async fn generate(&self, req: &GenRequest) -> Result<GenResponse, ProviderError>;

    /// Cheap liveness probe.
    async fn health(&self) -> ProviderHealth;
}
