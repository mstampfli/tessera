//! Ask-with-citations (RAG).
//!
//! Retrieval finds the evidence; the model only explains it, on a leash: the
//! answer must cite `[C#]` markers that map to the provided context chunks, and
//! we keep only citations that resolve. If retrieval finds nothing, we return
//! "no evidence" rather than letting the model invent an answer.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::Serialize;
use sqlx::PgPool;
use tessera_core::error::Result;
use tessera_db::repos::embeddings::EmbeddingSpace;
use tessera_providers::{EmbeddingProvider, GenRequest, LlmProvider};

use crate::{search, SearchHit, SearchMode};

const SYSTEM_PROMPT: &str = "You are a precise analyst. Answer ONLY from the provided context. \
You MUST place a citation marker in square brackets, like [C1] or [C3], immediately after \
every sentence, naming the context item(s) that support it. Never write a sentence without a \
citation marker. Use only the item numbers that appear in the context. Do not use any outside \
knowledge. If the context does not answer the question, reply that you have no evidence. \
Example of the required style: \"The host is a known C2 server [C2] and it hosts a phishing kit [C1].\" \
Be concise.";

/// A resolved citation backing part of an answer.
#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub marker: String,
    pub chunk_id: uuid::Uuid,
    pub document_id: uuid::Uuid,
    pub seq: i32,
    pub title: Option<String>,
    pub excerpt: String,
}

/// An answer plus the citations that survived resolution.
#[derive(Debug, Clone, Serialize)]
pub struct AskAnswer {
    pub answer: String,
    pub citations: Vec<Citation>,
    /// How many context chunks were provided to the model.
    pub context_used: usize,
}

/// Two-level relevance gate. kNN always returns nearest neighbors, so a floor is
/// needed to distinguish "on topic" from "closest of whatever exists".
///
/// `QUERY_FLOOR`: the whole query is only on-topic if its single best semantic
/// hit is at least this close (or there is an exact keyword match). This is what
/// makes an unrelated question return "no evidence".
/// `INCLUDE_MAX`: once on-topic, include neighbors out to this looser bound so
/// supporting chunks that are relevant but not the single closest still count.
const QUERY_FLOOR: f64 = 0.45;
const INCLUDE_MAX: f64 = 0.55;

/// Answer a question over the knowledge base with citations.
pub async fn ask(
    pool: &PgPool,
    embedder: &Arc<dyn EmbeddingProvider>,
    llm: &Arc<dyn LlmProvider>,
    space: Option<&EmbeddingSpace>,
    question: &str,
    k: i64,
) -> Result<AskAnswer> {
    let mut hits = search(pool, embedder, space, question, SearchMode::Hybrid, k).await?;

    // Is anything actually on topic? A keyword hit is an exact identifier match;
    // otherwise the best semantic hit must clear the query floor.
    let has_keyword = hits.iter().any(|h| h.keyword);
    let best_distance = hits
        .iter()
        .filter_map(|h| h.distance)
        .fold(f64::INFINITY, f64::min);
    let on_topic = has_keyword || best_distance <= QUERY_FLOOR;

    if on_topic {
        hits.retain(|h| h.keyword || h.distance.is_some_and(|d| d <= INCLUDE_MAX));
    } else {
        hits.clear();
    }

    if hits.is_empty() {
        return Ok(AskAnswer {
            answer: "I have no evidence in the knowledge base to answer that.".to_string(),
            citations: Vec::new(),
            context_used: 0,
        });
    }

    let prompt = build_prompt(question, &hits);
    let resp = llm
        .generate(&GenRequest {
            prompt,
            system: Some(SYSTEM_PROMPT.to_string()),
            max_tokens: Some(700),
        })
        .await
        .map_err(|e| {
            tessera_core::error::Error::new(
                tessera_core::error::ErrorKind::Provider,
                format!("ask generation: {e}"),
            )
        })?;

    // The leash: keep only citations whose marker maps to a real context chunk.
    let referenced = referenced_markers(&resp.text, hits.len());
    let citations = referenced
        .into_iter()
        .filter_map(|idx| hits.get(idx - 1).map(|h| citation_from_hit(idx, h)))
        .collect();

    Ok(AskAnswer {
        answer: resp.text,
        citations,
        context_used: hits.len(),
    })
}

fn build_prompt(question: &str, hits: &[SearchHit]) -> String {
    use std::fmt::Write as _;
    let mut ctx = String::new();
    for (i, h) in hits.iter().enumerate() {
        let marker = i + 1;
        let title = h.title.as_deref().unwrap_or("untitled");
        // Bound each context item so a pathological chunk cannot blow the budget.
        let excerpt: String = h.text.chars().take(1200).collect();
        let _ = write!(ctx, "[C{marker}] (from \"{title}\")\n{excerpt}\n\n");
    }
    format!(
        "Context:\n{ctx}\nQuestion: {question}\n\n\
         Write the answer now. Every sentence must end with its [C#] citation marker(s):"
    )
}

/// Parse the distinct `[C<number>]` markers the answer actually references,
/// keeping only those within `[1, max]`.
fn referenced_markers(answer: &str, max: usize) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && (bytes[i + 1] == b'C' || bytes[i + 1] == b'c') {
            let mut j = i + 2;
            let mut num = 0usize;
            let mut saw_digit = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                num = num * 10 + usize::from(bytes[j] - b'0');
                saw_digit = true;
                j += 1;
            }
            if saw_digit && j < bytes.len() && bytes[j] == b']' && num >= 1 && num <= max {
                out.insert(num);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn citation_from_hit(idx: usize, h: &SearchHit) -> Citation {
    Citation {
        marker: format!("C{idx}"),
        chunk_id: h.chunk_id,
        document_id: h.document_id,
        seq: h.seq,
        title: h.title.clone(),
        excerpt: h.text.chars().take(320).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::referenced_markers;

    #[test]
    fn extracts_valid_markers_only() {
        let ans = "The host is malicious [C1] and resolves to an IP [C3]. Ignore [C9] and [Cx].";
        let got = referenced_markers(ans, 3);
        assert!(got.contains(&1));
        assert!(got.contains(&3));
        assert!(!got.contains(&9)); // out of range -> dropped (no fabricated citation)
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn handles_no_markers() {
        assert!(referenced_markers("no citations here", 5).is_empty());
    }
}
