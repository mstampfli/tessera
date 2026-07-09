//! Persisted ask-with-citations history.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AskRecord {
    pub id: Uuid,
    pub question: String,
    pub answer: Value,
    pub created_at: DateTime<Utc>,
}

/// Record one question and its full answer (best-effort; the caller ignores a
/// failure so a history hiccup never fails the ask itself).
pub async fn record(
    pool: &PgPool,
    principal: Option<&str>,
    question: &str,
    answer: &Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO ask_history (id, principal, question, answer) VALUES ($1, $2, $3, $4)",
    )
    .bind(tessera_core::new_id())
    .bind(principal)
    .bind(question)
    .bind(answer)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// The most recent questions and answers, newest first.
pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<AskRecord>> {
    sqlx::query_as::<_, AskRecord>(
        "SELECT id, question, answer, created_at FROM ask_history
         ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
