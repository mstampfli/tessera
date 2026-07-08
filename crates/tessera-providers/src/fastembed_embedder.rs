//! In-process ONNX embeddings via `fastembed`. Feature-gated behind `fastembed`.
//!
//! This is the intended production default (no runtime service dependency, CPU
//! viable, portable to the CPU-only deploy target). It is gated off in the lean
//! default build because it pulls a large native/codec dependency tree and
//! downloads an onnxruntime binary; enable `--features fastembed` to use it.

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::{EmbedKind, EmbeddingProvider, EmbeddingSpaceInfo, ProviderError, ProviderHealth};

/// Wraps a `fastembed` text-embedding model.
pub struct FastembedEmbedder {
    model: TextEmbedding,
    space: EmbeddingSpaceInfo,
}

impl FastembedEmbedder {
    /// Load `model_name` (e.g. `bge-small-en-v1.5`). Downloads and caches the
    /// model on first use.
    pub fn load(model_name: &str) -> Result<Self, ProviderError> {
        let (model, dim) = resolve_model(model_name)?;
        let embedding =
            TextEmbedding::try_new(InitOptions::new(model).with_show_download_progress(false))
                .map_err(|e| ProviderError::Unavailable(format!("fastembed init: {e}")))?;
        let space = EmbeddingSpaceInfo {
            name: model_name.to_string(),
            dim,
            metric: "cosine",
        };
        Ok(Self {
            model: embedding,
            space,
        })
    }
}

/// Map a friendly model name to the fastembed enum and its dimensionality.
fn resolve_model(name: &str) -> Result<(EmbeddingModel, usize), ProviderError> {
    match name {
        "bge-small-en-v1.5" => Ok((EmbeddingModel::BGESmallENV15, 384)),
        "bge-base-en-v1.5" => Ok((EmbeddingModel::BGEBaseENV15, 768)),
        "nomic-embed-text-v1.5" => Ok((EmbeddingModel::NomicEmbedTextV15, 768)),
        "all-minilm-l6-v2" => Ok((EmbeddingModel::AllMiniLML6V2, 384)),
        other => Err(ProviderError::Unavailable(format!(
            "unknown fastembed model: {other}"
        ))),
    }
}

#[async_trait]
impl EmbeddingProvider for FastembedEmbedder {
    fn space(&self) -> &EmbeddingSpaceInfo {
        &self.space
    }

    async fn embed(
        &self,
        texts: &[String],
        _kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // fastembed is synchronous and CPU-bound. The pipeline already invokes
        // embedding from worker tasks with a bounded connection budget; a future
        // hardening step wraps this in spawn_blocking with an Arc'd model. For
        // now it runs inline on the caller's task.
        let dim = self.space.dim;
        let vectors = self
            .model
            .embed(texts.iter().map(String::as_str).collect(), None)
            .map_err(|e| ProviderError::Other(format!("fastembed embed: {e}")))?;
        for v in &vectors {
            if v.len() != dim {
                return Err(ProviderError::InvalidOutput(format!(
                    "embedding dim {} != space dim {dim}",
                    v.len()
                )));
            }
        }
        Ok(vectors)
    }

    async fn health(&self) -> ProviderHealth {
        // The model is loaded in-process; if we exist, we are up.
        ProviderHealth::Up
    }
}
