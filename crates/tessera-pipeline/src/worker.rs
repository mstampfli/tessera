//! The worker runtime: a pool of claim-and-run loops plus a lease reaper.

use std::time::Duration;

use tessera_db::queue::{self, Job};
use tessera_db::Db;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::context::PipelineContext;
use crate::stages;

/// Lease duration for a claimed job. A worker that dies loses its lease after
/// this and the job is reaped back to `queued`.
const LEASE_SECS: i64 = 60;
/// How long to wait before polling again when the queue is empty.
const IDLE_POLL: Duration = Duration::from_millis(500);
/// How often the reaper requeues expired leases.
const REAP_INTERVAL: Duration = Duration::from_secs(30);
/// Retry backoff cap.
const BACKOFF_CAP_SECS: i64 = 600;

/// A running pipeline; call [`PipelineHandle::shutdown`] to stop it gracefully.
pub struct PipelineHandle {
    shutdown: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl PipelineHandle {
    /// Signal all workers to stop and wait for in-flight jobs to finish or
    /// checkpoint (via lease expiry).
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Start `workers` worker tasks plus a reaper. Returns a handle for shutdown.
#[must_use]
pub fn run_pipeline(ctx: PipelineContext, workers: usize) -> PipelineHandle {
    let shutdown = CancellationToken::new();
    let mut tasks = Vec::with_capacity(workers + 1);

    for i in 0..workers.max(1) {
        let worker_id = format!("worker-{i}-{}", tessera_core::new_id().simple());
        let ctx = ctx.clone();
        let stop = shutdown.clone();
        tasks.push(tokio::spawn(worker_loop(ctx, worker_id, stop)));
    }

    let db = ctx.db.clone();
    let stop = shutdown.clone();
    tasks.push(tokio::spawn(reaper_loop(db, stop)));

    tracing::info!(workers, "pipeline started");
    PipelineHandle { shutdown, tasks }
}

async fn worker_loop(ctx: PipelineContext, worker_id: String, shutdown: CancellationToken) {
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        match queue::claim(&ctx.db.worker, &worker_id, LEASE_SECS).await {
            Ok(Some(job)) => run_job(&ctx, &worker_id, job).await,
            Ok(None) => {
                // Nothing ready; sleep briefly or wake early on shutdown.
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(IDLE_POLL) => {}
                }
            }
            Err(e) => {
                tracing::error!(worker = %worker_id, error = %e, "job claim failed");
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(IDLE_POLL) => {}
                }
            }
        }
    }
    tracing::debug!(worker = %worker_id, "worker stopped");
}

async fn run_job(ctx: &PipelineContext, worker_id: &str, job: Job) {
    let job_id = job.id;
    let kind = job.kind.clone();
    let heartbeat_secs = u64::try_from((LEASE_SECS / 3).max(1)).unwrap_or(20);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(heartbeat_secs));
    heartbeat.tick().await; // consume the immediate first tick

    let work = stages::dispatch(ctx, &job.kind, &job.payload);
    tokio::pin!(work);

    loop {
        tokio::select! {
            result = &mut work => {
                match result {
                    Ok(()) => {
                        if let Err(e) = queue::complete(&ctx.db.worker, job_id).await {
                            tracing::error!(job = %job_id, error = %e, "failed to mark job done");
                        }
                    }
                    Err(e) => {
                        let backoff = backoff_secs(job.attempts);
                        tracing::warn!(job = %job_id, kind = %kind, attempt = job.attempts, error = %e, "job failed, will retry");
                        if let Err(e2) = queue::fail(&ctx.db.worker, job_id, &e.to_string(), backoff).await {
                            tracing::error!(job = %job_id, error = %e2, "failed to record job failure");
                        }
                    }
                }
                break;
            }
            _ = heartbeat.tick() => {
                match queue::heartbeat(&ctx.db.worker, job_id, worker_id, LEASE_SECS).await {
                    Ok(true) => {}
                    Ok(false) => {
                        // We lost the lease (reaped as stale). Stop working; the
                        // job is back in the queue for someone else.
                        tracing::warn!(job = %job_id, "lost lease mid-job, abandoning");
                        break;
                    }
                    Err(e) => tracing::error!(job = %job_id, error = %e, "heartbeat failed"),
                }
            }
        }
    }
}

async fn reaper_loop(db: Db, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(REAP_INTERVAL);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = interval.tick() => {
                match queue::reap_expired(&db.worker).await {
                    Ok(n) if n > 0 => tracing::warn!(reaped = n, "requeued jobs with expired leases"),
                    Ok(_) => {}
                    Err(e) => tracing::error!(error = %e, "reaper failed"),
                }
            }
        }
    }
}

/// Exponential backoff, capped. (Jitter is added with the circuit breaker in M3.)
fn backoff_secs(attempts: i32) -> i64 {
    let exp = attempts.clamp(1, 20) - 1;
    (5_i64.saturating_mul(1_i64 << exp.min(12))).min(BACKOFF_CAP_SECS)
}
