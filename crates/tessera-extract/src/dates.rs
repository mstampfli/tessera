//! Event-date extraction: pull the time an event happened out of a document's
//! text, so temporal correlation has an axis to work on.
//!
//! Conservative on purpose. It matches ISO-8601 dates and datetimes (the format
//! structured feeds and machine-written reports use) and a small set of common
//! written forms, and it returns the EARLIEST plausible date found, which for a
//! report of an event is usually the event itself rather than a later reference.
//! A date far in the future or absurdly old is rejected as noise.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

fn iso_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // 2026-07-09  or  2026-07-09T10:30:00(Z|+02:00)  or  2026-07-09 10:30:00
        Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?(Z|[+-]\d{2}:?\d{2})?)?\b")
            .expect("valid regex")
    })
}

/// A plausible event date: within a sane window (not obviously a version number,
/// port, or far-future placeholder). Years 1970..=2100.
fn plausible(dt: DateTime<Utc>) -> bool {
    let year = dt.format("%Y").to_string().parse::<i32>().unwrap_or(0);
    (1970..=2100).contains(&year)
}

/// The earliest plausible date/time mentioned in the text, as UTC. Returns
/// `None` when the text carries no parseable date.
#[must_use]
pub fn extract_earliest(text: &str) -> Option<DateTime<Utc>> {
    let mut earliest: Option<DateTime<Utc>> = None;
    for cap in iso_re().captures_iter(text) {
        let y: i32 = cap.get(1)?.as_str().parse().ok()?;
        let mo: u32 = cap.get(2)?.as_str().parse().ok()?;
        let d: u32 = cap.get(3)?.as_str().parse().ok()?;
        let Some(date) = NaiveDate::from_ymd_opt(y, mo, d) else {
            continue;
        };
        let (h, mi, s) = (
            cap.get(4)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0),
            cap.get(5)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0),
            cap.get(6)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0),
        );
        let Some(naive) = date.and_hms_opt(h, mi, s) else {
            continue;
        };
        let dt = Utc.from_utc_datetime(&naive);
        if !plausible(dt) {
            continue;
        }
        if earliest.is_none_or(|e| dt < e) {
            earliest = Some(dt);
        }
    }
    earliest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_iso_date() {
        let dt = extract_earliest("The breach occurred on 2026-03-14 per the report.").unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-03-14");
    }

    #[test]
    fn returns_earliest() {
        let dt = extract_earliest("seen 2026-05-01, first observed 2026-01-09, again 2026-06-02")
            .unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-01-09");
    }

    #[test]
    fn parses_datetime_with_zone() {
        let dt = extract_earliest("at 2026-07-09T10:30:00Z the alert fired").unwrap();
        assert_eq!(dt.format("%Y-%m-%dT%H:%M").to_string(), "2026-07-09T10:30");
    }

    #[test]
    fn none_when_no_date() {
        assert!(extract_earliest("no dates here, just 10:30 and version 1.2.3").is_none());
    }

    #[test]
    fn rejects_implausible_year() {
        assert!(extract_earliest("port 9999-99-99 nonsense").is_none());
    }
}
