//! Shared state every pipeline stage needs.

use std::sync::Arc;

use tessera_db::cas::CasStore;
use tessera_db::Db;
use tessera_extract::plugin::PluginRegistry;
use tessera_providers::{EmbeddingProvider, LlmProvider};

/// Everything a stage handler needs, cloneable across worker tasks.
#[derive(Clone)]
pub struct PipelineContext {
    pub db: Db,
    pub cas: CasStore,
    pub embedder: Arc<dyn EmbeddingProvider>,
    /// Extractor plugins matched by media type (empty when none configured).
    pub plugins: Arc<PluginRegistry>,
    /// The generation provider used for insight synthesis (a fallback chain).
    pub llm: Arc<dyn LlmProvider>,
    /// The active embedding space id that new vectors are written into.
    pub space_id: i16,
    /// The active space's dimension (for index-hitting halfvec casts).
    pub space_dim: i32,
    /// Batch size for embedding jobs.
    pub embed_batch: usize,
    /// Max cosine distance for a chunk to join an existing cluster.
    pub cluster_max_distance: f64,
    /// New members a cluster must gain before its insight is re-synthesized.
    pub cluster_dirty_threshold: i32,
    /// Debounce before synthesizing a dirty cluster.
    pub synth_debounce_secs: i64,
    /// Global nearest neighbours linked per entity by semantic correlation.
    pub semantic_k: i64,
    /// Floor cosine similarity for a semantic correlation edge.
    pub semantic_min_sim: f64,
    /// Max separation (days) for a temporal correlation edge.
    pub temporal_window_days: f64,
    /// Decay constant (days) for temporal edge strength.
    pub temporal_tau_days: f64,
    /// Co-occurrence degree above which an entity cannot merge communities.
    pub community_hub_degree: i64,
}
