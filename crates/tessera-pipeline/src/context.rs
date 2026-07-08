//! Shared state every pipeline stage needs.

use std::sync::Arc;

use tessera_db::cas::CasStore;
use tessera_db::Db;
use tessera_providers::EmbeddingProvider;

/// Everything a stage handler needs, cloneable across worker tasks.
#[derive(Clone)]
pub struct PipelineContext {
    pub db: Db,
    pub cas: CasStore,
    pub embedder: Arc<dyn EmbeddingProvider>,
    /// The active embedding space id that new vectors are written into.
    pub space_id: i16,
    /// Batch size for embedding jobs.
    pub embed_batch: usize,
}

impl PipelineContext {
    #[must_use]
    pub fn new(
        db: Db,
        cas: CasStore,
        embedder: Arc<dyn EmbeddingProvider>,
        space_id: i16,
        embed_batch: usize,
    ) -> Self {
        Self {
            db,
            cas,
            embedder,
            space_id,
            embed_batch,
        }
    }
}
