//! HTTP error rendering.
//!
//! [`ApiError`] wraps the workspace [`tessera_core::Error`] so we can implement
//! axum's `IntoResponse` for it (the orphan rule forbids implementing a foreign
//! trait for the foreign core error directly). Responses are RFC 9457
//! problem+json. Server-fault (5xx) detail is logged, never sent to the client.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tessera_core::error::{Error, ErrorKind};

/// Newtype wrapper giving the core error an HTTP representation.
#[derive(Debug)]
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let kind = self.0.kind();
        let status =
            StatusCode::from_u16(kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // 5xx: log the full detail (incl. any source chain), return a generic
        // message. 4xx: the message describes a client fault and is safe to send.
        let detail = if kind.detail_is_public() {
            self.0.message().to_string()
        } else {
            tracing::error!(error = %self.0, kind = kind.slug(), "request failed");
            "internal error".to_string()
        };

        let body = json!({
            "type": format!("https://tessera.mstampfli.com/errors/{}", kind.slug()),
            "title": kind.slug(),
            "status": status.as_u16(),
            "detail": detail,
        });
        (status, Json(body)).into_response()
    }
}

/// Helper so handlers can early-return a typed error succinctly.
pub fn err(kind: ErrorKind, msg: impl Into<String>) -> ApiError {
    ApiError(Error::new(kind, msg))
}
