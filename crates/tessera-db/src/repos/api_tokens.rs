//! The `api_tokens` repository.
//!
//! Tokens are looked up by their plaintext 8-char `prefix` (uniquely indexed),
//! then the stored `token_hash` is compared to the presented secret's hash in
//! constant time by the caller. The secret itself is never stored.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

/// An API token row (never contains the secret).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub prefix: String,
    pub token_hash: Vec<u8>,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl ApiToken {
    /// Whether this token is currently usable (not revoked, not expired).
    #[must_use]
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|exp| exp > now)
    }
}

/// Insert a token record. The caller generated the prefix/hash via
/// `tessera_core::secret::generate_api_token` and keeps the plaintext to show
/// the user exactly once.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    prefix: &str,
    token_hash: &[u8],
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> Result<ApiToken> {
    let id = tessera_core::new_id();
    sqlx::query_as::<_, ApiToken>(
        "INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, scopes, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, user_id, name, prefix, token_hash, scopes,
                   created_at, expires_at, revoked_at, last_used_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(prefix)
    .bind(token_hash)
    .bind(scopes)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)
}

/// Find a token by its plaintext prefix. `Ok(None)` if unknown.
pub async fn by_prefix(pool: &PgPool, prefix: &str) -> Result<Option<ApiToken>> {
    sqlx::query_as::<_, ApiToken>(
        "SELECT id, user_id, name, prefix, token_hash, scopes,
                created_at, expires_at, revoked_at, last_used_at
         FROM api_tokens WHERE prefix = $1",
    )
    .bind(prefix)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Stamp last-used. Best-effort; a failure here must not fail the request.
pub async fn touch(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

/// List a user's tokens, newest first (never returns the hash to the UI layer).
pub async fn list(pool: &PgPool, user_id: Uuid) -> Result<Vec<ApiToken>> {
    sqlx::query_as::<_, ApiToken>(
        "SELECT id, user_id, name, prefix, token_hash, scopes,
                created_at, expires_at, revoked_at, last_used_at
         FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Revoke a token (idempotent: sets `revoked_at` if not already set).
pub async fn revoke(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(affected > 0)
}
