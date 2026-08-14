-- One Campaign keeps a single active PreparedAction lane. A sealed strategy may
-- contain several claim-component obligations that the same bounded action
-- observes. Permit each exact advisory apply member to reference that action
-- while retaining the compiled obligation, capability, assessment, strategy,
-- denominator and manifest hash authority checks.

CREATE OR REPLACE FUNCTION enforce_investigation_verification_advisory_campaign_apply()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    header investigation_verification_task_advisory_receipts%ROWTYPE;
    member investigation_verification_task_advisory_members%ROWTYPE;
    expected_compiler_input_sha256 TEXT;
    expected_compiler_result_authority_sha256 TEXT;
    expected_apply_sha256 TEXT;
BEGIN
    SELECT * INTO STRICT header
      FROM investigation_verification_task_advisory_receipts
     WHERE advisory_receipt_id=NEW.advisory_receipt_id FOR SHARE;
    SELECT * INTO STRICT member
      FROM investigation_verification_task_advisory_members
     WHERE advisory_member_id=NEW.advisory_member_id
       AND advisory_receipt_id=NEW.advisory_receipt_id
       AND campaign_id=NEW.campaign_id FOR SHARE;
    expected_compiler_input_sha256 :=
        investigation_verification_action_compiler_input_sha256_v1(
            NEW.advisory_member_id,NEW.strategy_artifact_id,
            NEW.strategy_obligation_id,NEW.campaign_denominator_id,
            NEW.campaign_coverage_member_id
        );
    expected_compiler_result_authority_sha256 :=
        investigation_verification_action_compiler_result_sha256_v1(
            NEW.result_kind,NEW.result_id
        );
    IF header.status<>'building'
       OR NEW.intent_id<>member.intent_id
       OR NEW.compiler_contract_version<>
          'investigation-verification-action-compiler.v1'
       OR NEW.compiler_input_sha256 IS DISTINCT FROM expected_compiler_input_sha256
       OR NEW.compiler_result_authority_sha256 IS DISTINCT FROM
          expected_compiler_result_authority_sha256
       OR NOT EXISTS(
            SELECT 1 FROM verification_campaign_rounds round
             WHERE round.round_id=NEW.round_id AND round.campaign_id=NEW.campaign_id
               AND round.operation_id=header.operation_id
               AND round.organization_id=header.organization_id
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_strategy_artifacts strategy
             WHERE strategy.strategy_artifact_id=NEW.strategy_artifact_id
               AND strategy.round_id=NEW.round_id AND strategy.campaign_id=NEW.campaign_id
               AND strategy.operation_id=header.operation_id
               AND strategy.organization_id=header.organization_id
               AND strategy.typed_strategy->>'strategy_id'=member.strategy_id::TEXT
               AND strategy.typed_strategy=member.typed_strategy
               AND strategy.strategy_hash=member.strategy_sha256
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_strategy_obligations obligation
            JOIN verification_campaign_coverage_members coverage
              ON coverage.campaign_denominator_id=NEW.campaign_denominator_id
             AND coverage.campaign_coverage_member_id=NEW.campaign_coverage_member_id
             AND coverage.semantic_key=obligation.semantic_key
             AND coverage.expected_capability_kind=obligation.obligation_kind
             WHERE obligation.strategy_artifact_id=NEW.strategy_artifact_id
               AND obligation.obligation_id=NEW.strategy_obligation_id
               AND obligation.disposition='planned'
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_campaign_coverage_denominators denominator
             WHERE denominator.campaign_denominator_id=NEW.campaign_denominator_id
               AND denominator.campaign_id=NEW.campaign_id
               AND denominator.operation_id=header.operation_id
               AND denominator.organization_id=header.organization_id
               AND denominator.sealed_at IS NOT NULL
       )
       OR (NEW.result_kind='prepared_action' AND NOT EXISTS(
            SELECT 1 FROM verification_prepared_actions action
             WHERE action.prepared_action_id=NEW.result_id
               AND action.campaign_id=NEW.campaign_id AND action.round_id=NEW.round_id
               AND action.strategy_artifact_id=NEW.strategy_artifact_id
               AND action.operation_id=header.operation_id
               AND action.organization_id=header.organization_id
               AND action.private_manifest_hash=NEW.result_sha256
               AND EXISTS(
                    SELECT 1
                      FROM verification_strategy_obligations compiled_obligation
                      JOIN verification_campaign_coverage_members compiled_coverage
                        ON compiled_coverage.campaign_denominator_id=
                           NEW.campaign_denominator_id
                       AND compiled_coverage.semantic_key=
                           compiled_obligation.semantic_key
                       AND compiled_coverage.expected_capability_kind=
                           compiled_obligation.obligation_kind
                       AND compiled_coverage.member_hash=
                           action.private_manifest->>'coverage_member_hash'
                     WHERE compiled_obligation.strategy_artifact_id=
                           NEW.strategy_artifact_id
                       AND compiled_obligation.obligation_id::TEXT=
                           action.private_manifest->>'strategy_obligation_id'
                       AND compiled_obligation.disposition='planned'
               )
               AND action.private_manifest->>'strategy_decision_id'=(
                    SELECT strategy.typed_strategy->>'strategy_id'
                      FROM verification_strategy_artifacts strategy
                     WHERE strategy.strategy_artifact_id=NEW.strategy_artifact_id
               )
               AND action.private_manifest->>'strategy_decision_hash'=(
                    SELECT strategy.strategy_hash
                      FROM verification_strategy_artifacts strategy
                     WHERE strategy.strategy_artifact_id=NEW.strategy_artifact_id
               )
               AND action.private_manifest->>'capability_id'=(
                    SELECT coverage.expected_capability_kind
                      FROM verification_campaign_coverage_members coverage
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'capability_assessment_id'=(
                    SELECT coverage.capability_assessment_id::TEXT
                      FROM verification_campaign_coverage_members coverage
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'capability_assessment_set_hash'=(
                    SELECT assessment_set.member_set_hash
                      FROM verification_campaigns campaign
                      JOIN verification_capability_assessment_set_seals assessment_set
                        ON assessment_set.assessment_set_seal_id=
                           campaign.capability_assessment_set_seal_id
                     WHERE campaign.campaign_id=NEW.campaign_id
               )
               AND action.private_manifest->>'capability_registry_contract_hash'=(
                    SELECT assessment_set.registry_contract_hash
                      FROM verification_campaigns campaign
                      JOIN verification_capability_assessment_set_seals assessment_set
                        ON assessment_set.assessment_set_seal_id=
                           campaign.capability_assessment_set_seal_id
                     WHERE campaign.campaign_id=NEW.campaign_id
               )
               AND NEW.compiler_detail_sha256 IS NULL
       ))
       OR (NEW.result_kind='residual' AND NOT EXISTS(
            SELECT 1 FROM hypothesis_residual_risks residual
             WHERE residual.residual_id=NEW.result_id
               AND residual.operation_id=header.operation_id
               AND residual.organization_id=header.organization_id
               AND residual.revision_id=header.hypothesis_revision_id
               AND residual.closed_at IS NULL
               AND residual.reason_code='investigation_verification_action_not_compilable'
               AND residual.owner_kind='plan_c'
               AND residual.residual_hash=NEW.result_sha256
               AND jsonb_typeof(residual.affected_inputs)='array'
               AND jsonb_array_length(residual.affected_inputs)=4
               AND residual.affected_inputs=jsonb_build_array(
                    'verification_task:' || header.verification_task_id::TEXT,
                    'campaign:' || NEW.campaign_id::TEXT,
                    'strategy_obligation:' || NEW.strategy_obligation_id::TEXT,
                    residual.affected_inputs->>3
               )
               AND residual.affected_inputs->>3 ~
                   '^compiler_detail_sha256:sha256:[0-9a-f]{64}$'
               AND residual.next_action=jsonb_build_object(
                    'kind','verification_strategy_refinement_required','retry',FALSE
               )
               AND residual.residual_hash=tool_truth_sha256(jsonb_build_object(
                    'reason_code','investigation_verification_action_not_compilable',
                    'affected_inputs',residual.affected_inputs,
                    'next_action',residual.next_action,
                    'compiler_detail_sha256',substr(
                        residual.affected_inputs->>3,
                        length('compiler_detail_sha256:')+1
                    )
               )::TEXT)
               AND NEW.compiler_detail_sha256=substr(
                    residual.affected_inputs->>3,
                    length('compiler_detail_sha256:')+1
               )
       ))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_CAMPAIGN_APPLY_AUTHORITY_MISMATCH';
    END IF;
    expected_apply_sha256 := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-campaign-apply.v1',
        'advisory_receipt_id',NEW.advisory_receipt_id,
        'advisory_member_id',NEW.advisory_member_id,
        'campaign_id',NEW.campaign_id,'round_id',NEW.round_id,
        'strategy_artifact_id',NEW.strategy_artifact_id,
        'strategy_obligation_id',NEW.strategy_obligation_id,
        'campaign_denominator_id',NEW.campaign_denominator_id,
        'campaign_coverage_member_id',NEW.campaign_coverage_member_id,
        'intent_id',NEW.intent_id,
        'compiler_contract_version',NEW.compiler_contract_version,
        'compiler_input_sha256',NEW.compiler_input_sha256,
        'compiler_result_authority_sha256',NEW.compiler_result_authority_sha256,
        'compiler_detail_sha256',NEW.compiler_detail_sha256,
        'result_kind',NEW.result_kind,'result_id',NEW.result_id,
        'result_sha256',NEW.result_sha256
    )::TEXT);
    IF NEW.apply_sha256<>expected_apply_sha256 THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_CAMPAIGN_APPLY_HASH_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;
