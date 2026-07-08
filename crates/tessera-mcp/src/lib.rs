//! MCP server (the agent-facing surface).
//!
//! Exposes tessera to AI agents as MCP tools (`tessera_ingest`,
//! `tessera_search`, `tessera_ask`, `tessera_list_insights`,
//! `tessera_get_entity_neighborhood`, `tessera_job_status`). Every tool is a
//! thin delegate to the same service layer the REST API calls, so the two
//! surfaces can never drift. Transports: streamable HTTP mounted on the main
//! server under API-token auth, and a stdio subcommand for local agents.
//!
//! Built on the official `rmcp` SDK; because it is pre-1.0, all rmcp types are
//! confined to this crate so SDK churn touches nothing else. Lands in M4; M0 is
//! a placeholder so the workspace DAG is complete.

/// Placeholder marker retained until the MCP tools land in M4.
#[must_use]
pub const fn planned() -> &'static str {
    "mcp tools land in M4"
}
