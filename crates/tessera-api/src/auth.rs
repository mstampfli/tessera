//! Authentication and authorization.
//!
//! One extractor, [`AuthContext`], resolves either an `Authorization: Bearer`
//! API token or a session cookie into an authenticated principal with a scope
//! set. Authorization is deny-by-default: handlers call
//! [`AuthContext::require`] for the scope they need.
//!
//! CSRF: a cookie-authenticated mutating request must also carry the
//! `X-Tessera-Csrf` header. Bearer (programmatic) requests are exempt because
//! they are not sent automatically by a browser. This is defense in depth on top
//! of the `SameSite=Lax` cookie.

use axum::extract::FromRef;
use axum::http::request::Parts;
use axum::http::{header, Method};
use chrono::Utc;
use tessera_core::error::{Error, ErrorKind};
use tessera_core::secret;
use uuid::Uuid;

use crate::error::ApiError;
use crate::AppState;

/// The session cookie name.
pub const SESSION_COOKIE: &str = "tessera_session";
/// The CSRF opt-in header cookie-authed mutations must send.
pub const CSRF_HEADER: &str = "x-tessera-csrf";

/// A capability scope. `Admin` implies every other scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Ingest,
    Mcp,
    Admin,
}

impl Scope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Ingest => "ingest",
            Scope::Mcp => "mcp",
            Scope::Admin => "admin",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "read" => Some(Scope::Read),
            "ingest" => Some(Scope::Ingest),
            "mcp" => Some(Scope::Mcp),
            "admin" => Some(Scope::Admin),
            _ => None,
        }
    }
}

/// Who is making the request.
#[derive(Debug, Clone)]
pub enum Principal {
    /// The web user, authenticated by session cookie. Holds all scopes.
    User,
    /// A program/agent, authenticated by API token, holding explicit scopes.
    Token { id: Uuid, scopes: Vec<Scope> },
}

/// The resolved authentication context for a request.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub principal: Principal,
}

impl AuthContext {
    /// A stable audit string for this principal.
    #[must_use]
    pub fn audit_id(&self) -> String {
        match &self.principal {
            Principal::User => format!("user:{}", self.user_id),
            Principal::Token { id, .. } => format!("token:{id}"),
        }
    }

    /// Enforce that this principal holds `scope`. `Admin` satisfies anything, and
    /// the web user holds all scopes.
    pub fn require(&self, scope: Scope) -> Result<(), ApiError> {
        let ok = match &self.principal {
            Principal::User => true,
            Principal::Token { scopes, .. } => {
                scopes.contains(&Scope::Admin) || scopes.contains(&scope)
            }
        };
        if ok {
            Ok(())
        } else {
            Err(ApiError(Error::new(
                ErrorKind::Forbidden,
                format!("missing required scope: {}", scope.as_str()),
            )))
        }
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extract a cookie value by name from a header map. Shared by the auth
/// extractor and the logout handler so cookie parsing lives in one place.
#[must_use]
pub fn cookie_from_headers(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    cookie_from_headers(&parts.headers, name)
}

impl<S> axum::extract::FromRequestParts<S> for AuthContext
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);

        // 1) API token (programs/agents). No CSRF requirement.
        if let Some(token) = bearer_token(parts) {
            let presented = secret::parse_api_token(&token).map_err(ApiError)?;
            let record = tessera_db::repos::api_tokens::by_prefix(&app.db.api, &presented.prefix)
                .await
                .map_err(ApiError)?
                .ok_or_else(|| ApiError(Error::unauthorized("invalid token")))?;

            if !record.is_active(Utc::now())
                || !secret::hashes_equal(&record.token_hash, &presented.presented_hash)
            {
                return Err(ApiError(Error::unauthorized("invalid token")));
            }

            // Best-effort last-used stamp; a failure must not reject the request.
            let _ = tessera_db::repos::api_tokens::touch(&app.db.api, record.id).await;

            let scopes = record
                .scopes
                .iter()
                .filter_map(|s| Scope::parse(s))
                .collect();
            return Ok(AuthContext {
                user_id: record.user_id,
                principal: Principal::Token {
                    id: record.id,
                    scopes,
                },
            });
        }

        // 2) Session cookie (web UI). Mutating requests must carry the CSRF header.
        if let Some(cookie) = cookie_value(parts, SESSION_COOKIE) {
            let hash = secret::hash_session_cookie(&cookie).map_err(ApiError)?;
            let session = tessera_db::repos::sessions::resolve_active(&app.db.api, &hash)
                .await
                .map_err(ApiError)?
                .ok_or_else(|| ApiError(Error::unauthorized("session expired")))?;

            if is_mutating(&parts.method) && parts.headers.get(CSRF_HEADER).is_none() {
                return Err(ApiError(Error::new(
                    ErrorKind::Forbidden,
                    "missing CSRF header on state-changing request",
                )));
            }

            return Ok(AuthContext {
                user_id: session.user_id,
                principal: Principal::User,
            });
        }

        Err(ApiError(Error::unauthorized("authentication required")))
    }
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}
