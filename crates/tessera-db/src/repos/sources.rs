//! The `sources` repository. A source groups documents by where they came from
//! (an upload batch, an API caller, a URL, an agent).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Source {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub config: Value,
    pub created_at: DateTime<Utc>,
}

pub async fn create(pool: &PgPool, kind: &str, name: &str, config: &Value) -> Result<Source> {
    let id = tessera_core::new_id();
    sqlx::query_as::<_, Source>(
        "INSERT INTO sources (id, kind, name, config)
         VALUES ($1, $2, $3, $4)
         RETURNING id, kind, name, config, created_at",
    )
    .bind(id)
    .bind(kind)
    .bind(name)
    .bind(config)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT id, kind, name, config, created_at FROM sources WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// List sources newest-first with keyset pagination on the uuidv7 id.
pub async fn list(pool: &PgPool, before: Option<Uuid>, limit: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT id, kind, name, config, created_at FROM sources
         WHERE ($1::uuid IS NULL OR id < $1)
         ORDER BY id DESC LIMIT $2",
    )
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
