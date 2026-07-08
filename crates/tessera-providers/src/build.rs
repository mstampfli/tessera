//! Construct providers from configuration.
//!
//! This is the single place model backends are instantiated, so provider choice
//! and per-source data-handling policy have one choke point (see the security
//! invariants). The embedder is selected by `providers.embedder`; the LLM is an
//! ordered fallback chain built from `providers.llm_chain`.

use std::sync::Arc;

use tessera_core::config::ProvidersConfig;

use crate::chain::ChainedLlm;
use crate::claude_cli::ClaudeCliLlm;
use crate::ollama::{OllamaEmbedder, OllamaLlm};
use crate::{EmbeddingProvider, LlmProvider, ProviderError};

/// Build the active embedding provider from config.
pub async fn build_embedder(
    cfg: &ProvidersConfig,
) -> Result<Arc<dyn EmbeddingProvider>, ProviderError> {
    match cfg.embedder.as_str() {
        "ollama" => {
            let e = OllamaEmbedder::connect(
                &cfg.ollama.base_url,
                &cfg.ollama.embed_model,
                cfg.ollama.timeout_secs,
            )
            .await?;
            Ok(Arc::new(e))
        }
        "fastembed" => build_fastembed(&cfg.fastembed.model),
        other => Err(ProviderError::Unavailable(format!(
            "unknown embedder: {other}"
        ))),
    }
}

#[cfg(feature = "fastembed")]
fn build_fastembed(model: &str) -> Result<Arc<dyn EmbeddingProvider>, ProviderError> {
    let e = crate::fastembed_embedder::FastembedEmbedder::load(model)?;
    Ok(Arc::new(e))
}

#[cfg(not(feature = "fastembed"))]
fn build_fastembed(_model: &str) -> Result<Arc<dyn EmbeddingProvider>, ProviderError> {
    Err(ProviderError::Unavailable(
        "fastembed embedder requires building with --features fastembed".into(),
    ))
}

/// Build the LLM provider (an ordered fallback chain) from config.
pub fn build_llm(cfg: &ProvidersConfig) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    let mut providers: Vec<Arc<dyn LlmProvider>> = Vec::new();
    for name in &cfg.llm_chain {
        match name.as_str() {
            "ollama" => providers.push(Arc::new(OllamaLlm::new(
                &cfg.ollama.base_url,
                &cfg.ollama.chat_model,
                cfg.ollama.timeout_secs,
            )?)),
            "claude_cli" => providers.push(Arc::new(ClaudeCliLlm::new(
                &cfg.claude_cli.bin,
                cfg.claude_cli.model.clone(),
                cfg.claude_cli.timeout_secs,
                cfg.claude_cli.max_concurrency,
            ))),
            other => {
                return Err(ProviderError::Unavailable(format!(
                    "unknown llm provider: {other}"
                )))
            }
        }
    }
    Ok(Arc::new(ChainedLlm::new(providers)?))
}
