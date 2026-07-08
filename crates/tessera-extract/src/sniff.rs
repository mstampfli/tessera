//! Content-type sniffing. The client-declared media type is a hint, never
//! trusted: we look at the bytes (magic numbers, then UTF-8 structure) and
//! decide the label the extractor dispatches on.

use crate::SniffedType;

/// Sniff a media type and dispatch label from the raw bytes plus an optional
/// client-declared media type (used only as a tie-breaking hint).
#[must_use]
pub fn sniff(bytes: &[u8], declared: Option<&str>) -> SniffedType {
    // 1) Binary formats by magic number. Text-ish types (infer also recognizes
    // HTML/XML) fall through to the structural heuristics below.
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type();
        let binary_label = if mime == "application/pdf" {
            Some("pdf")
        } else if mime.starts_with("image/") {
            Some("image")
        } else if mime.starts_with("text/") || mime == "application/xml" {
            None
        } else {
            Some("binary")
        };
        if let Some(label) = binary_label {
            return SniffedType {
                media_type: mime.to_string(),
                label: label.to_string(),
            };
        }
    }

    // 2) Everything else is treated as text. Sniff structure on a bounded prefix.
    let head_len = bytes.len().min(8192);
    let head = String::from_utf8_lossy(&bytes[..head_len]);
    let trimmed = head.trim_start();

    // A declared type nudges ambiguous cases.
    let declared_label = declared.and_then(declared_to_label);

    let label = if looks_like_html(trimmed) {
        "html"
    } else if let Some(json_label) = looks_like_json(trimmed, &head) {
        json_label
    } else if declared_label == Some("csv") || looks_like_csv(&head) {
        "csv"
    } else if declared_label == Some("markdown") || looks_like_markdown(&head) {
        "markdown"
    } else if looks_like_log(&head) {
        "log"
    } else {
        declared_label.unwrap_or("text")
    };

    let media_type = match label {
        "html" => "text/html",
        "json" => "application/json",
        "ndjson" => "application/x-ndjson",
        "csv" => "text/csv",
        "markdown" => "text/markdown",
        // "log" and anything else fall to plain text.
        _ => "text/plain",
    };
    SniffedType {
        media_type: media_type.to_string(),
        label: label.to_string(),
    }
}

fn declared_to_label(m: &str) -> Option<&'static str> {
    let m = m.split(';').next().unwrap_or(m).trim().to_ascii_lowercase();
    match m.as_str() {
        "application/json" => Some("json"),
        "application/x-ndjson" | "application/jsonl" | "application/x-jsonlines" => Some("ndjson"),
        "text/csv" => Some("csv"),
        "text/markdown" | "text/x-markdown" => Some("markdown"),
        "text/html" | "application/xhtml+xml" => Some("html"),
        "text/plain" => Some("text"),
        _ => None,
    }
}

fn looks_like_html(s: &str) -> bool {
    let lower = s[..s.len().min(256)].to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || (lower.contains("<html") && lower.contains("<body"))
}

/// Returns "json" for a single document, "ndjson" for multiple JSON lines.
fn looks_like_json(trimmed: &str, head: &str) -> Option<&'static str> {
    let first = trimmed.chars().next()?;
    if first != '{' && first != '[' {
        return None;
    }
    // NDJSON: several lines that each independently parse as a JSON value.
    let json_lines = head
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .filter(|l| serde_json::from_str::<serde_json::Value>(l.trim()).is_ok())
        .count();
    if json_lines >= 2 {
        return Some("ndjson");
    }
    // A leading `{`/`[` is enough to call it JSON: a full parse of the 8 KiB
    // prefix would fail for any larger valid document, so we do not require it.
    Some("json")
}

fn looks_like_csv(head: &str) -> bool {
    let lines: Vec<&str> = head
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();
    if lines.len() < 2 {
        return false;
    }
    let commas_first = lines[0].matches(',').count();
    // At least one delimiter, and a consistent column count across sampled rows.
    commas_first >= 1 && lines.iter().all(|l| l.matches(',').count() == commas_first)
}

fn looks_like_markdown(head: &str) -> bool {
    head.lines().take(40).any(|l| {
        let t = l.trim_start();
        t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("```")
            || t.starts_with("> ")
    })
}

fn looks_like_log(head: &str) -> bool {
    // Heuristic: a majority of the first lines start with a timestamp-ish token
    // (ISO date, bracketed date, or syslog month).
    let lines: Vec<&str> = head
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .collect();
    if lines.len() < 3 {
        return false;
    }
    let logish = lines
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            starts_with_iso_date(t) || t.starts_with('[') || starts_with_syslog_month(t)
        })
        .count();
    logish * 2 >= lines.len()
}

fn starts_with_iso_date(s: &str) -> bool {
    // YYYY-MM-DD
    let b = s.as_bytes();
    b.len() >= 10
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

fn starts_with_syslog_month(s: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS.iter().any(|m| s.starts_with(m))
}

#[cfg(test)]
mod tests {
    use super::sniff;

    #[test]
    fn sniffs_common_text_formats() {
        assert_eq!(sniff(b"# Title\n\nsome prose", None).label, "markdown");
        assert_eq!(sniff(b"{\"a\":1,\"b\":2}", None).label, "json");
        assert_eq!(
            sniff(b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}", None).label,
            "ndjson"
        );
        assert_eq!(
            sniff(b"name,age,city\nalice,30,nyc\nbob,25,la", None).label,
            "csv"
        );
        assert_eq!(
            sniff(b"<!doctype html><html><body>hi</body></html>", None).label,
            "html"
        );
        assert_eq!(
            sniff(b"2026-07-08 10:00:01 INFO up\n2026-07-08 10:00:02 WARN slow\n2026-07-08 10:00:03 INFO ok", None).label,
            "log"
        );
        assert_eq!(sniff(b"just some plain words here", None).label, "text");
    }

    #[test]
    fn declared_type_breaks_ties_but_bytes_win_for_structure() {
        // Declared csv, but the bytes are clearly HTML: bytes win.
        assert_eq!(
            sniff(b"<html><body>x</body></html>", Some("text/csv")).label,
            "html"
        );
        // Ambiguous plain text with a csv hint: honor the hint.
        assert_eq!(sniff(b"a,b\n1,2", Some("text/csv")).label, "csv");
    }
}
