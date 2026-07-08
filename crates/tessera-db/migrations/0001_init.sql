-- tessera initial schema.
--
-- Design notes that bind the rest of the system:
--  * Every primary key is a UUIDv7 (time-ordered -> keyset pagination for free).
--  * Postgres is the ONLY stateful service: vectors (pgvector), full text
--    (tsvector), the job queue (SKIP LOCKED), sessions, everything.
--  * Raw bytes live in an on-disk content-addressed store keyed by blake3; this
--    schema holds normalized text + metadata only. documents.content_hash is
--    both the CAS key and the idempotency key.
--  * HNSW (not IVFFlat) for vectors: it inserts incrementally at write time and
--    never needs a full-corpus rebuild. The per-space partial expression index
--    is created at runtime once a model/space is registered, so this migration
--    defines the columns but not the vector index.

CREATE EXTENSION IF NOT EXISTS vector;

-- ---------------------------------------------------------------------------
-- Identity, auth, audit
-- ---------------------------------------------------------------------------

CREATE TABLE users (
    id            uuid PRIMARY KEY,
    username      text NOT NULL UNIQUE,
    password_hash text NOT NULL,                 -- argon2id PHC string
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash    bytea NOT NULL UNIQUE,         -- blake3(32-byte cookie secret)
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_by_user ON sessions (user_id);
CREATE INDEX sessions_expiry ON sessions (expires_at);

CREATE TABLE api_tokens (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    prefix        text NOT NULL UNIQUE,          -- 8 hex chars, plaintext, O(1) lookup
    token_hash    bytea NOT NULL,                -- blake3(secret)
    scopes        text[] NOT NULL,               -- {read, ingest, mcp, admin}
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz,
    revoked_at    timestamptz,
    last_used_at  timestamptz
);

-- Append-only audit trail (repudiation mitigation). Never updated or deleted.
CREATE TABLE audit_log (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at            timestamptz NOT NULL DEFAULT now(),
    principal     text,                          -- 'user:<id>' or 'token:<id>'
    action        text NOT NULL,                 -- 'ingest','token.create','login',...
    target        text,                          -- affected resource id, if any
    detail        jsonb NOT NULL DEFAULT '{}'    -- ids/metadata only, never content
);
CREATE INDEX audit_by_time ON audit_log (at DESC);

-- ---------------------------------------------------------------------------
-- Content: sources, documents, chunks
-- ---------------------------------------------------------------------------

CREATE TABLE sources (
    id          uuid PRIMARY KEY,
    kind        text NOT NULL,                   -- 'upload','api','url','agent'
    name        text NOT NULL,
    config      jsonb NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE documents (
    id            uuid PRIMARY KEY,
    source_id     uuid NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    content_hash  bytea NOT NULL UNIQUE,          -- blake3(raw bytes) = CAS key = idempotency
    media_type    text NOT NULL,
    size_bytes    bigint NOT NULL CHECK (size_bytes >= 0),
    title         text,
    uri           text,
    meta          jsonb NOT NULL DEFAULT '{}',
    status        text NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','processing','ready','failed')),
    error         text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    processed_at  timestamptz
);
CREATE INDEX documents_source_idx ON documents (source_id, id);
CREATE INDEX documents_status_idx ON documents (status) WHERE status <> 'ready';

CREATE TABLE chunks (
    id           uuid PRIMARY KEY,
    document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    seq          int NOT NULL,
    text         text NOT NULL,
    token_count  int NOT NULL DEFAULT 0,
    byte_start   bigint,
    byte_end     bigint,
    meta         jsonb NOT NULL DEFAULT '{}',
    -- Explicit 'english' regconfig makes to_tsvector IMMUTABLE (required for a
    -- generated column). Bounded input length caps the work on pathological rows.
    tsv          tsvector GENERATED ALWAYS AS (to_tsvector('english', left(text, 100000))) STORED,
    UNIQUE (document_id, seq)
);
CREATE INDEX chunks_tsv_idx ON chunks USING gin (tsv);
CREATE INDEX chunks_by_doc ON chunks (document_id, seq);

-- ---------------------------------------------------------------------------
-- Embeddings: a registry of swappable model "spaces" + untyped vector storage.
-- The per-space HNSW index (a partial expression index casting to halfvec) is
-- created at runtime by the application, not here.
-- ---------------------------------------------------------------------------

CREATE TABLE embedding_spaces (
    id          smallint PRIMARY KEY,
    name        text NOT NULL UNIQUE,            -- 'bge-small-en-v1.5'
    provider    text NOT NULL,                   -- 'fastembed','ollama',...
    dim         int NOT NULL,
    metric      text NOT NULL DEFAULT 'cosine',
    active      boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE chunk_embeddings (
    chunk_id    uuid NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    space_id    smallint NOT NULL REFERENCES embedding_spaces(id) ON DELETE CASCADE,
    embedding   vector NOT NULL,                 -- dim enforced in code against the space
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (chunk_id, space_id)
);

-- ---------------------------------------------------------------------------
-- Entities, mentions, edges (the correlation graph, all in Postgres)
-- ---------------------------------------------------------------------------

CREATE TABLE entities (
    id             uuid PRIMARY KEY,
    kind           text NOT NULL,                -- 'ip','domain','hash_sha256','cve',...
    value          text NOT NULL,                -- canonical form (one primitive per kind)
    display_value  text NOT NULL,                -- a representative raw surface form
    attrs          jsonb NOT NULL DEFAULT '{}',
    mention_count  bigint NOT NULL DEFAULT 0,
    salience       real NOT NULL DEFAULT 0,
    first_seen     timestamptz NOT NULL DEFAULT now(),
    last_seen      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, value)                          -- DB-level dedup anchor
);
CREATE INDEX entities_by_kind ON entities (kind);

CREATE TABLE entity_mentions (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    entity_id    uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    chunk_id     uuid NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    raw_surface  text NOT NULL,
    span         int4range,
    extractor    text NOT NULL,
    confidence   real NOT NULL DEFAULT 1.0,
    UNIQUE (entity_id, chunk_id, span)
);
CREATE INDEX mentions_by_entity ON entity_mentions (entity_id, document_id);
CREATE INDEX mentions_by_doc ON entity_mentions (document_id);

CREATE TABLE entity_edges (
    src_id      uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    dst_id      uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    rel         text NOT NULL,                   -- 'co_occurs','resolves_to','alias_of',...
    weight      double precision NOT NULL DEFAULT 1,
    score       double precision NOT NULL DEFAULT 0,
    evidence    jsonb NOT NULL DEFAULT '{}',
    source_count int NOT NULL DEFAULT 1,
    first_seen  timestamptz NOT NULL DEFAULT now(),
    last_seen   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (src_id, dst_id, rel),
    -- Symmetric relations are stored once, canonically ordered, so co_occurs(a,b)
    -- and co_occurs(b,a) are the same row.
    CHECK (rel <> 'co_occurs' OR src_id < dst_id)
);
CREATE INDEX edges_by_dst ON entity_edges (dst_id, rel);

-- ---------------------------------------------------------------------------
-- Clusters and insights
-- ---------------------------------------------------------------------------

CREATE TABLE clusters (
    id           uuid PRIMARY KEY,
    space_id     smallint NOT NULL REFERENCES embedding_spaces(id) ON DELETE CASCADE,
    centroid     vector NOT NULL,
    size         int NOT NULL DEFAULT 0,
    label        text,
    dirty_count  int NOT NULL DEFAULT 0,          -- new members since last synthesis
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE cluster_members (
    cluster_id  uuid NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    chunk_id    uuid NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    similarity  real NOT NULL,
    added_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cluster_id, chunk_id)
);
-- Each chunk belongs to at most one cluster.
CREATE UNIQUE INDEX cluster_members_by_chunk ON cluster_members (chunk_id);

CREATE TABLE insights (
    id           uuid PRIMARY KEY,
    cluster_id   uuid REFERENCES clusters(id) ON DELETE SET NULL,
    kind         text NOT NULL DEFAULT 'cluster_summary',
    title        text NOT NULL,
    body_md      text NOT NULL,
    tags         text[] NOT NULL DEFAULT '{}',
    severity     text NOT NULL DEFAULT 'info'
                    CHECK (severity IN ('info','low','medium','high','critical')),
    confidence   real NOT NULL DEFAULT 0,
    suggested_actions jsonb NOT NULL DEFAULT '[]',
    entity_ids   uuid[] NOT NULL DEFAULT '{}',
    model        text NOT NULL DEFAULT '',
    input_hash   bytea,                           -- dedup key across synthesis runs
    status       text NOT NULL DEFAULT 'new'
                    CHECK (status IN ('new','surfaced','useful','dismissed','superseded')),
    supersedes_id uuid REFERENCES insights(id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX insights_status_idx ON insights (status, created_at DESC);

CREATE TABLE insight_evidence (
    insight_id   uuid NOT NULL REFERENCES insights(id) ON DELETE CASCADE,
    chunk_id     uuid NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    entity_id    uuid REFERENCES entities(id) ON DELETE SET NULL,
    note         text,
    PRIMARY KEY (insight_id, chunk_id)
);

-- ---------------------------------------------------------------------------
-- Job queue (SKIP LOCKED) and settings
-- ---------------------------------------------------------------------------

CREATE TABLE jobs (
    id           uuid PRIMARY KEY,
    kind         text NOT NULL,                   -- 'process_document','embed_chunks',...
    payload      jsonb NOT NULL,
    state        text NOT NULL DEFAULT 'queued'
                    CHECK (state IN ('queued','running','done','failed','dead')),
    priority     smallint NOT NULL DEFAULT 0,
    run_at       timestamptz NOT NULL DEFAULT now(),
    attempts     int NOT NULL DEFAULT 0,
    max_attempts int NOT NULL DEFAULT 5,
    locked_by    text,
    locked_until timestamptz,
    dedupe_key   text,
    last_error   text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    finished_at  timestamptz
);
-- Claims are O(log n) regardless of table size thanks to this partial index.
CREATE INDEX jobs_claim_idx ON jobs (priority DESC, run_at, id) WHERE state = 'queued';
CREATE UNIQUE INDEX jobs_dedupe_idx ON jobs (dedupe_key)
    WHERE dedupe_key IS NOT NULL AND state IN ('queued','running');
CREATE INDEX jobs_lease_idx ON jobs (locked_until) WHERE state = 'running';

CREATE TABLE settings (
    key         text PRIMARY KEY,
    value       jsonb NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
