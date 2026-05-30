//! Unix timestamp helpers shared across the workspace.
//!
//! Single source of truth for the `now_ts` / `now_ms` / `ts_from_dt` helpers
//! that were previously copy-pasted across `golish`, `golish-pipeline` and
//! `golish-vuln-intel` (architecture audit B-D3 / roadmap P1-3).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix time in **seconds**.
pub fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Current Unix time in **milliseconds**.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Convert a UTC datetime to whole Unix **seconds** (sub-second part truncated).
pub fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

/// Format a UTC timestamp as a coarse relative time (e.g. `"2h ago"`,
/// `"just now"`). Single source of truth for what used to be copy-pasted
/// `format_relative_time` helpers in `golish-indexer` and `golish`.
pub fn format_relative_time(datetime: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(datetime);

    if duration.num_days() > 0 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn now_ts_is_seconds_and_recent() {
        // Sanity: after 2021-01-01 (1_609_459_200s) and below year ~2100.
        let s = now_ts();
        assert!(
            s > 1_609_459_200,
            "now_ts should be a post-2021 second count"
        );
        assert!(s < 4_102_444_800, "now_ts unexpectedly far in the future");
    }

    #[test]
    fn now_ms_is_milliseconds_consistent_with_now_ts() {
        let s = now_ts();
        let ms = now_ms();
        // ms/1000 must be within a couple seconds of s (clock advances between calls).
        let ms_secs = ms / 1000;
        assert!(
            ms_secs + 2 >= s && s + 2 >= ms_secs,
            "now_ms ({ms}) inconsistent with now_ts ({s})"
        );
    }

    #[test]
    fn ts_from_dt_truncates_to_whole_seconds() {
        let dt = Utc.timestamp_opt(1_609_459_200, 999_000_000).unwrap();
        assert_eq!(ts_from_dt(dt), 1_609_459_200);
    }

    #[test]
    fn ts_from_dt_matches_known_epoch() {
        let dt = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert_eq!(ts_from_dt(dt), 1_700_000_000);
    }
}
