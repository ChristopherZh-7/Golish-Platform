const MIGRATION: &str =
    include_str!("../migrations/20260812000005_hypothesis_pending_evolution_authority.sql");

#[test]
fn pending_evolution_authority_is_exact_and_append_only() {
    for required in [
        "fact_delta_member_count BIGINT NOT NULL CHECK (fact_delta_member_count>0)",
        "consolidation_batch_id UUID NOT NULL UNIQUE",
        "source_generation_id UUID NOT NULL",
        "source_wave_denominator_id UUID NOT NULL",
        "wave_coverage_receipt_id UUID NOT NULL",
        "applied_fact_delta_set_hash TEXT NOT NULL",
        "residual_set_hash TEXT NOT NULL",
        "source_snapshot_hash TEXT NOT NULL",
        "HYPOTHESIS_PENDING_EVOLUTION_BATCH_AUTHORITY_INVALID",
        "HYPOTHESIS_PENDING_EVOLUTION_ALREADY_CONSOLIDATED",
        "HYPOTHESIS_PENDING_EVOLUTION_WAVE_AUTHORITY_INVALID",
        "HYPOTHESIS_PENDING_EVOLUTION_CONSUMPTION_AUTHORITY_INVALID",
        "hypothesis_pending_evolution_authorities_append_only",
        "verification_reject_append_only()",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing migration fence: {required}"
        );
    }
}

#[test]
fn pending_evolution_authority_recomputes_the_exact_consumption_set() {
    assert!(MIGRATION.contains("investigation_exact_member_set_hash("));
    assert!(MIGRATION.contains("'hypothesis_consolidation_consumptions.v1'"));
    assert!(MIGRATION.contains("campaign.wave_denominator_id=NEW.source_wave_denominator_id"));
    assert!(MIGRATION.contains("consumption.disposition='applied'"));
    assert!(MIGRATION.contains("actual_applied_set_hash<>NEW.applied_fact_delta_set_hash"));
}
