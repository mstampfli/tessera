//! Prometheus metrics: the `/metrics` scrape endpoint and an HTTP-request
//! middleware. Job and queue metrics are recorded in the pipeline crate.
//!
//! `/metrics` is unauthenticated (it exposes counters, not data); on a private
//! tailnet that is the conventional posture for a scrape target.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(render))
}

async fn render(State(state): State<AppState>) -> String {
    // Also refresh the queue-depth gauge on scrape.
    if let Ok(depth) = tessera_db::queue::depth(&state.db.api).await {
        metrics::gauge!("tessera_queue_depth").set(depth as f64);
    }
    state.metrics.render()
}

/// Record request count and latency, labeled by method and status only (never by
/// path, which would explode cardinality with ids in the URL).
pub async fn track_http(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let start = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    let elapsed = start.elapsed().as_secs_f64();

    metrics::counter!("tessera_http_requests_total", "method" => method.clone(), "status" => status)
        .increment(1);
    metrics::histogram!("tessera_http_request_duration_seconds", "method" => method)
        .record(elapsed);
    response
}
