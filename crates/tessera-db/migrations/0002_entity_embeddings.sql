-- Materialized per-entity context embeddings: the mean of the embeddings of the
-- chunks an entity is mentioned in. Kept in its own table (not derived on the
-- fly) so global nearest-neighbour correlation can run off an ANN index in
-- O(log n) per entity instead of an all-pairs scan. The HNSW index itself is
-- created at startup (its dimension follows the active embedding space), the
-- same pattern chunk_embeddings uses.
CREATE TABLE entity_embeddings (
    entity_id  uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    space_id   smallint NOT NULL REFERENCES embedding_spaces(id) ON DELETE CASCADE,
    embedding  vector NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entity_id, space_id)
);
