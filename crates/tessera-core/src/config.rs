//! Layered configuration: built-in defaults, then `tessera.toml`, then
//! `TESSERA__*` environment overrides, with the database URL taken from the
//! conventional `DATABASE_URL` secret env var.
//!
//! Secrets (the DB URL, the initial admin password) come from the environment,
//! never a committed file.

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind};

/// Root configuration for `tesserad`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub cas: CasConfig,
    pub log: LogConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

/// Extractor plugin configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// Directory of plugin manifest TOML files. When unset, no plugins run.
    #[serde(default)]
    pub dir: Option<PathBuf>,
}

/// Configuration for the pluggable AI provider layer. These are plain strings
/// (endpoints, model names, binary paths); the provider crate interprets them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersConfig {
    /// Which embedding backend to use: `ollama` (default) or `fastembed`.
    #[serde(default = "default_embedder")]
    pub embedder: String,
    /// Ordered LLM providers to try for generation (first that succeeds wins).
    #[serde(default = "default_llm_chain")]
    pub llm_chain: Vec<String>,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub fastembed: FastembedConfig,
    #[serde(default)]
    pub claude_cli: ClaudeCliConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub base_url: String,
    #[serde(default = "default_ollama_embed_model")]
    pub embed_model: String,
    #[serde(default = "default_ollama_chat_model")]
    pub chat_model: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FastembedConfig {
    #[serde(default = "default_fastembed_model")]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCliConfig {
    #[serde(default = "default_claude_bin")]
    pub bin: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_claude_timeout_secs")]
    pub timeout_secs: u64,
    /// Max concurrent CLI invocations (it rides the subscription).
    #[serde(default = "default_claude_concurrency")]
    pub max_concurrency: usize,
}

/// Pipeline worker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Number of concurrent worker tasks.
    #[serde(default = "default_workers")]
    pub workers: usize,
    /// How many chunk ids go into one embedding job batch.
    #[serde(default = "default_embed_batch")]
    pub embed_batch: usize,
    /// Max cosine distance for a chunk to join an existing cluster (model
    /// dependent; the default suits nomic-embed-text).
    #[serde(default = "default_cluster_max_distance")]
    pub cluster_max_distance: f64,
    /// New members a cluster must gain before its insight is re-synthesized.
    #[serde(default = "default_dirty_threshold")]
    pub cluster_dirty_threshold: i32,
    /// Debounce before synthesizing a dirty cluster's insight, so a burst of
    /// ingestion produces one synthesis, not many.
    #[serde(default = "default_synth_debounce_secs")]
    pub synth_debounce_secs: i64,
    /// How many global nearest neighbours each entity is linked to by semantic
    /// (context-similarity) correlation edges.
    #[serde(default = "default_semantic_k")]
    pub semantic_k: i64,
    /// Floor cosine similarity for a semantic correlation edge (relative top-k
    /// still does the ranking; this only drops near-orthogonal pairs).
    #[serde(default = "default_semantic_min_sim")]
    pub semantic_min_sim: f64,
    /// Max separation, in days, for a temporal correlation edge.
    #[serde(default = "default_temporal_window_days")]
    pub temporal_window_days: f64,
    /// Decay constant, in days, for temporal edge strength (exp(-delta/tau)).
    #[serde(default = "default_temporal_tau_days")]
    pub temporal_tau_days: f64,
}

fn default_embedder() -> String {
    "ollama".to_string()
}
fn default_llm_chain() -> Vec<String> {
    vec!["ollama".to_string()]
}
fn default_ollama_url() -> String {
    "http://127.0.0.1:11434".to_string()
}
fn default_ollama_embed_model() -> String {
    "nomic-embed-text".to_string()
}
fn default_ollama_chat_model() -> String {
    "qwen2.5:3b".to_string()
}
fn default_timeout_secs() -> u64 {
    120
}
fn default_fastembed_model() -> String {
    "bge-small-en-v1.5".to_string()
}
fn default_claude_bin() -> String {
    "claude".to_string()
}
fn default_claude_timeout_secs() -> u64 {
    180
}
fn default_claude_concurrency() -> usize {
    1
}
fn default_workers() -> usize {
    4
}
fn default_embed_batch() -> usize {
    64
}
fn default_cluster_max_distance() -> f64 {
    0.4
}
fn default_dirty_threshold() -> i32 {
    3
}
fn default_semantic_k() -> i64 {
    6
}
fn default_semantic_min_sim() -> f64 {
    0.3
}
fn default_temporal_window_days() -> f64 {
    14.0
}
fn default_temporal_tau_days() -> f64 {
    7.0
}
fn default_synth_debounce_secs() -> i64 {
    30
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            embedder: default_embedder(),
            llm_chain: default_llm_chain(),
            ollama: OllamaConfig::default(),
            fastembed: FastembedConfig::default(),
            claude_cli: ClaudeCliConfig::default(),
        }
    }
}
impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_ollama_url(),
            embed_model: default_ollama_embed_model(),
            chat_model: default_ollama_chat_model(),
            timeout_secs: default_timeout_secs(),
        }
    }
}
impl Default for FastembedConfig {
    fn default() -> Self {
        Self {
            model: default_fastembed_model(),
        }
    }
}
impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            bin: default_claude_bin(),
            model: None,
            timeout_secs: default_claude_timeout_secs(),
            max_concurrency: default_claude_concurrency(),
        }
    }
}
impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            workers: default_workers(),
            embed_batch: default_embed_batch(),
            cluster_max_distance: default_cluster_max_distance(),
            cluster_dirty_threshold: default_dirty_threshold(),
            synth_debounce_secs: default_synth_debounce_secs(),
            semantic_k: default_semantic_k(),
            semantic_min_sim: default_semantic_min_sim(),
            temporal_window_days: default_temporal_window_days(),
            temporal_tau_days: default_temporal_tau_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address the HTTP server binds to. Default is loopback; production sets
    /// this to the host's tailnet IP via env so the port is never public.
    pub bind: SocketAddr,
    /// Public base URL used when emitting absolute links (optional).
    #[serde(default)]
    pub public_url: Option<String>,
    /// Set the `Secure` attribute on session cookies. Default false so local
    /// HTTP development works; production (behind Caddy TLS) sets this true.
    #[serde(default)]
    pub secure_cookies: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Postgres connection URL. Sourced from `DATABASE_URL`; the placeholder
    /// default fails fast if neither env nor file provides a real one.
    pub url: String,
    /// Max connections for the interactive (API) pool.
    #[serde(default = "default_api_conns")]
    pub max_connections: u32,
    /// Max connections for the background worker pool (kept separate so bulk
    /// pipeline work can never starve interactive queries).
    #[serde(default = "default_worker_conns")]
    pub worker_max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasConfig {
    /// Filesystem root of the content-addressed store for raw ingested bytes.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// `json` for production, `pretty` for local development.
    #[serde(default = "default_log_format")]
    pub format: String,
    /// A `tracing_subscriber` env-filter directive.
    #[serde(default = "default_log_filter")]
    pub filter: String,
}

fn default_api_conns() -> u32 {
    10
}
fn default_worker_conns() -> u32 {
    6
}
fn default_log_format() -> String {
    "pretty".to_string()
}
fn default_log_filter() -> String {
    "info,tessera=debug,sqlx=warn".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                bind: "127.0.0.1:8080".parse().expect("static addr"),
                public_url: None,
                secure_cookies: false,
            },
            database: DatabaseConfig {
                url: "postgres://tessera:tessera@127.0.0.1:5432/tessera".to_string(),
                max_connections: default_api_conns(),
                worker_max_connections: default_worker_conns(),
            },
            cas: CasConfig {
                path: PathBuf::from("./data/cas"),
            },
            log: LogConfig {
                format: default_log_format(),
                filter: default_log_filter(),
            },
            providers: ProvidersConfig::default(),
            pipeline: PipelineConfig::default(),
            plugins: PluginsConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from defaults, an optional TOML file, and the
    /// environment. `DATABASE_URL`, if set, always wins for the DB URL.
    pub fn load(toml_path: Option<&std::path::Path>) -> Result<Self, Error> {
        let mut fig = Figment::from(Serialized::defaults(Config::default()));
        if let Some(path) = toml_path {
            fig = fig.merge(Toml::file(path));
        } else {
            // Conventionally look for a tessera.toml in the working directory.
            fig = fig.merge(Toml::file("tessera.toml"));
        }
        fig = fig.merge(Env::prefixed("TESSERA__").split("__"));

        let mut config: Config = fig
            .extract()
            .map_err(|e| Error::new(ErrorKind::Invalid, format!("invalid configuration: {e}")))?;

        if let Ok(url) = std::env::var("DATABASE_URL") {
            if !url.is_empty() {
                config.database.url = url;
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn defaults_are_valid() {
        let c = Config::default();
        assert_eq!(c.server.bind.port(), 8080);
        assert_eq!(c.database.max_connections, 10);
        assert_eq!(c.log.format, "pretty");
    }
}
