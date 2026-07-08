//! API token management (admin scope). The web UI settings page and the CLI both
//! create/list/revoke tokens; the secret is shown exactly once, on creation.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tessera_core::error::{Error, ErrorKind};
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tokens", get(list).post(create))
        .route("/tokens/{id}", axum::routing::delete(revoke))
}

#[derive(Debug, Deserialize)]
struct CreateTokenRequest {
    name: String,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CreatedToken {
    id: Uuid,
    prefix: String,
    /// The full token string. Returned once and never again.
    token: String,
    scopes: Vec<String>,
}

async fn create(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<CreatedToken>, ApiError> {
    ctx.require(Scope::Admin)?;

    // Validate scopes up front so we never persist a token with an unknown scope.
    let mut scopes = Vec::with_capacity(req.scopes.len());
    for s in &req.scopes {
        let scope = Scope::parse(s).ok_or_else(|| {
            ApiError(Error::new(
                ErrorKind::Invalid,
                format!("unknown scope: {s}"),
            ))
        })?;
        scopes.push(scope.as_str().to_string());
    }
    if scopes.is_empty() {
        return Err(ApiError(Error::new(
            ErrorKind::Invalid,
            "at least one scope is required",
        )));
    }

    let minted = tessera_core::secret::generate_api_token();
    let record = tessera_db::repos::api_tokens::create(
        &state.db.api,
        ctx.user_id,
        &req.name,
        &minted.prefix,
        &minted.hash,
        &scopes,
        None,
    )
    .await?;

    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "token.create",
        Some(&record.id.to_string()),
        &serde_json::json!({ "name": req.name, "scopes": scopes }),
    )
    .await;

    Ok(Json(CreatedToken {
        id: record.id,
        prefix: record.prefix,
        token: minted.plaintext,
        scopes,
    }))
}

#[derive(Debug, Serialize)]
struct TokenSummary {
    id: Uuid,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked: bool,
}

async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<TokenSummary>>, ApiError> {
    ctx.require(Scope::Admin)?;
    let rows = tessera_db::repos::api_tokens::list(&state.db.api, ctx.user_id).await?;
    let out = rows
        .into_iter()
        .map(|t| TokenSummary {
            id: t.id,
            name: t.name,
            prefix: t.prefix,
            scopes: t.scopes,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            revoked: t.revoked_at.is_some(),
        })
        .collect();
    Ok(Json(out))
}

async fn revoke(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ctx.require(Scope::Admin)?;
    let did = tessera_db::repos::api_tokens::revoke(&state.db.api, ctx.user_id, id).await?;
    if !did {
        return Err(ApiError(Error::not_found("token")));
    }
    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "token.revoke",
        Some(&id.to_string()),
        &serde_json::json!({}),
    )
    .await;
    Ok(Json(serde_json::json!({ "revoked": true })))
}
