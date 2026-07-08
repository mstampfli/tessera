//! The built-in extractors and the `normalize` dispatch that turns untrusted
//! bytes of a sniffed type into a title plus prepared chunks.
//!
//! Records (CSV rows, JSON objects) become one chunk each; prose, markdown, and
//! HTML text are split with overlap; logs are chunked into line windows.

use scraper::{Html, Selector};
use serde_json::Value;

use crate::chunk::{chunk_markdown, chunk_prose, PreparedChunk};
use crate::{ExtractError, SniffedType};

/// The output of normalization: an optional document title and its chunks.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub title: Option<String>,
    pub chunks: Vec<PreparedChunk>,
}

/// Lines per chunk when windowing log files for embedding.
const LOG_WINDOW_LINES: usize = 20;
/// A record longer than this many characters is itself split into prose chunks.
const RECORD_SPLIT_THRESHOLD: usize = 1500;

/// Turn bytes of a sniffed type into a title and chunks. All input is untrusted.
pub fn normalize(bytes: &[u8], sniff: &SniffedType) -> Result<Prepared, ExtractError> {
    match sniff.label.as_str() {
        "pdf" | "image" | "binary" => Err(ExtractError::Other(format!(
            "extraction of {} content is not supported in this build",
            sniff.label
        ))),
        label => {
            // All supported labels are UTF-8 text. Decode lossily so a stray
            // invalid byte never aborts ingestion of an otherwise good document.
            let text = String::from_utf8_lossy(bytes);
            let prepared = match label {
                "markdown" => normalize_markdown(&text),
                "html" => normalize_html(&text),
                "json" => normalize_json(&text)?,
                "ndjson" => normalize_ndjson(&text),
                "csv" => normalize_csv(&text)?,
                "log" => normalize_log(&text),
                _ => normalize_text(&text),
            };
            Ok(prepared)
        }
    }
}

/// Convert a plugin's extraction events into the same `Prepared` shape the
/// built-in extractors produce, so plugin output flows through the identical
/// downstream pipeline.
#[must_use]
pub fn events_to_prepared(events: Vec<tessera_core::ExtractEvent>) -> Prepared {
    use tessera_core::ExtractEvent;
    let mut title = None;
    let mut chunks = Vec::new();
    for ev in events {
        match ev {
            ExtractEvent::Meta { title: t, .. } => {
                if title.is_none() {
                    title = t;
                }
            }
            ExtractEvent::Text { text, .. } => chunks.extend(chunk_prose(&text)),
            ExtractEvent::Record { data } => {
                chunks.extend(record_text_chunks(&value_to_text(&data)));
            }
            ExtractEvent::Entity { .. } | ExtractEvent::Warn { .. } => {}
        }
    }
    Prepared { title, chunks }
}

fn normalize_text(text: &str) -> Prepared {
    Prepared {
        title: None,
        chunks: chunk_prose(text),
    }
}

fn normalize_markdown(text: &str) -> Prepared {
    let title = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Prepared {
        title,
        chunks: chunk_markdown(text),
    }
}

fn normalize_html(text: &str) -> Prepared {
    let doc = Html::parse_document(text);
    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| {
            doc.select(&sel)
                .next()
                .map(|t| t.text().collect::<String>())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let body_text = html_visible_text(&doc);
    Prepared {
        title,
        chunks: chunk_prose(&body_text),
    }
}

/// Collect visible text, skipping script/style/head/noscript subtrees. Walking
/// the parsed tree (rather than `Element::text`) is what lets us exclude those.
fn html_visible_text(doc: &Html) -> String {
    let mut out = String::new();
    for node in doc.tree.nodes() {
        let scraper::Node::Text(t) = node.value() else {
            continue;
        };
        let mut skip = false;
        let mut ancestor = node.parent();
        while let Some(p) = ancestor {
            if let scraper::Node::Element(e) = p.value() {
                if matches!(e.name(), "script" | "style" | "head" | "noscript") {
                    skip = true;
                    break;
                }
            }
            ancestor = p.parent();
        }
        if !skip {
            let s = t.trim();
            if !s.is_empty() {
                out.push_str(s);
                out.push(' ');
            }
        }
    }
    out.trim().to_string()
}

fn normalize_json(text: &str) -> Result<Prepared, ExtractError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| ExtractError::Malformed(format!("json: {e}")))?;
    let chunks = match value {
        // A top-level array is a collection of records.
        Value::Array(items) => items.iter().flat_map(record_chunks).collect(),
        // Anything else is a single record.
        other => record_chunks(&other),
    };
    Ok(Prepared {
        title: None,
        chunks,
    })
}

fn normalize_ndjson(text: &str) -> Prepared {
    let chunks = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .flat_map(|v| record_chunks(&v))
        .collect();
    Prepared {
        title: None,
        chunks,
    }
}

fn normalize_csv(text: &str) -> Result<Prepared, ExtractError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| ExtractError::Malformed(format!("csv header: {e}")))?
        .clone();

    let mut chunks = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| ExtractError::Malformed(format!("csv row: {e}")))?;
        let rendered = headers
            .iter()
            .zip(record.iter())
            .map(|(h, v)| format!("{}: {}", h.trim(), v.trim()))
            .collect::<Vec<_>>()
            .join("; ");
        if !rendered.trim().is_empty() {
            chunks.extend(record_text_chunks(&rendered));
        }
    }
    Ok(Prepared {
        title: None,
        chunks,
    })
}

fn normalize_log(text: &str) -> Prepared {
    let lines: Vec<&str> = text.lines().collect();
    let chunks = lines
        .chunks(LOG_WINDOW_LINES)
        .map(|window| PreparedChunk::new(window.join("\n")))
        .filter(|c| !c.text.trim().is_empty())
        .collect();
    Prepared {
        title: None,
        chunks,
    }
}

/// Render a JSON value as a readable, searchable record and chunk it.
fn record_chunks(value: &Value) -> Vec<PreparedChunk> {
    record_text_chunks(&value_to_text(value))
}

/// A record becomes one chunk, unless it is large enough to warrant splitting.
fn record_text_chunks(text: &str) -> Vec<PreparedChunk> {
    if text.chars().count() > RECORD_SPLIT_THRESHOLD {
        chunk_prose(text)
    } else if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![PreparedChunk::new(text)]
    }
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{k}: {}", scalar_text(v)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Array(items) => items
            .iter()
            .map(value_to_text)
            .collect::<Vec<_>>()
            .join("\n"),
        other => scalar_text(other),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
        // Nested composites: compact JSON keeps them searchable without noise.
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use crate::sniff::sniff;

    fn norm(bytes: &[u8]) -> super::Prepared {
        let s = sniff(bytes, None);
        normalize(bytes, &s).expect("normalize ok")
    }

    #[test]
    fn markdown_title_and_chunks() {
        let p = norm(b"# My Title\n\nSome body text here.");
        assert_eq!(p.title.as_deref(), Some("My Title"));
        assert!(!p.chunks.is_empty());
    }

    #[test]
    fn json_array_becomes_one_chunk_per_record() {
        let p = norm(br#"[{"host":"a.com","ip":"1.2.3.4"},{"host":"b.com","ip":"5.6.7.8"}]"#);
        assert_eq!(p.chunks.len(), 2);
        assert!(p.chunks[0].text.contains("host: a.com"));
        assert!(p.chunks[0].text.contains("ip: 1.2.3.4"));
    }

    #[test]
    fn csv_rows_become_records() {
        let p = norm(b"host,ip\na.com,1.2.3.4\nb.com,5.6.7.8");
        assert_eq!(p.chunks.len(), 2);
        assert!(p.chunks[1].text.contains("host: b.com"));
    }

    #[test]
    fn html_strips_script_and_style() {
        let html = b"<html><head><title>Doc</title><style>.x{color:red}</style></head><body><p>Visible text</p><script>alert('x')</script></body></html>";
        let p = norm(html);
        assert_eq!(p.title.as_deref(), Some("Doc"));
        let all: String = p.chunks.iter().map(|c| c.text.clone()).collect();
        assert!(all.contains("Visible text"), "got: {all}");
        assert!(!all.contains("alert"), "script leaked: {all}");
        assert!(!all.contains("color:red"), "style leaked: {all}");
    }

    #[test]
    fn log_windows_group_lines() {
        use std::fmt::Write as _;
        let mut log = String::new();
        for i in 0..45 {
            let _ = writeln!(log, "2026-07-08 10:00:{i:02} INFO event {i}");
        }
        let p = norm(log.as_bytes());
        // 45 lines / 20 per window = 3 chunks.
        assert_eq!(p.chunks.len(), 3);
    }

    #[test]
    fn binary_is_rejected() {
        let sniffed = crate::SniffedType {
            media_type: "application/pdf".into(),
            label: "pdf".into(),
        };
        assert!(normalize(b"%PDF-1.4", &sniffed).is_err());
    }
}
