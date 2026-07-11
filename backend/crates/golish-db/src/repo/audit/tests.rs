//! Unit tests for the pure (non-DB) parts of the audit repo: the exact-project
//! SQL builders (`queries`) and the reclaim cutoff math (`mod`). DB-aware paths
//! need a pg-embed integration harness (deferred). Merged verbatim from the
//! original `audit.rs` `sql_tests` + `reclaim_tests` inline modules.

use chrono::{Duration, Utc};

use super::queries::{
    build_clear_by_project_exact_sql, build_list_by_project_exact_sql,
    build_list_by_target_current_owner_sql,
};
use super::{
    audit_project_path, reclaim_cutoff, DEFAULT_RECLAIM_THRESHOLD_HOURS,
    EVIDENCE_FACTS_FOR_SESSION_ORG_FRESH_SQL,
};

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
fn target_timeline_requires_current_in_scope_project_owner() {
    let sql = build_list_by_target_current_owner_sql();
    assert!(sql.contains("JOIN targets t ON t.id = al.target_id"));
    assert!(sql.contains("t.scope::text = 'in'"));
    assert!(sql.contains("t.project_path IS NOT NULL"));
    assert!(sql.contains("al.project_path = t.project_path"));
}

#[test]
fn default_reclaim_threshold_is_one_hour() {
    assert_eq!(DEFAULT_RECLAIM_THRESHOLD_HOURS, 1);
}

#[test]
fn audit_project_path_defaults_to_empty_string() {
    assert_eq!(audit_project_path(None), "");
    assert_eq!(audit_project_path(Some("/tmp/project")), "/tmp/project");
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
fn target_bound_evidence_query_is_producer_org_target_owner_and_freshness_scoped() {
    let sql = EVIDENCE_FACTS_FOR_SESSION_ORG_FRESH_SQL;
    assert!(sql.contains("JOIN targets t ON t.id = al.target_id"));
    assert!(sql.contains("al.detail->>'organization_id' AS evidence_organization_id"));
    assert!(sql.contains("al.tool_name"));
    assert!(sql.contains("al.detail->>'kind' AS evidence_kind"));
    assert!(sql.contains("t.organization_id AS target_organization_id"));
    assert!(sql.contains("t.name AS target_name"));
    assert!(sql.contains("t.value AS target_value"));
    assert!(sql.contains("COALESCE(t.ports, '[]'::jsonb) AS target_ports"));
    assert!(sql.contains("al.detail->>'organization_id' = $2::text"));
    assert!(sql.contains("t.organization_id = $2"));
    assert!(sql.contains("t.scope::text = 'in'"));
    assert!(sql.contains("t.project_path IS NOT NULL"));
    assert!(sql.contains("al.project_path = t.project_path"));
    assert!(sql.contains("al.created_at >= $3"));
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
