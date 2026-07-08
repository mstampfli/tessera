//! The `users` repository. tessera is single-primary-user; this still models a
//! table so multi-user is a data change, not a schema rewrite.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tessera_core::error::{Error, Result};
use uuid::Uuid;

use crate::map_sqlx;

/// A user row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Insert a new user with an already-hashed password. Returns the row.
pub async fn create(pool: &PgPool, username: &str, password_hash: &str) -> Result<User> {
    let id = tessera_core::new_id();
    sqlx::query_as::<_, User>(
        "INSERT INTO users (id, username, password_hash)
         VALUES ($1, $2, $3)
         RETURNING id, username, password_hash, created_at",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)
}

/// Update an existing user's password hash (used by `tesserad user set-password`).
pub async fn set_password(pool: &PgPool, username: &str, password_hash: &str) -> Result<()> {
    let affected = sqlx::query("UPDATE users SET password_hash = $2 WHERE username = $1")
        .bind(username)
        .bind(password_hash)
        .execute(pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    if affected == 0 {
        return Err(Error::not_found("no such user"));
    }
    Ok(())
}

/// Look up a user by username. `Ok(None)` when absent (not an error).
pub async fn by_username(pool: &PgPool, username: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Look up a user by id. `Ok(None)` when absent.
pub async fn by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Count users. Used to decide whether a first-run admin needs bootstrapping.
pub async fn count(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)
}
