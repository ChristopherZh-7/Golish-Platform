//! Unit tests for the pure (non-DB) parts of the audit repo: the exact-project
//! SQL builders (`queries`) and the reclaim cutoff math (`mod`). DB-aware paths
//! need a pg-embed integration harness (deferred). Merged verbatim from the
//! original `audit.rs` `sql_tests` + `reclaim_tests` inline modules.

use chrono::{Duration, Utc};

use super::queries::{build_clear_by_project_exact_sql, build_list_by_project_exact_sql};
use super::{reclaim_cutoff, DEFAULT_RECLAIM_THRESHOLD_HOURS};

#[test]
fn audit_command_sql_matches_command_layer() {
    assert_eq!(
        build_list_by_project_exact_sql(),
        "SELECT created_at, action, category, details, entity_type, entity_id, source FROM audit_log WHERE ($1::text IS NULL OR category = $1) AND project_path = $2 ORDER BY created_at DESC LIMIT $3"
    );
    assert_eq!(
        build_clear_by_project_exact_sql(),
        "DELETE FROM audit_log WHERE project_path = $1"
    );
}

#[test]
fn default_reclaim_threshold_is_one_hour() {
    assert_eq!(DEFAULT_RECLAIM_THRESHOLD_HOURS, 1);
}

#[test]
fn reclaim_cutoff_one_hour_back_in_range() {
    let now_before = Utc::now();
    let cutoff = reclaim_cutoff(Duration::hours(1));
    let now_after = Utc::now();

    // cutoff 必须落在 (now_before - 1h, now_after - 1h] 区间内
    let lower = now_before - Duration::hours(1) - Duration::seconds(1);
    let upper = now_after - Duration::hours(1) + Duration::seconds(1);
    assert!(
        cutoff > lower,
        "cutoff {} < {} (lower bound)",
        cutoff,
        lower
    );
    assert!(
        cutoff < upper,
        "cutoff {} > {} (upper bound)",
        cutoff,
        upper
    );
}

#[test]
fn reclaim_cutoff_zero_duration_is_now() {
    let before = Utc::now();
    let cutoff = reclaim_cutoff(Duration::zero());
    let after = Utc::now();
    assert!(cutoff >= before - Duration::milliseconds(1));
    assert!(cutoff <= after + Duration::milliseconds(1));
}

#[test]
fn reclaim_cutoff_large_duration_far_in_past() {
    let cutoff = reclaim_cutoff(Duration::days(365));
    let one_year_ago_roughly = Utc::now() - Duration::days(364);
    assert!(
        cutoff < one_year_ago_roughly,
        "cutoff {} should be more than 364 days ago",
        cutoff
    );
}
