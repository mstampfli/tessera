//! Database layer: connection pools, embedded migrations, and typed repos.
//!
//! Two pools share one `DATABASE_URL`: `api` for short interactive requests and
//! `worker` for long background pipeline work, so bulk jobs can never exhaust
//! the pool that serves user requests. Both use `min_connections = 0`, so the
//! second pool costs nothing until the pipeline (M1) starts using it.

pub mod repos;

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tessera_core::error::{Error, ErrorKind};

/// Map any sqlx error into the workspace error taxonomy, classifying a unique
/// violation as a conflict so callers and the API layer can react correctly.
pub fn map_sqlx(e: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &e {
        // 23505 = unique_violation, 23503 = foreign_key_violation.
        if db.code().as_deref() == Some("23505") {
            return Error::new(ErrorKind::Conflict, "resource already exists").with_source(e);
        }
    }
    if matches!(e, sqlx::Error::RowNotFound) {
        return Error::new(ErrorKind::NotFound, "not found");
    }
    Error::new(ErrorKind::Db, "database error").with_source(e)
}

/// The database handle passed around the application.
#[derive(Clone)]
pub struct Db {
    /// Interactive pool for API request handlers.
    pub api: PgPool,
    /// Background pool for pipeline workers (used from M1).
    pub worker: PgPool,
}

impl Db {
    /// Open both pools against `url`.
    pub async fn connect(url: &str, api_max: u32, worker_max: u32) -> Result<Self, Error> {
        let api = PgPoolOptions::new()
            .max_connections(api_max)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(map_sqlx)?;
        let worker = PgPoolOptions::new()
            .max_connections(worker_max)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(30))
            .connect(url)
            .await
            .map_err(map_sqlx)?;
        Ok(Self { api, worker })
    }

    /// Apply all embedded migrations. Idempotent.
    pub async fn migrate(&self) -> Result<(), Error> {
        sqlx::migrate!("./migrations")
            .run(&self.api)
            .await
            .map_err(|e| Error::new(ErrorKind::Db, "migration failed").with_source(e))
    }

    /// Cheap readiness probe: a round-trip `SELECT 1`.
    pub async fn ping(&self) -> Result<(), Error> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.api)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}
