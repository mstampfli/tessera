//! Insight synthesis: turn a cluster the math already grouped into an actionable,
//! cited card. The model only explains; it never decides what belongs together.
//!
//! The leash: the narrative must cite `[E#]` markers that map to the context
//! chunks provided, and only citations that resolve are kept. The model output is
//! parsed as data (strict JSON), never executed.

use std::collections::BTreeSet;

use serde::Deserialize;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_db::repos::clusters::MemberChunk;
use tessera_providers::{GenRequest, LlmProvider};

const SYSTEM: &str = "You are a security intelligence analyst. You are given a group of related \
excerpts that an algorithm has already clustered together, and the notable entities in them. \
Write ONE actionable insight about what this group is and what to do. \
Respond with STRICT JSON only, no prose outside the JSON, with exactly these fields: \
title (string, short), narrative (string; cite supporting excerpts inline as [E1], [E2], etc; \
every claim must have a citation and you may only use the E-numbers provided), \
severity (one of: info, low, medium, high, critical), confidence (number 0 to 1), \
suggested_actions (array of short imperative strings). \
Use only the provided context; invent nothing.";

/// The parsed model output.
#[derive(Debug, Deserialize)]
struct RawInsight {
    title: String,
    narrative: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    suggested_actions: Vec<String>,
}

/// A validated, cited insight ready to persist.
pub struct Synthesized {
    pub title: String,
    pub narrative: String,
    pub severity: String,
    pub confidence: f32,
    pub suggested_actions: Vec<String>,
    /// The 1-based context indices the narrative actually cites and that resolve.
    pub cited: Vec<usize>,
    pub model: String,
}

const ALLOWED_SEVERITY: [&str; 5] = ["info", "low", "medium", "high", "critical"];

/// Synthesize an insight over a cluster's representative chunks. Retries once if
/// the model does not return parseable JSON.
pub async fn synthesize(
    llm: &std::sync::Arc<dyn LlmProvider>,
    chunks: &[MemberChunk],
    entities: &[(String, String)],
) -> Result<Synthesized> {
    let prompt = build_prompt(chunks, entities);

    let mut last_err = String::new();
    for attempt in 0..2 {
        let system = if attempt == 0 {
            SYSTEM.to_string()
        } else {
            format!("{SYSTEM}\nYour previous reply was not valid JSON. Reply with JSON only.")
        };
        let resp = llm
            .generate(&GenRequest {
                prompt: prompt.clone(),
                system: Some(system),
                max_tokens: Some(700),
            })
            .await
            .map_err(|e| Error::new(ErrorKind::Provider, format!("synthesis generation: {e}")))?;

        match parse(&resp.text) {
            Ok(raw) => {
                let severity = if ALLOWED_SEVERITY.contains(&raw.severity.as_str()) {
                    raw.severity
                } else {
                    "medium".to_string()
                };
                let cited: Vec<usize> = referenced_markers(&raw.narrative, chunks.len())
                    .into_iter()
                    .collect();
                return Ok(Synthesized {
                    title: raw.title.trim().to_string(),
                    narrative: raw.narrative.trim().to_string(),
                    severity,
                    confidence: raw.confidence.clamp(0.0, 1.0),
                    suggested_actions: raw.suggested_actions,
                    cited,
                    model: resp.model,
                });
            }
            Err(e) => last_err = e,
        }
    }
    Err(Error::new(
        ErrorKind::Provider,
        format!("could not parse insight JSON: {last_err}"),
    ))
}

fn build_prompt(chunks: &[MemberChunk], entities: &[(String, String)]) -> String {
    use std::fmt::Write as _;
    let mut ctx = String::new();
    for (i, c) in chunks.iter().enumerate() {
        let marker = i + 1;
        let title = c.title.as_deref().unwrap_or("untitled");
        let excerpt: String = c.text.chars().take(600).collect();
        let _ = write!(ctx, "[E{marker}] (from \"{title}\")\n{excerpt}\n\n");
    }
    let ent_list = if entities.is_empty() {
        "none".to_string()
    } else {
        entities
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Notable entities in this group: {ent_list}\n\nExcerpts:\n{ctx}\n\
         Write the JSON insight now."
    )
}

/// Extract the outermost JSON object from the model text (models sometimes wrap
/// it in prose or markdown fences) and parse it.
fn parse(text: &str) -> std::result::Result<RawInsight, String> {
    let start = text.find('{').ok_or("no JSON object found")?;
    let end = text.rfind('}').ok_or("no closing brace")?;
    if end <= start {
        return Err("malformed JSON bounds".into());
    }
    serde_json::from_str::<RawInsight>(&text[start..=end]).map_err(|e| e.to_string())
}

/// Distinct `[E<number>]` markers within `[1, max]` that the narrative cites.
fn referenced_markers(narrative: &str, max: usize) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let bytes = narrative.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && (bytes[i + 1] == b'E' || bytes[i + 1] == b'e') {
            let mut j = i + 2;
            let mut num = 0usize;
            let mut saw = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                num = num * 10 + usize::from(bytes[j] - b'0');
                saw = true;
                j += 1;
            }
            if saw && j < bytes.len() && bytes[j] == b']' && num >= 1 && num <= max {
                out.insert(num);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse, referenced_markers};

    #[test]
    fn parses_json_wrapped_in_prose() {
        let text = "Sure! Here is the insight:\n```json\n{\"title\":\"t\",\"narrative\":\"n [E1]\",\"severity\":\"high\",\"confidence\":0.8,\"suggested_actions\":[\"block it\"]}\n```\nDone.";
        let raw = parse(text).expect("parses");
        assert_eq!(raw.title, "t");
        assert_eq!(raw.severity, "high");
    }

    #[test]
    fn markers_are_bounded_and_deduped() {
        let got = referenced_markers("a [E1] b [E3] c [E1] d [E9]", 3);
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec![1, 3]);
    }
}
