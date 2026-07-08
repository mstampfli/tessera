//! Session auth for the web UI: login, logout, and whoami.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tessera_core::error::Error;
use tessera_core::secret;

use crate::auth::{cookie_from_headers, AuthContext, Principal, SESSION_COOKIE};
use crate::error::ApiError;
use crate::AppState;

/// Web session lifetime.
const SESSION_TTL_DAYS: i64 = 30;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

fn set_cookie(value: &str, secure: bool, max_age_secs: i64) -> String {
    let mut c =
        format!("{SESSION_COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}");
    if secure {
        c.push_str("; Secure");
    }
    c
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = tessera_db::repos::users::by_username(&state.db.api, &req.username)
        .await?
        // Uniform error whether the user is missing or the password is wrong, so
        // login does not reveal which usernames exist.
        .ok_or_else(|| ApiError(Error::unauthorized("invalid credentials")))?;

    if !secret::verify_password(&req.password, &user.password_hash)? {
        return Err(ApiError(Error::unauthorized("invalid credentials")));
    }

    let new_session = secret::generate_session();
    tessera_db::repos::sessions::create(
        &state.db.api,
        user.id,
        &new_session.hash,
        SESSION_TTL_DAYS,
    )
    .await?;

    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&format!("user:{}", user.id)),
        "login",
        None,
        &json!({}),
    )
    .await;

    let cookie = set_cookie(
        &new_session.cookie_value,
        state.config.server.secure_cookies,
        SESSION_TTL_DAYS * 86_400,
    );
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "user_id": user.id, "username": user.username })),
    ))
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    ctx: AuthContext,
) -> Result<impl IntoResponse, ApiError> {
    // Truly invalidate the session server-side (not just clear the cookie) so a
    // stolen cookie is dead the moment its owner logs out. Only cookie sessions
    // have a server-side row; a bearer logout just clears the (absent) cookie.
    if let Principal::User = ctx.principal {
        if let Some(cookie) = cookie_from_headers(&headers, SESSION_COOKIE) {
            if let Ok(hash) = secret::hash_session_cookie(&cookie) {
                tessera_db::repos::sessions::delete(&state.db.api, &hash).await?;
            }
        }
    }
    let cleared = set_cookie("", state.config.server.secure_cookies, 0);
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cleared)],
        Json(json!({ "ok": true })),
    ))
}

#[derive(Debug, Serialize)]
struct MeResponse {
    user_id: uuid::Uuid,
    username: String,
    principal: &'static str,
    scopes: Vec<String>,
}

async fn me(State(state): State<AppState>, ctx: AuthContext) -> Result<Json<MeResponse>, ApiError> {
    let user = tessera_db::repos::users::by_id(&state.db.api, ctx.user_id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("user")))?;

    let (principal, scopes) = match &ctx.principal {
        Principal::User => (
            "user",
            vec!["read".into(), "ingest".into(), "mcp".into(), "admin".into()],
        ),
        Principal::Token { scopes, .. } => (
            "token",
            scopes.iter().map(|s| s.as_str().to_string()).collect(),
        ),
    };

    Ok(Json(MeResponse {
        user_id: user.id,
        username: user.username,
        principal,
        scopes,
    }))
}
