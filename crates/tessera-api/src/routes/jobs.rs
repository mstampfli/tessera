//! Job status endpoint.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use tessera_core::error::Error;
use tessera_db::queue::JobStatus;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/jobs/{id}", get(get_job))
}

async fn get_job(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatus>, ApiError> {
    ctx.require(Scope::Read)?;
    let job = tessera_db::queue::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("job")))?;
    Ok(Json(job))
}
