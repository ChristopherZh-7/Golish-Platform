use golish_core::{InvestigationContractVersion, InvestigationRolloutMode};
use golish_db::repo::operation_default_rollout::{
    operation_promotion_component_member_hash, operation_promotion_component_set_hash,
    validate_operation_promotion_component_rows, OperationPromotionComponentRow,
    OPERATION_PROMOTION_COMPONENT_KINDS,
};
use golish_db::repo::operation_rollout::{
    joint_contract_rank, promotion_evidence_shape, OperationPromotionCriteriaV1,
    OperationPromotionTransitionError,
};
use golish_pentest_domain::tool_truth::ToolTruthContract;

#[test]
fn joint_contract_rank_is_closed_and_monotonic() {
    use InvestigationContractVersion::{HypothesisRegistryV1, LegacyCandidateV1};
    use InvestigationRolloutMode::{
        DualReadCompare, LegacyOnly, NewOnly, RegistryAuthoritativeLegacyProjection, ShadowRegistry,
    };
    use ToolTruthContract::{LegacyV1, ReceiptV1, ShadowV1};

    let cases = [
        (0, LegacyV1, LegacyCandidateV1, LegacyOnly),
        (1, ShadowV1, LegacyCandidateV1, LegacyOnly),
        (2, ShadowV1, HypothesisRegistryV1, ShadowRegistry),
        (3, ShadowV1, HypothesisRegistryV1, DualReadCompare),
        (4, ReceiptV1, HypothesisRegistryV1, DualReadCompare),
        (
            5,
            ReceiptV1,
            HypothesisRegistryV1,
            RegistryAuthoritativeLegacyProjection,
        ),
        (6, ReceiptV1, HypothesisRegistryV1, NewOnly),
    ];
    for (rank, tool_truth, investigation_contract, investigation_mode) in cases {
        assert_eq!(
            joint_contract_rank(tool_truth, investigation_contract, investigation_mode),
            Some(rank),
        );
    }
    assert_eq!(
        joint_contract_rank(
            ShadowV1,
            LegacyCandidateV1,
            InvestigationRolloutMode::ShadowRegistry,
        ),
        None,
    );
}

#[test]
fn promotion_edge_criteria_are_closed_and_edge_specific() {
    use OperationPromotionCriteriaV1::{
        ClosedShadowCohortExact, DualAndAuthoritativeCanaryExact, LegacyConsumersRetired,
        ShadowEvaluatorReady, ToolTruthReceiptReconciliationExact, ToolTruthShadowWriterReady,
    };

    let expected = [
        (0, ToolTruthShadowWriterReady),
        (1, ShadowEvaluatorReady),
        (2, ClosedShadowCohortExact),
        (3, ToolTruthReceiptReconciliationExact),
        (4, DualAndAuthoritativeCanaryExact),
        (5, LegacyConsumersRetired),
    ];
    for (from_rank, criteria) in expected {
        let shape = promotion_evidence_shape(from_rank, from_rank + 1).expect("adjacent edge");
        assert_eq!(shape.criteria, criteria);
        assert_eq!(
            shape.requires_positive_comparison_cohort,
            matches!(from_rank, 2 | 4),
        );
        assert_eq!(shape.requires_authoritative_canary, from_rank == 4);
        assert_eq!(shape.requires_adversarial_acceptance_corpus, from_rank == 4,);
    }
}

#[test]
fn promotion_edge_rejects_skip_downgrade_and_out_of_range() {
    assert_eq!(
        promotion_evidence_shape(2, 4),
        Err(OperationPromotionTransitionError::RankSkipped),
    );
    assert_eq!(
        promotion_evidence_shape(4, 3),
        Err(OperationPromotionTransitionError::RankNotForward),
    );
    assert_eq!(
        promotion_evidence_shape(-1, 0),
        Err(OperationPromotionTransitionError::SourceRankInvalid),
    );
    assert_eq!(
        promotion_evidence_shape(6, 7),
        Err(OperationPromotionTransitionError::TargetRankInvalid),
    );
}

#[test]
fn plan_d_migration_reuses_plan_b_projection_authority() {
    let plan_b = include_str!("../migrations/20260729000006_hypothesis_registry.sql");
    let plan_d = include_str!("../migrations/20260729000008_investigation_projection.sql");
    for relation in [
        "investigation_rollout",
        "investigation_projection_outbox_batches",
        "investigation_projection_heads",
        "investigation_projection_changes",
        "investigation_projection_compare_samples",
    ] {
        assert!(plan_b.contains(&format!("CREATE TABLE {relation}")));
        assert!(!plan_d.contains(&format!("CREATE TABLE {relation}")));
    }
    assert!(!plan_d.contains("investigation_shadow_comparisons"));
}

#[test]
fn fresh_install_defaults_are_pristine_only_exact_and_audited() {
    let sql = include_str!("../migrations/20260810000003_fresh_install_full_chain_defaults.sql");
    for guard in [
        "LOCK TABLE operation_state IN SHARE ROW EXCLUSIVE MODE",
        "IF operation_count<>0 THEN",
        "FRESH_INSTALL_FULL_CHAIN_DEFAULT_SOURCE_DRIFT",
        "dual_write_legacy_read",
        "dual_write_read_legacy",
        "agent_team_v2",
        "receipt_v1",
        "hypothesis_registry_v1",
        "new_only",
        "unified_investigation_v1",
        "fresh_install_full_chain_bootstrap_receipts_append_only",
    ] {
        assert!(sql.contains(guard), "missing {guard}");
    }
    assert!(!sql.contains("UPDATE operation_state"));
    assert!(!sql.contains("DELETE FROM"));
    assert!(!sql.contains("TRUNCATE"));
}

#[test]
fn analysis_post_synthesis_rearm_migration_keeps_blocked_terminal_by_default() {
    let sql = include_str!(
        "../migrations/20260809000002_investigation_analysis_post_synthesis_rearm.sql"
    );
    for exact_witness in [
        "post_synthesis_analysis_recovery.v1",
        "investigation_analysis_host_authority_mismatch",
        "investigation_primary_recovery",
        "sealed-investigation-synthesis-recovery-primary-v2",
        "synthesis.event_sha256=split_part(rearm.reason_code,'|',2)",
        "tool_truth_sha256(recovery_worker.checkpoint::TEXT)",
        "@.name == \"submit_result\"",
        "decision.binding_id=binding.binding_id",
    ] {
        assert!(sql.contains(exact_witness), "missing {exact_witness}");
    }
    assert!(sql.contains(
        "unified_investigation_work_transition_allowed(work.current_state,NEW.to_state)\n        OR unified_investigation_post_synthesis_analysis_rearm_allowed(work,NEW)"
    ));
    assert!(!sql.contains("('blocked','running')"));
}

#[test]
fn normal_primary_post_synthesis_rearm_requires_the_completed_sealed_witness() {
    let sql = include_str!(
        "../migrations/20260809000003_investigation_analysis_primary_post_synthesis_rearm.sql"
    );
    for exact_witness in [
        "post_synthesis_analysis_primary_recovery.v1",
        "investigation_analysis_host_infrastructure",
        "investigation_analysis_host_authority_mismatch",
        "primary_item.stable_key='leader:primary'",
        "primary_item.kind='investigation_primary'",
        "primary_item.status='completed'",
        "primary_worker.status='passed'",
        "task_plan.status='sealed'",
        "synthesis.event_sha256=split_part(rearm.reason_code,'|',2)",
        "tool_truth_sha256(primary_worker.checkpoint::TEXT)",
        "NOT EXISTS(",
        "decision.binding_id=binding.binding_id",
    ] {
        assert!(sql.contains(exact_witness), "missing {exact_witness}");
    }
    assert!(sql.contains(
        "OR unified_investigation_post_synthesis_analysis_rearm_allowed(work,NEW)\n        OR unified_investigation_primary_post_synthesis_analysis_rearm_allowed(work,NEW)"
    ));
    assert!(!sql.contains("('blocked','running')"));
}

#[test]
fn checkpoint_submit_result_parser_preserves_migration_history_and_binds_all_runtime_reads() {
    let parser = include_str!(
        "../migrations/20260809000004_investigation_checkpoint_submit_result_parser.sql"
    );
    for exact_guard in [
        "jsonb_typeof(raw_result)='object'",
        "jsonb_typeof(raw_result)='string'",
        "EXCEPTION WHEN invalid_text_representation",
        "COALESCE(cardinality(matches),0)<>1",
        "parsed_result->'schema_version'<>'1'::JSONB",
        "INVESTIGATION_CHECKPOINT_PARSER_MIGRATION_SOURCE_DRIFT",
        "pg_get_functiondef(function_name::REGPROC)",
    ] {
        assert!(parser.contains(exact_guard), "missing {exact_guard}");
    }
    for source in [
        include_str!("../src/repo/runtime_memory_tx.rs"),
        include_str!("../../golish-agent-app/src/ai/db_bridge/investigation_analysis_host.rs"),
        include_str!("../../golish/src/stage_run/runtime_v2.rs"),
    ] {
        assert!(source.contains("unified_investigation_submit_result_v1("));
        assert!(!source.contains("@.arguments.result.schema_version == 1"));
    }
    assert!(include_str!(
        "../migrations/20260809000002_investigation_analysis_post_synthesis_rearm.sql"
    )
    .contains("@.arguments.result.schema_version == 1"));
    assert!(include_str!(
        "../migrations/20260809000003_investigation_analysis_primary_post_synthesis_rearm.sql"
    )
    .contains("@.arguments.result.schema_version == 1"));
}

#[test]
fn verification_advisory_header_uses_the_durable_worker_output_manifest() {
    let migration = include_str!(
        "../migrations/20260809000005_investigation_advisory_durable_output_manifest.sql"
    );
    for exact_guard in [
        "unified_investigation_verification_accepted_output_hashes",
        "output.output_hash",
        "output.work_item_id=dispatch.stage_work_item_id",
        "output_worker.id=output.worker_run_id",
        "receipt_count<>expected_dispatch_count",
        "work_item_count<>expected_dispatch_count",
        "INVESTIGATION_VERIFICATION_DURABLE_OUTPUT_MANIFEST_MISMATCH",
        "INVESTIGATION_VERIFICATION_ADVISORY_TRIGGER_SOURCE_DRIFT",
    ] {
        assert!(migration.contains(exact_guard), "missing {exact_guard}");
    }
    let application = include_str!(
        "../../golish-agent-app/src/ai/db_bridge/investigation_verification_advisory.rs"
    );
    assert!(application.contains("output.output_hash AS output_sha256"));
    assert!(application.contains("output_worker.id=output.worker_run_id"));
    assert!(application.matches("\"advisory_request_id\"").count() >= 2);
}

#[test]
fn campaign_closeout_forward_fixes_are_row_shape_safe() {
    let claim_outcome = include_str!(
        "../migrations/20260809000006_verification_claim_component_outcome_authority_fix.sql"
    );
    assert!(claim_outcome.contains("expected_contract_id UUID"));
    assert!(claim_outcome.contains("binding.contract_id=expected_contract_id"));
    assert!(claim_outcome.contains("predicate.contract_id=expected_contract_id"));
    assert!(!claim_outcome.contains("DECLARE\n    contract_id UUID"));

    let campaign_child = include_str!(
        "../migrations/20260809000007_investigation_campaign_child_closure_fence_fix.sql"
    );
    assert!(campaign_child.contains("(to_jsonb(NEW)->>'state')"));
    assert!(!campaign_child.contains("AND NEW.state"));
    assert!(campaign_child.contains("investigation_assert_stage_accepts_new_work"));
    assert!(campaign_child.contains("investigation_assert_stage_not_closed"));

    let residual_authority = include_str!(
        "../migrations/20260809000009_investigation_oracle_residual_closure_authority.sql"
    );
    assert!(residual_authority.contains("unified_investigation_residual_has_stage_authority_v1"));
    assert!(residual_authority.contains("oracle_member.residual_id=residual.residual_id"));
    assert!(residual_authority.contains("oracle_member.disposition='untested'"));
    assert!(residual_authority.contains("task.stage_execution_id=p_stage_execution_id"));
    assert!(residual_authority.contains("admission_member.hypothesis_revision_id"));
}

#[test]
fn promotion_component_census_is_exact_and_hash_bound() {
    let component_hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let rows = OPERATION_PROMOTION_COMPONENT_KINDS
        .iter()
        .map(|kind| OperationPromotionComponentRow {
            component_kind: (*kind).to_string(),
            component_sha256: component_hash.to_string(),
            member_sha256: operation_promotion_component_member_hash(kind, component_hash),
        })
        .collect::<Vec<_>>();
    let census_hash = operation_promotion_component_set_hash(&rows);
    validate_operation_promotion_component_rows(&rows, &census_hash, &census_hash)
        .expect("the seven exact promotion components are accepted");

    let missing = validate_operation_promotion_component_rows(
        &rows[..rows.len() - 1],
        &census_hash,
        &census_hash,
    )
    .expect_err("a missing promotion component must fail closed");
    assert_eq!(
        missing.code(),
        "OPERATION_PROMOTION_COMPONENT_SET_INCOMPLETE"
    );

    let mut duplicate = rows.clone();
    duplicate[6] = duplicate[0].clone();
    let duplicate =
        validate_operation_promotion_component_rows(&duplicate, &census_hash, &census_hash)
            .expect_err("a duplicate promotion component must fail closed");
    assert_eq!(duplicate.code(), "OPERATION_PROMOTION_COMPONENT_DUPLICATE");

    let mut member_drift = rows.clone();
    member_drift[0].member_sha256 = component_hash.to_string();
    let member_drift =
        validate_operation_promotion_component_rows(&member_drift, &census_hash, &census_hash)
            .expect_err("component member hash drift must fail closed");
    assert_eq!(
        member_drift.code(),
        "OPERATION_PROMOTION_COMPONENT_MEMBER_HASH_DRIFT"
    );

    let set_drift = validate_operation_promotion_component_rows(
        &rows,
        &census_hash,
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .expect_err("component census hash drift must fail closed");
    assert_eq!(
        set_drift.code(),
        "OPERATION_PROMOTION_COMPONENT_SET_HASH_DRIFT"
    );
}
