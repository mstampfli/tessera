//! Insights: the actionable, cited cards produced by synthesis over clusters.
//!
//! Each insight carries a title, narrative, severity, confidence, suggested
//! actions, and evidence (the specific chunks that back it). When a cluster is
//! re-synthesized, the previous insight is superseded rather than deleted, so the
//! history is preserved and the "live" insight is the newest non-superseded one.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Insight {
    pub id: Uuid,
    pub cluster_id: Option<Uuid>,
    pub kind: String,
    pub title: String,
    pub body_md: String,
    pub tags: Vec<String>,
    pub severity: String,
    pub confidence: f32,
    pub suggested_actions: Value,
    pub entity_ids: Vec<Uuid>,
    pub model: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

const COLS: &str = "id, cluster_id, kind, title, body_md, tags, severity, confidence, \
                    suggested_actions, entity_ids, model, status, created_at";

/// One piece of evidence backing an insight.
pub struct EvidenceInput {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub entity_id: Option<Uuid>,
    pub note: Option<String>,
}

/// Everything needed to persist a synthesized insight.
pub struct InsightInput {
    pub cluster_id: Uuid,
    pub title: String,
    pub body_md: String,
    pub tags: Vec<String>,
    pub severity: String,
    pub confidence: f32,
    pub suggested_actions: Value,
    pub entity_ids: Vec<Uuid>,
    pub model: String,
    pub input_hash: Vec<u8>,
    pub evidence: Vec<EvidenceInput>,
}

/// The `input_hash` of the current live (non-superseded) insight for a cluster,
/// used to skip synthesis when nothing material changed.
pub async fn live_input_hash(pool: &PgPool, cluster_id: Uuid) -> Result<Option<Vec<u8>>> {
    sqlx::query_scalar::<_, Option<Vec<u8>>>(
        "SELECT input_hash FROM insights
         WHERE cluster_id = $1 AND status <> 'superseded'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(cluster_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
    .map(Option::flatten)
}

/// Create an insight, superseding any prior live insight for the same cluster,
/// and write its evidence. Returns the new insight id.
pub async fn create(pool: &PgPool, input: &InsightInput) -> Result<Uuid> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;

    // Supersede the previous live insight(s) for this cluster.
    sqlx::query(
        "UPDATE insights SET status = 'superseded', updated_at = now()
         WHERE cluster_id = $1 AND status IN ('new', 'surfaced', 'useful')",
    )
    .bind(input.cluster_id)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    let id = tessera_core::new_id();
    sqlx::query(
        "INSERT INTO insights
            (id, cluster_id, title, body_md, tags, severity, confidence,
             suggested_actions, entity_ids, model, input_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(input.cluster_id)
    .bind(&input.title)
    .bind(&input.body_md)
    .bind(&input.tags)
    .bind(&input.severity)
    .bind(input.confidence)
    .bind(&input.suggested_actions)
    .bind(&input.entity_ids)
    .bind(&input.model)
    .bind(&input.input_hash)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    for ev in &input.evidence {
        sqlx::query(
            "INSERT INTO insight_evidence (insight_id, chunk_id, document_id, entity_id, note)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (insight_id, chunk_id) DO NOTHING",
        )
        .bind(id)
        .bind(ev.chunk_id)
        .bind(ev.document_id)
        .bind(ev.entity_id)
        .bind(ev.note.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    tx.commit().await.map_err(map_sqlx)?;
    Ok(id)
}

/// List insights (optionally filtered by status), newest first.
pub async fn list(pool: &PgPool, status: Option<&str>, limit: i64) -> Result<Vec<Insight>> {
    sqlx::query_as::<_, Insight>(&format!(
        "SELECT {COLS} FROM insights
         WHERE ($1::text IS NULL OR status = $1)
           AND status <> 'superseded'
         ORDER BY
             CASE severity WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2
                           WHEN 'low' THEN 3 ELSE 4 END,
             created_at DESC
         LIMIT $2"
    ))
    .bind(status)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Insight>> {
    sqlx::query_as::<_, Insight>(&format!("SELECT {COLS} FROM insights WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(map_sqlx)
}

/// The evidence chunks for an insight (for the evidence drawer).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Evidence {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub title: Option<String>,
    pub seq: i32,
    pub excerpt: String,
}

pub async fn evidence(pool: &PgPool, insight_id: Uuid) -> Result<Vec<Evidence>> {
    sqlx::query_as::<_, Evidence>(
        "SELECT ie.chunk_id, ie.document_id, d.title, c.seq,
                left(c.text, 400) AS excerpt
         FROM insight_evidence ie
         JOIN chunks c ON c.id = ie.chunk_id
         JOIN documents d ON d.id = ie.document_id
         WHERE ie.insight_id = $1
         ORDER BY c.seq",
    )
    .bind(insight_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Update an insight's triage status (surfaced / useful / dismissed).
pub async fn set_status(pool: &PgPool, id: Uuid, status: &str) -> Result<bool> {
    let n = sqlx::query("UPDATE insights SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    Ok(n > 0)
}
