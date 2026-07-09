//! PRIMITIVE: normalize untrusted decoded text for storage and search.

/// Strip control characters that have no place in extracted content, keeping only
/// tab and newline. Carriage returns and every other C0/C1 control (including
/// NUL, which a Postgres `text` value cannot even hold) are dropped.
///
/// This is NOT display sanitization (React escaping handles that at render time).
/// It keeps the content store, the full-text `tsvector`, and the embeddings free
/// of control noise, and it keeps a stray NUL from failing the chunk insert.
#[must_use]
pub fn clean_text(input: &str) -> String {
    input
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::clean_text;

    #[test]
    fn drops_nul_and_controls_keeps_tab_and_newline() {
        // NUL would make the Postgres text insert fail; it must go.
        assert_eq!(clean_text("a\0b"), "ab");
        // Tab and newline are legitimate content whitespace.
        assert_eq!(clean_text("a\tb\nc"), "a\tb\nc");
        // A carriage return is dropped, so CRLF normalizes to LF.
        assert_eq!(clean_text("a\r\nb"), "a\nb");
        // Other C0 controls (here BEL) are dropped.
        assert_eq!(clean_text("x\u{07}y"), "xy");
        // Clean text is returned unchanged.
        assert_eq!(clean_text("plain text, 1.2.3.4"), "plain text, 1.2.3.4");
    }
}
