//! The `sessions` repository. Web-UI sessions are opaque server-side records;
//! the browser holds only a random cookie value whose blake3 hash is stored.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

/// A session row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// Create a session for `user_id` valid for `ttl_days`, storing the hash of the
/// cookie secret (produced by `tessera_core::secret::generate_session`).
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &[u8],
    ttl_days: i64,
) -> Result<Session> {
    let id = tessera_core::new_id();
    let expires_at = Utc::now() + Duration::days(ttl_days);
    sqlx::query_as::<_, Session>(
        "INSERT INTO sessions (id, user_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4)
         RETURNING id, user_id, token_hash, created_at, expires_at, last_seen_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)
}

/// Resolve a session by the hash of its cookie value, only if unexpired.
/// Also refreshes `last_seen_at` (sliding sessions).
pub async fn resolve_active(pool: &PgPool, token_hash: &[u8]) -> Result<Option<Session>> {
    sqlx::query_as::<_, Session>(
        "UPDATE sessions SET last_seen_at = now()
         WHERE token_hash = $1 AND expires_at > now()
         RETURNING id, user_id, token_hash, created_at, expires_at, last_seen_at",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Delete a session by its cookie-value hash (logout). Idempotent.
pub async fn delete(pool: &PgPool, token_hash: &[u8]) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

/// Purge expired sessions. Called periodically; safe to run any time.
pub async fn purge_expired(pool: &PgPool) -> Result<u64> {
    let affected = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    Ok(affected)
}
