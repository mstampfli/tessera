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
        .route("/bridges", get(bridges))
        .route("/correlation", get(correlation))
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
    /// Structural community (for node colouring); null before detection runs.
    community_id: Option<i32>,
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
                community_id: n.community_id,
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

#[derive(Debug, Serialize)]
struct BridgeView {
    a_id: Uuid,
    a_kind: String,
    a_value: String,
    a_community: i32,
    b_id: Uuid,
    b_kind: String,
    b_value: String,
    b_community: i32,
    strength: f64,
}

#[derive(Debug, Deserialize)]
struct CorrelationParams {
    a: Uuid,
    b: Uuid,
}

#[derive(Debug, Serialize)]
struct EvidenceView {
    chunk_id: Uuid,
    document_id: Uuid,
    title: Option<String>,
    excerpt: String,
    event_time: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<entities::EvidenceChunk> for EvidenceView {
    fn from(e: entities::EvidenceChunk) -> Self {
        Self {
            chunk_id: e.chunk_id,
            document_id: e.document_id,
            title: e.title,
            excerpt: e.excerpt,
            event_time: e.event_time,
        }
    }
}

#[derive(Debug, Serialize)]
struct CorrelationDetail {
    a: EntityView,
    b: EntityView,
    links: Vec<serde_json::Value>,
    shared_chunks: Vec<EvidenceView>,
    a_sample: Option<EvidenceView>,
    b_sample: Option<EvidenceView>,
}

/// Explain WHY two entities correlate: the methods and strengths linking them,
/// the chunks that mention both (the literal shared sentence, for co-occurrence),
/// and a representative mention of each (to ground a semantic/temporal link).
async fn correlation(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<CorrelationParams>,
) -> Result<Json<CorrelationDetail>, ApiError> {
    ctx.require(Scope::Read)?;
    let pool = &state.db.api;
    let a = entities::get(pool, params.a)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("entity")))?;
    let b = entities::get(pool, params.b)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("entity")))?;
    let links = entities::pair_links(pool, params.a, params.b).await?;
    let shared = entities::shared_chunks(pool, params.a, params.b, 3).await?;
    let a_sample = entities::sample_mention(pool, params.a).await?;
    let b_sample = entities::sample_mention(pool, params.b).await?;

    Ok(Json(CorrelationDetail {
        a: view(a),
        b: view(b),
        links: links
            .into_iter()
            .map(|l| serde_json::json!({ "method": l.method, "strength": l.strength }))
            .collect(),
        shared_chunks: shared.into_iter().map(EvidenceView::from).collect(),
        a_sample: a_sample.map(EvidenceView::from),
        b_sample: b_sample.map(EvidenceView::from),
    }))
}

/// The strongest cross-community bridges: semantic links between entities that
/// belong to different co-occurrence communities (things never stated together).
async fn bridges(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<GraphParams>,
) -> Result<Json<Vec<BridgeView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = tessera_db::repos::communities::bridges(&state.db.api, limit).await?;
    Ok(Json(
        rows.into_iter()
            .map(|b| BridgeView {
                a_id: b.a_id,
                a_kind: b.a_kind,
                a_value: b.a_value,
                a_community: b.a_community,
                b_id: b.b_id,
                b_kind: b.b_kind,
                b_value: b.b_value,
                b_community: b.b_community,
                strength: b.strength,
            })
            .collect(),
    ))
}
