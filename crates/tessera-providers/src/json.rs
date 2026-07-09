//! PRIMITIVE: get a typed JSON value out of an LLM. See docs/PRIMITIVES.md.

use serde::de::DeserializeOwned;

use crate::{GenRequest, LlmProvider, ProviderError};

/// Appended to the system prompt on the retry attempt.
const JSON_NUDGE: &str = "Your previous reply could not be parsed. Reply with a \
single JSON object only, with no prose, explanation, or code fences.";

/// Call an LLM and parse its reply into `T`.
///
/// This is THE way to get a typed JSON value from a model. It owns the fragile
/// mechanics that every caller would otherwise re-derive: models routinely wrap
/// the object in prose or Markdown code fences, so it extracts the outermost
/// `{...}` before deserializing, and it retries ONCE, with a corrective
/// instruction appended to the system prompt, if the first reply does not parse.
/// It returns the parsed value and the concrete model id (for provenance).
///
/// Only a *parse* failure is retried. A provider/transport error propagates
/// immediately, because failover across backends is the chain's job
/// (`ChainedLlm`), not this function's.
///
/// This is NOT a schema validator. It guarantees a well-formed `T`, not that the
/// field values are sane: the caller still owns its prompt (describe the fields
/// there) and any semantic validation of the parsed value (enum membership,
/// numeric ranges, cross-field checks).
pub async fn generate_json<T: DeserializeOwned>(
    llm: &dyn LlmProvider,
    req: &GenRequest,
) -> Result<(T, String), ProviderError> {
    let mut last_err = String::new();
    for attempt in 0..2 {
        let resp = if attempt == 0 {
            llm.generate(req).await?
        } else {
            tracing::debug!("generate_json: reply did not parse, retrying once");
            llm.generate(&with_nudge(req)).await?
        };
        match extract_json::<T>(&resp.text) {
            Ok(value) => return Ok((value, resp.model)),
            Err(e) => last_err = e,
        }
    }
    Err(ProviderError::InvalidOutput(last_err))
}

/// Copy `req` with the corrective JSON instruction appended to its system prompt.
fn with_nudge(req: &GenRequest) -> GenRequest {
    let system = match &req.system {
        Some(s) => format!("{s}\n{JSON_NUDGE}"),
        None => JSON_NUDGE.to_string(),
    };
    GenRequest {
        prompt: req.prompt.clone(),
        system: Some(system),
        max_tokens: req.max_tokens,
    }
}

/// Slice the outermost `{...}` out of the model text and deserialize it. Models
/// often wrap the object in prose or markdown fences, so we cut from the first
/// `{` to the last `}` before parsing. Both are ASCII, so the slice is always on
/// a char boundary.
fn extract_json<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    let start = text.find('{').ok_or("no JSON object in reply")?;
    let end = text.rfind('}').ok_or("no closing brace in reply")?;
    if end <= start {
        return Err("malformed JSON bounds".to_string());
    }
    serde_json::from_str::<T>(&text[start..=end]).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GenResponse, ProviderHealth};
    use async_trait::async_trait;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Doc {
        a: i32,
        b: String,
    }

    fn req() -> GenRequest {
        GenRequest {
            prompt: "p".to_string(),
            system: None,
            max_tokens: None,
        }
    }

    /// Always returns the same canned text.
    struct Canned(&'static str);
    #[async_trait]
    impl LlmProvider for Canned {
        fn id(&self) -> &'static str {
            "canned"
        }
        async fn generate(&self, _req: &GenRequest) -> Result<GenResponse, ProviderError> {
            Ok(GenResponse {
                text: self.0.to_string(),
                model: "test-model".to_string(),
            })
        }
        async fn health(&self) -> ProviderHealth {
            ProviderHealth::Up
        }
    }

    #[tokio::test]
    async fn parses_json_wrapped_in_prose_and_fences() {
        let llm = Canned("Sure!\n```json\n{\"a\":1,\"b\":\"x\"}\n```\nDone.");
        let (doc, model): (Doc, String) = generate_json(&llm, &req()).await.unwrap();
        assert_eq!(
            doc,
            Doc {
                a: 1,
                b: "x".to_string()
            }
        );
        assert_eq!(model, "test-model");
    }

    /// Returns unparseable text on the first call, valid JSON on the second, and
    /// asserts the retry carried the corrective nudge.
    struct FlakyLlm {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl LlmProvider for FlakyLlm {
        fn id(&self) -> &'static str {
            "flaky"
        }
        async fn generate(&self, req: &GenRequest) -> Result<GenResponse, ProviderError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let text = if n == 0 {
                "I cannot help with that.".to_string()
            } else {
                assert!(
                    req.system
                        .as_deref()
                        .is_some_and(|s| s.contains("single JSON object only")),
                    "retry must append the JSON nudge to the system prompt"
                );
                "{\"a\":2,\"b\":\"y\"}".to_string()
            };
            Ok(GenResponse {
                text,
                model: "m".to_string(),
            })
        }
        async fn health(&self) -> ProviderHealth {
            ProviderHealth::Up
        }
    }

    #[tokio::test]
    async fn retries_once_on_unparseable_reply() {
        let llm = FlakyLlm {
            calls: AtomicUsize::new(0),
        };
        let (doc, _): (Doc, String) = generate_json(&llm, &req()).await.unwrap();
        assert_eq!(
            doc,
            Doc {
                a: 2,
                b: "y".to_string()
            }
        );
        assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_two_bad_replies() {
        let llm = Canned("no json here");
        let r: Result<(Doc, String), _> = generate_json(&llm, &req()).await;
        assert!(matches!(r, Err(ProviderError::InvalidOutput(_))));
    }

    #[test]
    fn extract_rejects_reversed_braces() {
        assert!(extract_json::<Doc>("} then {").is_err());
    }

    #[test]
    fn extract_rejects_missing_object() {
        assert!(extract_json::<Doc>("no braces at all").is_err());
    }
}
