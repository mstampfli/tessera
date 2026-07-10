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
use tessera_providers::{generate_json, GenRequest, LlmProvider};

const SYSTEM: &str = "You state what a group of related sources actually SAYS, factually and \
objectively. An algorithm has already grouped these excerpts. State the SUBSTANCE they contain: \
the specific claims, arguments, names, dates, numbers, and findings - concretely and in detail, \
so a reader learns the content ITSELF. Do NOT merely say the sources 'discuss', 'mention', \
'cover', 'highlight', 'relate to', or 'provide information about' a topic without saying WHAT \
they claim or show; that is useless. When the sources describe a theory, position, event, or \
finding, STATE what it asserts, who advances it, and the specific evidence, figures, or rebuttal \
given. Prefer naming the concrete detail over summarizing that detail exists. \
Records co-occurring, or sharing infrastructure, a hosting provider, a CDN, or a certificate \
authority, is NOT evidence of an attack, intent, or compromise; do not infer one. These are \
records the operator collected about their own or authorized targets, not detections of an \
adversary. The context begins with a PROVENANCE line saying where the records came from; follow \
its guidance on how to frame the insight and what kind of suggested_actions to give. It also \
includes a SECURITY SIGNAL line: discuss a security concern, and raise severity above 'info', \
ONLY when that line lists concrete indicators; if it says none, stay neutral and set severity to \
'info' with no alarmist actions. \
Respond with STRICT JSON only, no prose outside the JSON, with exactly these fields: \
title (string, short, naming the specific subject not a generic label), narrative (string, \
several sentences that state the concrete substance and specifics - not a meta-summary that the \
topic exists; cite supporting excerpts inline as [E1], [E2], etc; every claim must have a \
citation and you may only use the E-numbers provided), \
severity (one of: info, low, medium, high, critical), confidence (number 0 to 1), \
suggested_actions (array of short imperative strings; may be empty). \
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

/// Synthesize an insight over a cluster's representative chunks. The JSON parse
/// (and its one retry) is handled by the `generate_json` primitive; this function
/// owns the domain validation of the result.
pub async fn synthesize(
    llm: &std::sync::Arc<dyn LlmProvider>,
    chunks: &[MemberChunk],
    entities: &[(String, String)],
    collected: bool,
) -> Result<Synthesized> {
    let req = GenRequest {
        prompt: build_prompt(chunks, entities, collected),
        system: Some(SYSTEM.to_string()),
        max_tokens: Some(1100),
    };
    let (raw, model): (RawInsight, String) = generate_json(llm.as_ref(), &req)
        .await
        .map_err(|e| Error::new(ErrorKind::Provider, format!("insight synthesis: {e}")))?;

    let severity = normalize_severity(&raw.severity, has_harm_signal(entities));
    let cited: Vec<usize> = referenced_markers(&raw.narrative, chunks.len())
        .into_iter()
        .collect();
    Ok(Synthesized {
        title: raw.title.trim().to_string(),
        narrative: raw.narrative.trim().to_string(),
        severity,
        confidence: raw.confidence.clamp(0.0, 1.0),
        suggested_actions: raw.suggested_actions,
        cited,
        model,
    })
}

fn build_prompt(chunks: &[MemberChunk], entities: &[(String, String)], collected: bool) -> String {
    use std::fmt::Write as _;
    let mut ctx = String::new();
    for (i, c) in chunks.iter().enumerate() {
        let marker = i + 1;
        let title = c.title.as_deref().unwrap_or("untitled");
        let excerpt: String = c.text.chars().take(1500).collect();
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
        "Provenance: {}\nSecurity signal: {}\n\nNotable entities in this group: {ent_list}\n\n\
         Excerpts:\n{ctx}\nWrite the JSON insight now.",
        provenance_line(collected),
        security_signal(entities),
    )
}

/// The provenance line surfaced to the model. Self-collected material is our own
/// OSINT/reference, not external activity - so the insight should summarize what
/// the sources SAY and suggest what to investigate next, never defend or moderate.
fn provenance_line(collected: bool) -> &'static str {
    if collected {
        "this material is OSINT you collected yourself (your own queries and fetched \
         sources), not external activity or an incoming report. Summarize what the sources \
         actually say about the subject; suggested_actions must be investigative next steps \
         (what to look into, corroborate, or pull next), never defensive or content-moderation \
         actions."
    } else {
        "unspecified."
    }
}

/// The deterministic security signal for a group, computed from its own entities.
/// Today the only structural harm indicator tessera extracts is a CVE reference;
/// the line stays honest about that. Other indicators (a malicious-IP
/// classification, exposed credentials, an IOC that matches a threat feed) are
/// added here as tessera learns to extract them as structured entities.
fn security_signal(entities: &[(String, String)]) -> String {
    let cves: Vec<&str> = entities
        .iter()
        .filter(|(kind, _)| kind.eq_ignore_ascii_case(tessera_extract::security::kind::CVE))
        .map(|(_, v)| v.as_str())
        .collect();
    if cves.is_empty() {
        "none detected (treat these as neutral observation records)".to_string()
    } else {
        format!("vulnerability references present: {}", cves.join(", "))
    }
}

/// Whether the group carries a structural harm indicator that justifies a security
/// framing (see `security_signal`). Absent one, the group is neutral observation
/// data and its insight stays objective.
fn has_harm_signal(entities: &[(String, String)]) -> bool {
    entities
        .iter()
        .any(|(kind, _)| kind.eq_ignore_ascii_case(tessera_extract::security::kind::CVE))
}

/// Severity is bounded by the evidence, not asserted by the model (the correlation
/// invariant applied to severity): with no harm signal the group is a neutral
/// observation and severity is forced to `info`; with one, the model's severity is
/// honored, and an invalid value degrades to `low` rather than inflating to medium.
fn normalize_severity(raw: &str, has_harm: bool) -> String {
    if !has_harm {
        return "info".to_string();
    }
    if ALLOWED_SEVERITY.contains(&raw) {
        raw.to_string()
    } else {
        "low".to_string()
    }
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
    use super::{has_harm_signal, normalize_severity, referenced_markers, security_signal};

    fn ents(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn markers_are_bounded_and_deduped() {
        let got = referenced_markers("a [E1] b [E3] c [E1] d [E9]", 3);
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn neutral_infrastructure_has_no_harm_signal() {
        let e = ents(&[("ip", "188.114.96.12"), ("domain", "mstampfli.com")]);
        assert!(!has_harm_signal(&e));
        assert!(security_signal(&e).starts_with("none detected"));
    }

    #[test]
    fn a_cve_reference_is_a_harm_signal() {
        let e = ents(&[("domain", "x.com"), ("cve", "CVE-2026-1")]);
        assert!(has_harm_signal(&e));
        assert!(security_signal(&e).contains("CVE-2026-1"));
    }

    #[test]
    fn no_harm_forces_info_regardless_of_the_model() {
        // The model crying "critical" over neutral DNS/cert records is capped to info.
        assert_eq!(normalize_severity("critical", false), "info");
        assert_eq!(normalize_severity("high", false), "info");
    }

    #[test]
    fn harm_signal_honors_model_severity_but_degrades_invalid_to_low() {
        assert_eq!(normalize_severity("high", true), "high");
        assert_eq!(normalize_severity("nonsense", true), "low");
    }

    #[test]
    fn self_collected_provenance_asks_to_summarize_not_police() {
        let collected = super::provenance_line(true);
        assert!(collected.contains("collected yourself"));
        assert!(collected.contains("investigative"));
        assert!(collected.contains("never defensive or content-moderation"));
        assert_eq!(super::provenance_line(false), "unspecified.");
        // The provenance guidance actually reaches the built prompt.
        let prompt = super::build_prompt(&[], &[], true);
        assert!(prompt.contains("Provenance:"));
        assert!(prompt.contains("investigative next steps"));
    }
}
