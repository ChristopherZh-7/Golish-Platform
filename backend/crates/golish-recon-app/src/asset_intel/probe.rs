//! Liveness mapping for active-probe output.
//!
//! `target_intel` stays **zero-touch** (design 2026-06-17 Task 13 decision):
//! `real_ip` here comes only from survey pairing + passive DNS, never from
//! sending packets at the target. This pure mapping pins the contract for
//! turning an `httpx -json` line into our `targets.status`; its IO caller is the
//! EAS active-scan specialist (httpx is an EAS-assigned tool), not the passive
//! enrich path — hence it is deliberately not wired here yet.

/// Map one `httpx -json` line into a `targets.status` value.
///
/// Any reachable HTTP response (status in `[100, 600)`) is `live`; an explicit
/// `failed: true` is `dead` (the asset was checked and did not answer — kept,
/// never dropped, per invariant D2); everything else is `unknown`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn liveness_from_httpx(line: &serde_json::Value) -> &'static str {
    match line.get("status_code").and_then(|value| value.as_i64()) {
        Some(code) if (100..600).contains(&code) => "live",
        _ if line
            .get("failed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false) =>
        {
            "dead"
        }
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_from_httpx_maps_status_failed_and_unknown() {
        assert_eq!(
            liveness_from_httpx(&serde_json::json!({"status_code": 200})),
            "live"
        );
        // Any reachable response code counts as live (the host answered).
        assert_eq!(
            liveness_from_httpx(&serde_json::json!({"status_code": 500})),
            "live"
        );
        // Explicitly failed → dead, but still a checked result (D2: never drop).
        assert_eq!(
            liveness_from_httpx(&serde_json::json!({"failed": true})),
            "dead"
        );
        // No signal at all → unknown (distinct from checked-empty).
        assert_eq!(liveness_from_httpx(&serde_json::json!({})), "unknown");
    }
}
