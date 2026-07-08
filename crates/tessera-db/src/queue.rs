//! The Postgres-backed job queue.
//!
//! Workers claim jobs with `SELECT ... FOR UPDATE SKIP LOCKED`, which the partial
//! index `jobs_claim_idx` makes O(log n) regardless of table size. A claimed job
//! holds a time-bounded lease that it heartbeats; if a worker dies, the lease
//! expires and the reaper requeues the job. Handlers are idempotent, so
//! at-least-once delivery is safe.
//!
//! `enqueue` accepts any executor (pool or transaction) so a job can be enqueued
//! in the SAME transaction that writes the data it processes, giving
//! exactly-once handoff with no outbox.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgExecutor, PgPool};
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    pub state: String,
    pub priority: i16,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
}

/// Options for enqueuing a job.
#[derive(Debug, Default)]
pub struct EnqueueOpts {
    pub priority: i16,
    /// Delay before the job becomes claimable.
    pub delay_secs: Option<i64>,
    /// Uniqueness key: while a job with this `dedupe_key` is queued/running,
    /// duplicate enqueues are silently dropped (used to debounce, e.g. one
    /// synthesis per cluster).
    pub dedupe_key: Option<String>,
    pub max_attempts: Option<i32>,
}

/// Enqueue a job. Returns the job id, or `None` if a `dedupe_key` collision meant
/// the job already exists.
pub async fn enqueue<'e, E>(
    executor: E,
    kind: &str,
    payload: &Value,
    opts: &EnqueueOpts,
) -> Result<Option<Uuid>>
where
    E: PgExecutor<'e>,
{
    let id = tessera_core::new_id();
    let delay = opts.delay_secs.unwrap_or(0);
    let max_attempts = opts.max_attempts.unwrap_or(5);
    let created = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO jobs (id, kind, payload, priority, run_at, max_attempts, dedupe_key)
         VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5), $6, $7)
         ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL AND state IN ('queued','running')
         DO NOTHING
         RETURNING id",
    )
    .bind(id)
    .bind(kind)
    .bind(payload)
    .bind(opts.priority)
    .bind(delay as f64)
    .bind(max_attempts)
    .bind(opts.dedupe_key.as_deref())
    .fetch_optional(executor)
    .await
    .map_err(map_sqlx)?;
    Ok(created)
}

/// Claim the next ready job for `worker_id`, leasing it for `lease_secs`.
pub async fn claim(pool: &PgPool, worker_id: &str, lease_secs: i64) -> Result<Option<Job>> {
    sqlx::query_as::<_, Job>(
        "UPDATE jobs SET
             state = 'running',
             locked_by = $1,
             locked_until = now() + make_interval(secs => $2),
             attempts = attempts + 1,
             updated_at = now()
         WHERE id = (
             SELECT id FROM jobs
             WHERE state = 'queued' AND run_at <= now()
             ORDER BY priority DESC, run_at, id
             FOR UPDATE SKIP LOCKED
             LIMIT 1)
         RETURNING id, kind, payload, state, priority, attempts, max_attempts, last_error",
    )
    .bind(worker_id)
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Extend a running job's lease. Guarded by `worker_id` so only the holder can
/// heartbeat. Returns false if the job is no longer held by this worker.
pub async fn heartbeat(
    pool: &PgPool,
    job_id: Uuid,
    worker_id: &str,
    lease_secs: i64,
) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE jobs SET locked_until = now() + make_interval(secs => $3), updated_at = now()
         WHERE id = $1 AND locked_by = $2 AND state = 'running'",
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(lease_secs as f64)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n > 0)
}

/// Mark a job done.
pub async fn complete(pool: &PgPool, job_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE jobs SET state = 'done', finished_at = now(), updated_at = now(),
             locked_by = NULL, locked_until = NULL
         WHERE id = $1",
    )
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Record a failure. Requeues with backoff if attempts remain, else dead-letters.
pub async fn fail(pool: &PgPool, job_id: Uuid, error: &str, backoff_secs: i64) -> Result<()> {
    sqlx::query(
        "UPDATE jobs SET
             state = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
             run_at = now() + make_interval(secs => $3),
             last_error = $2,
             locked_by = NULL,
             locked_until = NULL,
             finished_at = CASE WHEN attempts >= max_attempts THEN now() ELSE NULL END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(job_id)
    .bind(error)
    .bind(backoff_secs as f64)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Requeue jobs whose lease expired (their worker died). Returns how many.
pub async fn reap_expired(pool: &PgPool) -> Result<u64> {
    let n = sqlx::query(
        "UPDATE jobs SET
             state = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
             locked_by = NULL,
             locked_until = NULL,
             last_error = 'lease expired (worker died)',
             finished_at = CASE WHEN attempts >= max_attempts THEN now() ELSE NULL END,
             updated_at = now()
         WHERE state = 'running' AND locked_until < now()",
    )
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n)
}

/// Fetch one job by id (for status endpoints).
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<JobStatus>> {
    sqlx::query_as::<_, JobStatus>(
        "SELECT id, kind, state, attempts, max_attempts, last_error, created_at, finished_at
         FROM jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// A user-facing job status view.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct JobStatus {
    pub id: Uuid,
    pub kind: String,
    pub state: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Counts of queued/running/dead jobs (for backpressure and diagnostics).
pub async fn depth(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs WHERE state = 'queued'")
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)
}

/// Emit a `NOTIFY` on the tessera events channel. Used for live progress; the
/// payload is small JSON (ids and counts, never content).
pub async fn notify<'e, E>(executor: E, payload: &Value) -> Result<()>
where
    E: PgExecutor<'e>,
{
    // pg_notify is safer than raw NOTIFY here because the payload is bound, not
    // interpolated into SQL.
    sqlx::query("SELECT pg_notify('tessera_events', $1)")
        .bind(payload.to_string())
        .execute(executor)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}
