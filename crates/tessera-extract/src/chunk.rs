//! Chunking. Turns extracted text into embeddable/searchable units. Prose and
//! markdown use semantic-boundary splitting with overlap; structured records and
//! log windows are chunked by their extractor and pass through as-is.

use text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter};

/// Target chunk size in characters (roughly 375 tokens at ~4 chars/token, under
/// the 512-token ceiling of small embedding models).
const CHUNK_CHARS: usize = 1500;
/// Overlap between adjacent prose chunks, for context continuity at boundaries.
const CHUNK_OVERLAP: usize = 200;

/// One prepared chunk ready to persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedChunk {
    pub text: String,
    pub token_count: usize,
}

impl PreparedChunk {
    /// Build a chunk, cleaning control characters out of the text (every chunk in
    /// the system is constructed here, so this is where cleanliness is enforced)
    /// and computing an approximate token count.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = crate::text::clean_text(&text.into());
        let token_count = approx_tokens(&text);
        Self { text, token_count }
    }
}

/// Approximate token count without a model tokenizer: max of word count and
/// chars/4. Good enough for storage/metrics; real budgeting uses the embedder.
#[must_use]
pub fn approx_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    let by_chars = text.chars().count() / 4;
    words.max(by_chars)
}

fn config() -> ChunkConfig<text_splitter::Characters> {
    // with_overlap only fails if overlap >= capacity, which is statically false.
    ChunkConfig::new(CHUNK_CHARS)
        .with_overlap(CHUNK_OVERLAP)
        .expect("overlap < capacity by construction")
}

/// Split prose into overlapping, semantic-boundary chunks.
#[must_use]
pub fn chunk_prose(text: &str) -> Vec<PreparedChunk> {
    let splitter = TextSplitter::new(config());
    splitter
        .chunks(text)
        .filter(|c| !c.trim().is_empty())
        .map(PreparedChunk::new)
        .collect()
}

/// Split markdown, respecting heading and block structure.
#[must_use]
pub fn chunk_markdown(text: &str) -> Vec<PreparedChunk> {
    let splitter = MarkdownSplitter::new(config());
    splitter
        .chunks(text)
        .filter(|c| !c.trim().is_empty())
        .map(PreparedChunk::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{approx_tokens, chunk_prose};

    #[test]
    fn short_text_is_one_chunk() {
        let chunks = chunk_prose("a short sentence");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "a short sentence");
        assert!(chunks[0].token_count >= 3);
    }

    #[test]
    fn long_text_splits_into_multiple_chunks() {
        let para = "Lorem ipsum dolor sit amet. ".repeat(200); // ~5600 chars
        let chunks = chunk_prose(&para);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        // Every chunk stays near the configured size ceiling.
        assert!(chunks.iter().all(|c| c.text.chars().count() <= 1600));
    }

    #[test]
    fn approx_tokens_is_reasonable() {
        assert_eq!(approx_tokens(""), 0);
        assert!(approx_tokens("one two three four five") >= 5);
    }
}
