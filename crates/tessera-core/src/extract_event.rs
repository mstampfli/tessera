//! The normalized record format produced by every extractor, in-process or
//! subprocess. Schema id `tessera.extract.v1`.
//!
//! One format for both worlds means a future out-of-language extractor (spawned
//! as a subprocess emitting NDJSON) and a built-in Rust extractor feed the exact
//! same downstream pipeline. The chunker, entity stage, and storage layer only
//! ever see [`ExtractEvent`]s.

use serde::{Deserialize, Serialize};

/// The wire schema marker embedded in the subprocess NDJSON envelope.
pub const EXTRACT_SCHEMA: &str = "tessera.extract.v1";

/// A byte span `[start, end)` into the original source, used for citations.
pub type Span = (u64, u64);

/// One unit of extracted content.
///
/// Serialized internally-tagged on the field `event`. The entity type lives on
/// its own field (`entity_kind`) so it never collides with the variant tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ExtractEvent {
    /// Document-level metadata discovered during extraction.
    Meta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
        attrs: serde_json::Map<String, serde_json::Value>,
    },
    /// A block of prose or free text, optionally tagged with a section label.
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section: Option<String>,
    },
    /// A structured record (a CSV row, a JSON object) kept whole for exact
    /// filtering; serialized to canonical text separately for embedding.
    Record { data: serde_json::Value },
    /// A pre-identified entity emitted directly by the extractor (rare; most
    /// entities come from the dedicated extraction stage).
    Entity {
        entity_kind: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
        #[serde(default = "default_confidence")]
        confidence: f32,
    },
    /// A non-fatal problem the host records but does not trust as content.
    Warn { message: String },
}

fn default_confidence() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::{default_confidence, ExtractEvent};

    #[test]
    fn text_event_roundtrips_through_json() {
        let ev = ExtractEvent::Text {
            text: "hello".into(),
            section: Some("intro".into()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"text\""), "got {json}");
        let back: ExtractEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn entity_type_and_variant_tag_do_not_collide() {
        let ev = ExtractEvent::Entity {
            entity_kind: "ip".into(),
            value: "1.2.3.4".into(),
            span: Some((0, 7)),
            confidence: 1.0,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"entity\""));
        assert!(json.contains("\"entity_kind\":\"ip\""));
        assert_eq!(serde_json::from_str::<ExtractEvent>(&json).unwrap(), ev);
    }

    #[test]
    fn entity_confidence_defaults_to_one_when_omitted() {
        let back: ExtractEvent =
            serde_json::from_str(r#"{"event":"entity","entity_kind":"cve","value":"CVE-2026-1"}"#)
                .unwrap();
        match back {
            ExtractEvent::Entity { confidence, .. } => {
                assert!((confidence - default_confidence()).abs() < f32::EPSILON);
            }
            other => panic!("expected entity, got {other:?}"),
        }
    }
}
