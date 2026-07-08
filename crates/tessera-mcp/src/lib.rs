//! The MCP server: tessera as an agent-drivable tool.
//!
//! Exposes ingest, search, ask, list-insights, entity-neighborhood, and
//! job-status as MCP tools. Every tool is a thin delegate to the same service
//! layer the REST API uses, so the human surface and the agent surface cannot
//! drift. Transport is stdio (line-delimited JSON-RPC 2.0), for local agents
//! configured with `claude mcp add tessera -- tesserad mcp-stdio`.
//!
//! Implemented directly against the well-specified protocol rather than through
//! an SDK, so the server is dependency-light and fully under our control.

mod tools;

use std::sync::Arc;

use tessera_core::config::Config;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_db::cas::CasStore;
use tessera_db::repos::embeddings::{self, EmbeddingSpace};
use tessera_db::Db;
use tessera_providers::{EmbeddingProvider, LlmProvider};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

/// The service state the MCP tools operate over.
pub struct McpState {
    pub db: Db,
    pub cas: CasStore,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub llm: Arc<dyn LlmProvider>,
    pub space: EmbeddingSpace,
    /// The source that agent-ingested documents are attached to.
    pub source_id: Uuid,
}

impl McpState {
    /// Build the state from configuration (mirrors the server's provider and
    /// embedding-space setup so the stdio server is self-sufficient).
    pub async fn connect(config: &Config) -> Result<Self> {
        let db = Db::connect(
            &config.database.url,
            config.database.max_connections,
            config.database.worker_max_connections,
        )
        .await?;
        let cas = CasStore::open(&config.cas.path)?;

        let embedder = tessera_providers::build_embedder(&config.providers)
            .await
            .map_err(|e| Error::new(ErrorKind::Provider, e.to_string()))?;
        let info = embedder.space().clone();
        let mut space = embeddings::ensure(
            &db.api,
            &info.name,
            &config.providers.embedder,
            i32::try_from(info.dim).unwrap_or(i32::MAX),
            info.metric,
        )
        .await?;
        embeddings::ensure_hnsw_index(&db.api, space.id, space.dim).await?;
        embeddings::set_active(&db.api, space.id).await?;
        space.active = true;

        let llm = tessera_providers::build_llm(&config.providers)
            .map_err(|e| Error::new(ErrorKind::Provider, e.to_string()))?;

        // A single source for everything this agent ingests.
        let source = tessera_db::repos::sources::create(
            &db.api,
            "agent",
            "mcp",
            &serde_json::json!({ "transport": "stdio" }),
        )
        .await?;

        Ok(Self {
            db,
            cas,
            embedder,
            llm,
            space,
            source_id: source.id,
        })
    }

    /// Build the state from already-initialized pieces (used by the HTTP MCP
    /// transport, which shares the running server's providers and DB).
    #[must_use]
    pub fn from_parts(
        db: Db,
        cas: CasStore,
        embedder: Arc<dyn EmbeddingProvider>,
        llm: Arc<dyn LlmProvider>,
        space: EmbeddingSpace,
        source_id: Uuid,
    ) -> Self {
        Self {
            db,
            cas,
            embedder,
            llm,
            space,
            source_id,
        }
    }
}

/// Handle one JSON-RPC request, returning the response, or `None` for a
/// notification (no id, no reply). Used by the HTTP transport.
pub async fn dispatch_request(
    state: &McpState,
    request: &serde_json::Value,
) -> Option<serde_json::Value> {
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    let id = request.get("id").cloned()?;
    Some(handle(state, method, request.get("params"), id).await)
}

/// Run the stdio JSON-RPC loop until stdin closes.
pub async fn run_stdio(state: McpState) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| Error::new(ErrorKind::Io, format!("read stdin: {e}")))?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<serde_json::Value>(line) else {
            write_message(&mut stdout, &parse_error()).await?;
            continue;
        };

        // Notifications (no `id`) get no response.
        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default();

        if id.is_none() {
            // e.g. notifications/initialized: acknowledge by doing nothing.
            continue;
        }
        let id = id.unwrap();

        let response = handle(&state, method, request.get("params"), id).await;
        write_message(&mut stdout, &response).await?;
    }
    Ok(())
}

async fn handle(
    state: &McpState,
    method: &str,
    params: Option<&serde_json::Value>,
    id: serde_json::Value,
) -> serde_json::Value {
    match method {
        "initialize" => {
            let client_version = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            ok(
                id,
                serde_json::json!({
                    "protocolVersion": client_version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "tessera", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
        }
        "ping" => ok(id, serde_json::json!({})),
        "tools/list" => ok(id, serde_json::json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            match tools::call(state, name, &args).await {
                Ok(text) => ok(
                    id,
                    serde_json::json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    }),
                ),
                Err(msg) => ok(
                    id,
                    serde_json::json!({
                        "content": [{ "type": "text", "text": msg }],
                        "isError": true
                    }),
                ),
            }
        }
        _ => error(id, -32601, "method not found"),
    }
}

fn ok(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn parse_error() -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } })
}

async fn write_message(stdout: &mut tokio::io::Stdout, message: &serde_json::Value) -> Result<()> {
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    stdout
        .write_all(line.as_bytes())
        .await
        .map_err(|e| Error::new(ErrorKind::Io, format!("write stdout: {e}")))?;
    stdout
        .flush()
        .await
        .map_err(|e| Error::new(ErrorKind::Io, format!("flush: {e}")))?;
    Ok(())
}
