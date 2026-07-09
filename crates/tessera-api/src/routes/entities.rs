//! Entity endpoints: list/filter, detail (with correlation neighborhood), and
//! the neighborhood on its own.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tessera_core::error::Error;
use tessera_db::repos::entities;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/entities", get(list))
        .route("/entities/{id}", get(detail))
        .route("/entities/{id}/neighborhood", get(neighborhood))
        .route("/graph", get(graph))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct EntityView {
    id: Uuid,
    kind: String,
    value: String,
    display_value: String,
    mention_count: i64,
    first_seen: chrono::DateTime<chrono::Utc>,
    last_seen: chrono::DateTime<chrono::Utc>,
}

fn view(e: entities::Entity) -> EntityView {
    EntityView {
        id: e.id,
        kind: e.kind,
        value: e.value,
        display_value: e.display_value,
        mention_count: e.mention_count,
        first_seen: e.first_seen,
        last_seen: e.last_seen,
    }
}

async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<EntityView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = entities::list(
        &state.db.api,
        params.kind.as_deref(),
        params.q.as_deref(),
        limit,
    )
    .await?;
    Ok(Json(rows.into_iter().map(view).collect()))
}

#[derive(Debug, Serialize)]
struct NeighborView {
    id: Uuid,
    kind: String,
    value: String,
    display_value: String,
    /// `co_occurs` (direct) or `similar` (semantic).
    method: String,
    /// Correlation strength in `[0, 1]`.
    strength: f64,
}

#[derive(Debug, Serialize)]
struct DocumentRef {
    id: Uuid,
    title: Option<String>,
}

#[derive(Debug, Serialize)]
struct EntityDetail {
    entity: EntityView,
    neighborhood: Vec<NeighborView>,
    documents: Vec<DocumentRef>,
}

fn neighbor_view(n: entities::Neighbor) -> NeighborView {
    NeighborView {
        id: n.id,
        kind: n.kind,
        value: n.value,
        display_value: n.display_value,
        method: n.rel,
        strength: n.strength,
    }
}

async fn detail(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<EntityDetail>, ApiError> {
    ctx.require(Scope::Read)?;
    let entity = entities::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("entity")))?;
    let neighborhood = entities::neighborhood(&state.db.api, id, 50).await?;
    let documents = entities::documents_for(&state.db.api, id, 50).await?;

    Ok(Json(EntityDetail {
        entity: view(entity),
        neighborhood: neighborhood.into_iter().map(neighbor_view).collect(),
        documents: documents
            .into_iter()
            .map(|(id, title)| DocumentRef { id, title })
            .collect(),
    }))
}

async fn neighborhood(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NeighborView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let rows = entities::neighborhood(&state.db.api, id, 100).await?;
    Ok(Json(rows.into_iter().map(neighbor_view).collect()))
}

#[derive(Debug, Deserialize)]
struct GraphParams {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
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
struct EntityGraph {
    nodes: Vec<GraphNodeView>,
    edges: Vec<GraphEdgeView>,
    /// Total entities matching the filter, so the client can say how much of the
    /// full set a capped graph is showing.
    total: i64,
}

/// The global entity correlation graph (most-connected first, capped). Edges are
/// the persisted correlations (direct co-occurrence and global semantic kNN).
async fn graph(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<GraphParams>,
) -> Result<Json<EntityGraph>, ApiError> {
    ctx.require(Scope::Read)?;
    let kind = params.kind.as_deref().filter(|s| !s.is_empty());
    // Cap nodes so a large corpus renders as the top hubs, not a hairball.
    let cap = params.limit.unwrap_or(200).clamp(1, 1000);
    let (nodes, edges) = entities::graph(&state.db.api, kind, cap).await?;
    let total = entities::count(&state.db.api, kind).await?;

    Ok(Json(EntityGraph {
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
                method: e.method,
                strength: e.strength,
            })
            .collect(),
        total,
    }))
}
