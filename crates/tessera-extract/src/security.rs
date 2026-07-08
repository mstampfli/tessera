//! The security / OSINT entity pack: deterministic, high-precision extraction of
//! indicators and identifiers (IPs, domains, URLs, emails, hashes, CVEs, MACs,
//! ASNs). No model is involved, so these are confidence 1.0.
//!
//! Input is refanged first (`hxxp` -> `http`, `[.]` -> `.`, etc.) so defanged
//! indicators from threat reports are caught. Each match carries a canonical
//! value (one normalization per kind) that the storage layer dedups on, plus the
//! raw surface for display.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::LazyLock;

use regex::Regex;

/// Entity kind tags (stored as `entities.kind`).
pub mod kind {
    pub const IP: &str = "ip";
    pub const IPV6: &str = "ipv6";
    pub const DOMAIN: &str = "domain";
    pub const URL: &str = "url";
    pub const EMAIL: &str = "email";
    pub const MD5: &str = "hash_md5";
    pub const SHA1: &str = "hash_sha1";
    pub const SHA256: &str = "hash_sha256";
    pub const CVE: &str = "cve";
    pub const MAC: &str = "mac";
    pub const ASN: &str = "asn";
}

/// One extracted entity occurrence within a piece of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMatch {
    pub kind: &'static str,
    /// The canonical, deduped form (e.g. lowercased domain, uppercased CVE).
    pub value: String,
    /// The surface form as it appeared (post-refang).
    pub raw: String,
    /// Byte span into the refanged text (deterministic, so mentions are stable).
    pub start: usize,
    pub end: usize,
}

// Compiled once. Each pattern is deliberately anchored on word boundaries.
static RE_IPV4: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap()
});
static RE_IPV6: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[A-Fa-f0-9]{0,4}:){2,7}[A-Fa-f0-9]{0,4}").unwrap());
static RE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bhttps?://[^\s<>"'\]\)}]+"#).unwrap());
static RE_EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap());
static RE_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Za-z0-9](?:[A-Za-z0-9\-]{0,61}[A-Za-z0-9])?\.)+[A-Za-z]{2,}\b").unwrap()
});
static RE_SHA256: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Fa-f0-9]{64}\b").unwrap());
static RE_SHA1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Fa-f0-9]{40}\b").unwrap());
static RE_MD5: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Fa-f0-9]{32}\b").unwrap());
static RE_CVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bCVE-\d{4}-\d{4,7}\b").unwrap());
static RE_MAC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b").unwrap());
static RE_ASN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bAS\d{1,10}\b").unwrap());

/// Undo common defanging so indicators in threat reports are caught. This changes
/// offsets, which is fine: all spans we report are into the refanged text and are
/// deterministic, so mentions dedup stably.
#[must_use]
pub fn refang(text: &str) -> String {
    // Order matters: the longer, more specific replacements go first.
    let mut s = text.replace("hxxps", "https").replace("hxxp", "http");
    for (from, to) in [
        ("[.]", "."),
        ("(.)", "."),
        ("[dot]", "."),
        ("(dot)", "."),
        ("{dot}", "."),
        ("[:]", ":"),
        ("[://]", "://"),
        ("[@]", "@"),
        ("(at)", "@"),
        ("[at]", "@"),
    ] {
        if s.contains(from) {
            s = s.replace(from, to);
        }
    }
    s
}

/// Extract all security entities from a piece of text (refanging first).
#[must_use]
pub fn extract(text: &str) -> Vec<EntityMatch> {
    let refanged = refang(text);
    let mut out = Vec::new();

    // URLs and emails first (they are the most specific).
    push_matches(&refanged, &RE_URL, kind::URL, canon_url, &mut out);
    push_matches(&refanged, &RE_EMAIL, kind::EMAIL, canon_lower, &mut out);

    // Hashes: longest first. Word boundaries mean a 64-hex string only matches
    // sha256, not the shorter patterns.
    push_matches(&refanged, &RE_SHA256, kind::SHA256, canon_lower, &mut out);
    push_matches(&refanged, &RE_SHA1, kind::SHA1, canon_lower, &mut out);
    push_matches(&refanged, &RE_MD5, kind::MD5, canon_lower, &mut out);

    push_matches(&refanged, &RE_CVE, kind::CVE, canon_upper, &mut out);
    push_matches(&refanged, &RE_MAC, kind::MAC, canon_mac, &mut out);
    push_matches(&refanged, &RE_ASN, kind::ASN, canon_upper, &mut out);

    // IPv4 with validation (the regex is loose on leading zeros / ranges).
    push_validated(
        &refanged,
        &RE_IPV4,
        kind::IP,
        |m| m.parse::<Ipv4Addr>().ok().map(|ip| ip.to_string()),
        &mut out,
    );

    // IPv6: validate candidates so we do not treat "a:b" or times like "10:30" as IPs.
    push_validated(
        &refanged,
        &RE_IPV6,
        kind::IPV6,
        |m| {
            if m.matches(':').count() >= 2 {
                m.parse::<Ipv6Addr>().ok().map(|ip| ip.to_string())
            } else {
                None
            }
        },
        &mut out,
    );

    // Domains: validate as a registrable name via the public suffix list. This is
    // what keeps "file.txt" or a bare "foo.bar" out.
    push_validated(
        &refanged,
        &RE_DOMAIN,
        kind::DOMAIN,
        |m| registrable_domain(&m.to_ascii_lowercase()),
        &mut out,
    );

    out
}

fn push_matches(
    text: &str,
    re: &Regex,
    kind: &'static str,
    canon: fn(&str) -> String,
    out: &mut Vec<EntityMatch>,
) {
    for m in re.find_iter(text) {
        out.push(EntityMatch {
            kind,
            value: canon(m.as_str()),
            raw: m.as_str().to_string(),
            start: m.start(),
            end: m.end(),
        });
    }
}

fn push_validated(
    text: &str,
    re: &Regex,
    kind: &'static str,
    validate: impl Fn(&str) -> Option<String>,
    out: &mut Vec<EntityMatch>,
) {
    for m in re.find_iter(text) {
        if let Some(value) = validate(m.as_str()) {
            out.push(EntityMatch {
                kind,
                value,
                raw: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
            });
        }
    }
}

fn canon_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}
fn canon_upper(s: &str) -> String {
    s.to_ascii_uppercase()
}
fn canon_mac(s: &str) -> String {
    s.to_ascii_lowercase().replace('-', ":")
}
fn canon_url(s: &str) -> String {
    // Lowercase the scheme and host; leave the path as-is. Trim common trailing
    // punctuation captured from prose.
    let trimmed = s.trim_end_matches(['.', ',', ')', ']', '"', '\'']);
    match url::Url::parse(trimmed) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("").to_ascii_lowercase();
            let scheme = u.scheme().to_ascii_lowercase();
            let path = u.path();
            let query = u.query().map(|q| format!("?{q}")).unwrap_or_default();
            if host.is_empty() {
                trimmed.to_ascii_lowercase()
            } else {
                format!("{scheme}://{host}{path}{query}")
            }
        }
        Err(_) => trimmed.to_ascii_lowercase(),
    }
}

/// The registrable domain (eTLD+1) per the public suffix list, or `None` if the
/// name is not a valid registrable domain. The suffix must be a KNOWN (ICANN or
/// private) suffix; psl's default wildcard rule would otherwise accept any
/// `label.unknownword` (e.g. `config.yaml`) as a domain.
fn registrable_domain(name: &str) -> Option<String> {
    let trimmed = name.trim_end_matches('.');
    let suffix = psl::suffix(trimmed.as_bytes())?;
    // typ() is None only for the implicit `*` fallback (an unknown TLD): reject.
    suffix.typ()?;
    let dom = psl::domain_str(trimmed)?;
    // Require at least one dot (a bare public suffix is not an entity).
    if dom.contains('.') {
        Some(dom.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{extract, kind, refang};

    fn values(text: &str, k: &str) -> Vec<String> {
        let mut v: Vec<String> = extract(text)
            .into_iter()
            .filter(|m| m.kind == k)
            .map(|m| m.value)
            .collect();
        v.sort();
        v.dedup();
        v
    }

    #[test]
    fn refang_undoes_defanging() {
        assert_eq!(refang("hxxp://evil[.]com"), "http://evil.com");
        assert_eq!(refang("1.2.3.4"), "1.2.3.4");
        assert_eq!(refang("mail[at]evil[dot]com"), "mail@evil.com");
    }

    #[test]
    fn extracts_defanged_indicators_from_a_report() {
        let report = "Beacon to 185[.]220[.]101[.]44 via hxxps://evil-panel[.]com/gate. \
                      Loader SHA256 9f2a1c3e4b5d6a7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7. \
                      Exploited CVE-2026-31337. Contact ops[at]evil-panel[.]com. AS64500.";

        assert_eq!(values(report, kind::IP), vec!["185.220.101.44"]);
        assert_eq!(values(report, kind::DOMAIN), vec!["evil-panel.com"]);
        assert_eq!(
            values(report, kind::URL),
            vec!["https://evil-panel.com/gate"]
        );
        assert_eq!(values(report, kind::EMAIL), vec!["ops@evil-panel.com"]);
        assert_eq!(
            values(report, kind::SHA256),
            vec!["9f2a1c3e4b5d6a7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7"]
        );
        assert_eq!(values(report, kind::CVE), vec!["CVE-2026-31337"]);
        assert_eq!(values(report, kind::ASN), vec!["AS64500"]);
    }

    #[test]
    fn hashes_do_not_cross_contaminate() {
        // A standalone md5 is md5, not a truncated sha.
        let t =
            "md5 d41d8cd98f00b204e9800998ecf8427e sha1 da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert_eq!(
            values(t, kind::MD5),
            vec!["d41d8cd98f00b204e9800998ecf8427e"]
        );
        assert_eq!(
            values(t, kind::SHA1),
            vec!["da39a3ee5e6b4b0d3255bfef95601890afd80709"]
        );
        assert!(values(t, kind::SHA256).is_empty());
    }

    #[test]
    fn rejects_non_registrable_and_times() {
        // A bare word with a dot but invalid TLD is not a domain.
        assert!(values("see file.xyznotarealtld", kind::DOMAIN).is_empty());
        // A time like 10:30 is not an IPv6 address.
        assert!(values("the meeting at 10:30 today", kind::IPV6).is_empty());
    }

    #[test]
    fn extraction_is_deterministic() {
        let t = "hit 8.8.8.8 and 8.8.8.8 again, plus evil.com";
        let a = extract(t);
        let b = extract(t);
        assert_eq!(a, b);
    }
}
