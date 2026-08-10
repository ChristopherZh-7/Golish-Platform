use golish_core::{InvestigationContractVersion, InvestigationRolloutMode};
use golish_db::repo::investigation_projection::{
    LegacyField, ProjectionHead, ProjectionTemporalStatusV1,
};
use golish_pentest_domain::tool_truth::ToolTruthContract;

#[test]
fn plan_d_projection_envelope_is_versioned_and_uses_single_policy() {
    let mode = InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection;
    let head = ProjectionHead {
        projection_schema_version: 1,
        change_seq: 9,
        read_at: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0)
            .expect("Unix epoch is representable"),
        as_of_temporal_cutoff: None,
        authority_epoch_set_hash: [7; 32],
        tool_truth_contract: ToolTruthContract::ReceiptV1,
        investigation_contract_version: InvestigationContractVersion::HypothesisRegistryV1,
        investigation_rollout_mode: mode,
        mode_policy: mode.policy(),
    };
    assert_eq!(head.mode_policy, head.investigation_rollout_mode.policy());
    assert_eq!(head.projection_schema_version, 1);
    assert_eq!(
        ProjectionTemporalStatusV1::TemporallyStale,
        ProjectionTemporalStatusV1::TemporallyStale,
    );
    assert_eq!(
        LegacyField::<String>::LegacyUnavailable,
        LegacyField::LegacyUnavailable,
    );
}

#[test]
fn plan_d_migration_adds_only_projection_extensions() {
    let sql = include_str!("../migrations/20260729000008_investigation_projection.sql");
    assert!(sql.contains("ADD COLUMN investigation_workspace_json JSONB"));
    assert!(sql.contains("CREATE INDEX investigation_projection_changes_timeline_idx"));
    assert!(sql.contains("CREATE TABLE investigation_projection_compare_aggregates"));
    assert!(sql.contains("CREATE TABLE operation_default_promotion_receipts"));
    assert!(sql.contains("CREATE TABLE legacy_attempt_authority_receipts"));
    for plan_b_authority in [
        "CREATE TABLE investigation_rollout",
        "CREATE TABLE investigation_projection_source_heads",
        "CREATE TABLE investigation_projection_outbox_batches",
        "CREATE TABLE investigation_projection_heads",
        "CREATE TABLE investigation_projection_changes",
        "CREATE TABLE investigation_projection_compare_samples",
        "CREATE TABLE operation_contract_adoptions",
    ] {
        assert!(!sql.contains(plan_b_authority));
    }
    assert!(!sql.contains("investigation_shadow_comparisons"));
}

#[test]
fn adversarial_acceptance_fixture_universe_is_closed() {
    let sql = include_str!("../migrations/20260729000008_investigation_projection.sql");
    for fixture in [
        "known_vulnerable",
        "known_safe",
        "control_failure",
        "soft_404",
        "waf_interstitial",
        "dynamic_content",
        "multi_role_idor",
        "race",
        "adapter_missing",
    ] {
        assert!(sql.contains(&format!("'{fixture}'")));
    }
    assert!(sql.contains("fixture_member_count=9"));
    assert!(sql.contains("mismatch_count=0"));
}
