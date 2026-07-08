//! An LLM provider that composes an ordered list of providers with fallback.
//!
//! `ChainedLlm` itself implements [`LlmProvider`], so composition is not a second
//! call-site mechanism: every caller sees one provider. It tries each backend in
//! order and falls through to the next on error, returning the last error if all
//! fail. (A per-provider circuit breaker is added in M3; M1 uses plain fallback.)

use std::sync::Arc;

use async_trait::async_trait;

use crate::{GenRequest, GenResponse, LlmProvider, ProviderError, ProviderHealth};

/// Ordered fallback chain over one or more LLM providers.
pub struct ChainedLlm {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl ChainedLlm {
    /// Build a chain. Errors if empty (a chain with no backend cannot generate).
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Result<Self, ProviderError> {
        if providers.is_empty() {
            return Err(ProviderError::Unavailable("empty LLM chain".into()));
        }
        Ok(Self { providers })
    }
}

#[async_trait]
impl LlmProvider for ChainedLlm {
    fn id(&self) -> &'static str {
        "chain"
    }

    async fn generate(&self, req: &GenRequest) -> Result<GenResponse, ProviderError> {
        let mut last: Option<ProviderError> = None;
        for provider in &self.providers {
            match provider.generate(req).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    tracing::warn!(provider = provider.id(), error = %e, "llm provider failed, falling through");
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| ProviderError::Unavailable("no providers in chain".into())))
    }

    async fn health(&self) -> ProviderHealth {
        // Healthy if any backend is up; degraded if some are down; down if all are.
        let mut any_up = false;
        let mut reasons = Vec::new();
        for provider in &self.providers {
            match provider.health().await {
                ProviderHealth::Up => any_up = true,
                ProviderHealth::Degraded(r) | ProviderHealth::Down(r) => {
                    reasons.push(format!("{}: {r}", provider.id()));
                }
            }
        }
        match (any_up, reasons.is_empty()) {
            (true, true) => ProviderHealth::Up,
            (true, false) => ProviderHealth::Degraded(reasons.join("; ")),
            (false, _) => ProviderHealth::Down(reasons.join("; ")),
        }
    }
}
