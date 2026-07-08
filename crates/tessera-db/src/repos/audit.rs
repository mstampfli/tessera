//! The append-only `audit_log` repository (repudiation mitigation).
//!
//! Records WHO did WHAT to WHICH resource and WHEN. `detail` carries ids and
//! metadata only, never document content or secrets.

use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;

use crate::map_sqlx;

/// Append one audit entry. Best-effort at the call site: a failure to audit is
/// logged but must not mask the primary operation's result.
pub async fn record(
    pool: &PgPool,
    principal: Option<&str>,
    action: &str,
    target: Option<&str>,
    detail: &Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (principal, action, target, detail)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(principal)
    .bind(action)
    .bind(target)
    .bind(detail)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}
