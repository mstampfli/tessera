//! Cluster endpoints: list and detail (with member chunks).

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tessera_core::error::Error;
use tessera_db::repos::clusters;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/clusters", get(list))
        .route("/clusters/{id}", get(detail))
        .route("/clusters/{id}/graph", get(graph))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ClusterView {
    id: Uuid,
    size: i32,
    label: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn view(c: clusters::Cluster) -> ClusterView {
    ClusterView {
        id: c.id,
        size: c.size,
        label: c.label,
        created_at: c.created_at,
        updated_at: c.updated_at,
    }
}

async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<ClusterView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = clusters::list(&state.db.api, limit).await?;
    Ok(Json(rows.into_iter().map(view).collect()))
}

#[derive(Debug, Serialize)]
struct MemberView {
    chunk_id: Uuid,
    document_id: Uuid,
    title: Option<String>,
    excerpt: String,
    similarity: f32,
}

#[derive(Debug, Serialize)]
struct ClusterDetail {
    cluster: ClusterView,
    members: Vec<MemberView>,
}

async fn detail(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ClusterDetail>, ApiError> {
    ctx.require(Scope::Read)?;
    let cluster = clusters::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("cluster")))?;
    let members = clusters::members(&state.db.api, id, 100).await?;
    Ok(Json(ClusterDetail {
        cluster: view(cluster),
        members: members
            .into_iter()
            .map(|m| MemberView {
                chunk_id: m.chunk_id,
                document_id: m.document_id,
                title: m.title,
                excerpt: m.text.chars().take(200).collect(),
                similarity: m.similarity,
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize)]
struct GraphNodeView {
    id: Uuid,
    kind: String,
    value: String,
    display_value: String,
    weight: i64,
}

#[derive(Debug, Serialize)]
struct GraphEdgeView {
    src_id: Uuid,
    dst_id: Uuid,
    /// `co_occurs` (direct, strong) or `similar` (semantic, contextual).
    method: String,
    /// Correlation strength in `[0, 1]` (edge thickness in the UI).
    strength: f64,
}

#[derive(Debug, Serialize)]
struct ClusterGraph {
    nodes: Vec<GraphNodeView>,
    edges: Vec<GraphEdgeView>,
}

/// The cluster's entity correlation network (co-occurrence + semantic), capped.
async fn graph(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ClusterGraph>, ApiError> {
    ctx.require(Scope::Read)?;
    // Cap nodes so the client graph stays renderable (plan: cluster graph only
    // under ~500 nodes).
    let (nodes, cooccurrence) = clusters::graph(&state.db.api, id, 300).await?;

    let ids: Vec<Uuid> = nodes.iter().map(|n| n.id).collect();
    let semantic = tessera_db::repos::entities::semantic_edges(
        &state.db.api,
        state.space.id,
        &ids,
        crate::routes::entities::SEMANTIC_K,
        crate::routes::entities::SEMANTIC_MIN_SIM,
    )
    .await?;
    let edges = tessera_db::repos::entities::merge_correlation_edges(&cooccurrence, &semantic);

    Ok(Json(ClusterGraph {
        nodes: nodes
            .into_iter()
            .map(|n| GraphNodeView {
                id: n.id,
                kind: n.kind,
                value: n.value,
                display_value: n.display_value,
                weight: n.weight,
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(|e| GraphEdgeView {
                src_id: e.src_id,
                dst_id: e.dst_id,
                method: e.method.to_string(),
                strength: e.strength,
            })
            .collect(),
    }))
}
