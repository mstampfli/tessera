//! Ollama-backed providers (embedding and generation) over the local HTTP API.
//!
//! This is the default M1 embedding backend: it is dependency-light, works on
//! CPU and GPU, and is already running in the target environment. The
//! dimensionality of the embedding space is discovered at connect time by
//! embedding a probe string, so swapping the Ollama model needs no code change.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    EmbedKind, EmbeddingProvider, EmbeddingSpaceInfo, GenRequest, GenResponse, LlmProvider,
    ProviderError, ProviderHealth,
};

fn client(timeout_secs: u64) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| ProviderError::Other(format!("http client: {e}")))
}

// --- Embedding -------------------------------------------------------------

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

/// Embeds text via Ollama `/api/embed`.
pub struct OllamaEmbedder {
    http: reqwest::Client,
    base_url: String,
    model: String,
    space: EmbeddingSpaceInfo,
    prefix_document: Option<&'static str>,
    prefix_query: Option<&'static str>,
}

impl OllamaEmbedder {
    /// Connect and discover the embedding dimensionality by probing the model.
    pub async fn connect(
        base_url: &str,
        model: &str,
        timeout_secs: u64,
    ) -> Result<Self, ProviderError> {
        let http = client(timeout_secs)?;
        let base_url = base_url.trim_end_matches('/').to_string();

        // Some models (nomic) require task-instruction prefixes for good results.
        let (prefix_document, prefix_query) = if model.contains("nomic") {
            (Some("search_document: "), Some("search_query: "))
        } else {
            (None, None)
        };

        let probe =
            Self::embed_raw(&http, &base_url, model, &["dimension probe".to_string()]).await?;
        let dim = probe
            .first()
            .map(Vec::len)
            .filter(|d| *d > 0)
            .ok_or_else(|| ProviderError::InvalidOutput("empty probe embedding".into()))?;

        let space = EmbeddingSpaceInfo {
            name: model.to_string(),
            dim,
            metric: "cosine",
        };
        Ok(Self {
            http,
            base_url,
            model: model.to_string(),
            space,
            prefix_document,
            prefix_query,
        })
    }

    async fn embed_raw(
        http: &reqwest::Client,
        base_url: &str,
        model: &str,
        inputs: &[String],
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        let resp = http
            .post(format!("{base_url}/api/embed"))
            .json(&serde_json::json!({ "model": model, "input": inputs }))
            .send()
            .await
            .map_err(map_reqwest)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "ollama embed {status}: {body}"
            )));
        }
        let parsed: EmbedResponse = resp.json().await.map_err(map_reqwest)?;
        if parsed.embeddings.len() != inputs.len() {
            return Err(ProviderError::InvalidOutput(format!(
                "expected {} embeddings, got {}",
                inputs.len(),
                parsed.embeddings.len()
            )));
        }
        Ok(parsed.embeddings)
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedder {
    fn space(&self) -> &EmbeddingSpaceInfo {
        &self.space
    }

    async fn embed(
        &self,
        texts: &[String],
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefix = match kind {
            EmbedKind::Document => self.prefix_document,
            EmbedKind::Query => self.prefix_query,
        };
        let inputs: Vec<String> = match prefix {
            Some(p) => texts.iter().map(|t| format!("{p}{t}")).collect(),
            None => texts.to_vec(),
        };
        let vecs = Self::embed_raw(&self.http, &self.base_url, &self.model, &inputs).await?;
        // Guard the contract the storage layer relies on: every vector matches
        // the registered space dimensionality.
        for v in &vecs {
            if v.len() != self.space.dim {
                return Err(ProviderError::InvalidOutput(format!(
                    "embedding dim {} != space dim {}",
                    v.len(),
                    self.space.dim
                )));
            }
        }
        Ok(vecs)
    }

    async fn health(&self) -> ProviderHealth {
        match self
            .http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => ProviderHealth::Up,
            Ok(r) => ProviderHealth::Degraded(format!("ollama status {}", r.status())),
            Err(e) => ProviderHealth::Down(format!("ollama unreachable: {e}")),
        }
    }
}

// --- Generation ------------------------------------------------------------

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
    #[serde(default)]
    model: String,
}

/// Generates text via Ollama `/api/generate`.
pub struct OllamaLlm {
    http: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaLlm {
    pub fn new(base_url: &str, model: &str, timeout_secs: u64) -> Result<Self, ProviderError> {
        Ok(Self {
            http: client(timeout_secs)?,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
        })
    }
}

#[async_trait]
impl LlmProvider for OllamaLlm {
    fn id(&self) -> &'static str {
        "ollama"
    }

    async fn generate(&self, req: &GenRequest) -> Result<GenResponse, ProviderError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "prompt": req.prompt,
            "stream": false,
        });
        if let Some(system) = &req.system {
            body["system"] = serde_json::Value::String(system.clone());
        }
        if let Some(max) = req.max_tokens {
            body["options"] = serde_json::json!({ "num_predict": max });
        }

        let resp = self
            .http
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "ollama generate {status}: {text}"
            )));
        }
        let parsed: GenerateResponse = resp.json().await.map_err(map_reqwest)?;
        let model = if parsed.model.is_empty() {
            self.model.clone()
        } else {
            parsed.model
        };
        Ok(GenResponse {
            text: parsed.response,
            model,
        })
    }

    async fn health(&self) -> ProviderHealth {
        match self
            .http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => ProviderHealth::Up,
            Ok(r) => ProviderHealth::Degraded(format!("ollama status {}", r.status())),
            Err(e) => ProviderHealth::Down(format!("ollama unreachable: {e}")),
        }
    }
}

fn map_reqwest(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout(Duration::from_secs(0))
    } else if e.is_connect() {
        ProviderError::Unavailable(e.to_string())
    } else {
        ProviderError::Other(e.to_string())
    }
}
