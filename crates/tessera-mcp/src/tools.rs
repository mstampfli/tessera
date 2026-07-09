//! MCP tool definitions and dispatch. Each tool is a thin delegate to the same
//! service functions the REST API uses.

use serde_json::{json, Value};
use tessera_search::SearchMode;
use uuid::Uuid;

use crate::McpState;

/// The tool catalog returned by `tools/list`.
pub fn definitions() -> Value {
    json!([
        {
            "name": "tessera_ingest",
            "description": "Ingest a piece of content (text, logs, IOCs, JSON, CSV, markdown) into the knowledge base. It is chunked, embedded, and correlated in the background.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "The content to ingest." },
                    "media_type": { "type": "string", "description": "Optional media type hint, e.g. text/markdown." },
                    "title": { "type": "string", "description": "Optional title." }
                },
                "required": ["content"]
            }
        },
        {
            "name": "tessera_search",
            "description": "Search the knowledge base. Hybrid (vector + keyword) by default; use mode 'keyword' for exact identifiers like IPs or hashes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "mode": { "type": "string", "enum": ["hybrid", "semantic", "keyword"] },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "tessera_ask",
            "description": "Ask a question over the knowledge base and get an answer with citations to the source chunks. Returns 'no evidence' when nothing on topic is found.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "k": { "type": "integer", "description": "How many context chunks to retrieve." }
                },
                "required": ["question"]
            }
        },
        {
            "name": "tessera_list_insights",
            "description": "List the current actionable insights (cited cards produced by correlating and clustering the data).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["new", "surfaced", "useful", "dismissed"] },
                    "limit": { "type": "integer" }
                }
            }
        },
        {
            "name": "tessera_get_entity_neighborhood",
            "description": "Given an indicator or identifier value (e.g. an IP, domain, or hash), return the entity and its strongest correlated entities, ranked by shared occurrences weighted by rarity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "value": { "type": "string" },
                    "kind": { "type": "string", "description": "Optional entity kind filter, e.g. ip or domain." }
                },
                "required": ["value"]
            }
        },
        {
            "name": "tessera_job_status",
            "description": "Check the status of a background pipeline job by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            }
        }
    ])
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn opt_i64(args: &Value, key: &str, default: i64, min: i64, max: i64) -> i64 {
    args.get(key)
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(min, max)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("serialize: {e}"))
}

/// Dispatch a tool call. Returns the result as a text payload, or an error
/// message (surfaced to the agent as an errored tool result).
#[allow(clippy::too_many_lines)]
pub async fn call(state: &McpState, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "tessera_ingest" => {
            let content = required_str(args, "content")?;
            let ingested = tessera_pipeline::ingest_bytes(
                &state.db,
                &state.cas,
                tessera_pipeline::IngestBytes {
                    source_id: state.source_id,
                    bytes: content.as_bytes(),
                    media_type_hint: args.get("media_type").and_then(Value::as_str),
                    title: args.get("title").and_then(Value::as_str),
                    uri: None,
                    meta: json!({ "via": "mcp" }),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            to_json(&json!({
                "document_id": ingested.document_id,
                "deduped": ingested.deduped,
                "status": ingested.status
            }))
        }

        "tessera_search" => {
            let query = required_str(args, "query")?;
            let mode =
                SearchMode::parse(args.get("mode").and_then(Value::as_str).unwrap_or("hybrid"));
            let limit = opt_i64(args, "limit", 15, 1, 50);
            let hits = tessera_search::search(
                &state.db.api,
                &state.embedder,
                Some(&state.space),
                query,
                mode,
                limit,
            )
            .await
            .map_err(|e| e.to_string())?;
            to_json(&hits)
        }

        "tessera_ask" => {
            let question = required_str(args, "question")?;
            let k = opt_i64(args, "k", 8, 1, 30);
            let answer = tessera_search::ask(
                &state.db.api,
                &state.embedder,
                &state.llm,
                Some(&state.space),
                question,
                k,
            )
            .await
            .map_err(|e| e.to_string())?;
            to_json(&answer)
        }

        "tessera_list_insights" => {
            let status = args.get("status").and_then(Value::as_str);
            let limit = opt_i64(args, "limit", 20, 1, 100);
            let insights = tessera_db::repos::insights::list(&state.db.api, status, limit)
                .await
                .map_err(|e| e.to_string())?;
            // Serialize a compact view (the repo row is not Serialize by contract).
            let view: Vec<Value> = insights
                .into_iter()
                .map(|i| {
                    json!({
                        "id": i.id, "title": i.title, "severity": i.severity,
                        "confidence": i.confidence, "narrative": i.body_md,
                        "suggested_actions": i.suggested_actions, "status": i.status
                    })
                })
                .collect();
            to_json(&view)
        }

        "tessera_get_entity_neighborhood" => {
            let value = required_str(args, "value")?;
            let kind = args.get("kind").and_then(Value::as_str);
            let matches = tessera_db::repos::entities::list(&state.db.api, kind, Some(value), 5)
                .await
                .map_err(|e| e.to_string())?;
            let entity = matches
                .into_iter()
                .find(|e| e.value.eq_ignore_ascii_case(value))
                .ok_or_else(|| format!("no entity found for value: {value}"))?;
            let neighborhood =
                tessera_db::repos::entities::neighborhood(&state.db.api, entity.id, 25)
                    .await
                    .map_err(|e| e.to_string())?;
            let neighbors: Vec<Value> = neighborhood
                .into_iter()
                .map(|n| {
                    json!({
                        "kind": n.kind, "value": n.value, "method": n.rel,
                        "strength": n.strength
                    })
                })
                .collect();
            to_json(&json!({
                "entity": { "kind": entity.kind, "value": entity.value, "mentions": entity.mention_count },
                "neighborhood": neighbors
            }))
        }

        "tessera_job_status" => {
            let job_id = required_str(args, "job_id")?;
            let id = Uuid::parse_str(job_id).map_err(|_| "invalid job_id".to_string())?;
            let job = tessera_db::queue::get(&state.db.api, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "job not found".to_string())?;
            to_json(&job)
        }

        other => Err(format!("unknown tool: {other}")),
    }
}
