-- A host-stage reconciliation performs no transport. It may close a receipt
-- only under an enforced, sealed-empty sandboxed_cli policy; every other
-- complete receipt still requires at least one sealed network hop.
CREATE OR REPLACE FUNCTION tool_truth_guard_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    denominator_hash TEXT;
    expected_receipt_hash TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'tool_truth_receipt_append_only' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(NEW) - ARRAY[
                'attempt_state','landing_state','observation_state','coverage_extent',
                'coverage_gap_reason','reconciliation_state','security_interpretation',
                'typed_landing','residual','finalization_request_hash',
                'raw_witness_artifact_id','parser_census_id',
                'temporal_census_id','current_semantic_authority_version',
                'current_semantic_reconciliation_id','current_semantic_reconciliation_hash',
                'row_version','observation_completed_at','valid_until','finalized_at'
            ]) IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'attempt_state','landing_state','observation_state','coverage_extent',
                'coverage_gap_reason','reconciliation_state','security_interpretation',
                'typed_landing','residual','finalization_request_hash',
                'raw_witness_artifact_id','parser_census_id',
                'temporal_census_id','current_semantic_authority_version',
                'current_semantic_reconciliation_id','current_semantic_reconciliation_hash',
                'row_version','observation_completed_at','valid_until','finalized_at'
            ]) THEN
            RAISE EXCEPTION 'tool_truth_receipt_authority_immutable' USING ERRCODE='23514';
        END IF;
        IF NEW.row_version<>OLD.row_version+1 THEN
            RAISE EXCEPTION 'tool_truth_receipt_cas_required' USING ERRCODE='23514';
        END IF;
        IF (OLD.raw_witness_artifact_id IS NOT NULL
                AND NEW.raw_witness_artifact_id IS DISTINCT FROM OLD.raw_witness_artifact_id)
           OR (OLD.parser_census_id IS NOT NULL
                AND NEW.parser_census_id IS DISTINCT FROM OLD.parser_census_id)
           OR (OLD.temporal_census_id IS NOT NULL
                AND NEW.temporal_census_id IS DISTINCT FROM OLD.temporal_census_id)
           OR (OLD.finalized_at IS NOT NULL AND NEW.finalized_at IS DISTINCT FROM OLD.finalized_at) THEN
            RAISE EXCEPTION 'tool_truth_receipt_terminal_binding_immutable' USING ERRCODE='23514';
        END IF;
        IF NEW.current_semantic_authority_version NOT IN (
                OLD.current_semantic_authority_version,
                OLD.current_semantic_authority_version+1
            ) THEN
            RAISE EXCEPTION 'tool_truth_receipt_semantic_version_invalid' USING ERRCODE='23514';
        END IF;
        IF NEW.coverage_extent='complete' AND (
            NEW.finalization_request_hash IS NULL
            OR (
                NOT EXISTS (
                    SELECT 1 FROM capability_execution_network_hops h
                     WHERE h.receipt_id=NEW.id AND h.sealed_at IS NOT NULL
                )
                AND NOT EXISTS (
                    SELECT 1 FROM capability_execution_destination_policies p
                     WHERE p.id=NEW.destination_policy_id
                       AND p.execution_authority_id=NEW.execution_authority_id
                       AND p.policy_hash=NEW.destination_policy_hash
                       AND p.execution_backend='sandboxed_cli'
                       AND p.governance_status='enforced'
                       AND p.sealed_at IS NOT NULL
                       AND p.sealed_empty
                       AND p.member_count=0
                )
            )
            OR EXISTS (
                SELECT 1 FROM capability_execution_receipt_inputs i
                 WHERE i.receipt_id=NEW.id
                   AND (i.sealed_at IS NULL OR i.coverage_extent<>'complete'
                        OR COALESCE(i.member_count,0)=0)
            )
            OR (SELECT count(*) FROM capability_execution_receipt_inputs i
                 WHERE i.receipt_id=NEW.id)
               <> (SELECT count(*) FROM coverage_denominator_items d
                    WHERE d.denominator_id=NEW.denominator_id
                      AND d.expected_capability=NEW.capability)
            OR EXISTS (
                SELECT 1 FROM capability_execution_budget_contract_axes c
                LEFT JOIN capability_execution_budget_observations o
                  ON o.receipt_id=c.receipt_id AND o.axis=c.axis
                 AND o.execution_authority_id=c.execution_authority_id
                WHERE c.receipt_id=NEW.id AND c.required_for_complete
                  AND (o.receipt_id IS NULL OR NOT o.observed)
            )
            OR NOT EXISTS (
                SELECT 1 FROM capability_execution_reconciliations r
                 WHERE r.id=NEW.current_semantic_reconciliation_id
                   AND r.receipt_id=NEW.id AND r.reconciliation_state='consistent'
                   AND r.sealed_at IS NOT NULL AND COALESCE(r.member_count,0)>0
            )
        ) THEN
            RAISE EXCEPTION 'tool_truth_receipt_complete_authority_missing' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;

    SELECT d.denominator_hash INTO denominator_hash
      FROM coverage_denominators d
     WHERE d.id=NEW.denominator_id
       AND d.execution_authority_id=NEW.execution_authority_id
       AND d.input_manifest_hash=NEW.input_manifest_hash
       AND d.sealed_at IS NOT NULL
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tool_truth_denominator_unsealed_or_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM capability_execution_destination_policies p
         WHERE p.id=NEW.destination_policy_id
           AND p.execution_authority_id=NEW.execution_authority_id
           AND p.policy_hash=NEW.destination_policy_hash
           AND p.sealed_at IS NOT NULL FOR SHARE
    ) OR NOT EXISTS (
        SELECT 1 FROM evidence_temporal_validity_policies p
         WHERE p.id=NEW.temporal_validity_policy_id
           AND p.execution_authority_id=NEW.execution_authority_id
           AND p.policy_hash=NEW.temporal_validity_policy_hash
           AND p.sealed_at IS NOT NULL FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_receipt_policy_unsealed_or_mismatch' USING ERRCODE='23514';
    END IF;
    expected_receipt_hash := tool_truth_sha256(jsonb_build_object(
        'denominator_id',NEW.denominator_id,
        'denominator_hash',denominator_hash,
        'execution_authority_id',NEW.execution_authority_id,
        'capability',NEW.capability,
        'attempt_ordinal',NEW.attempt_ordinal,
        'input_manifest_hash',NEW.input_manifest_hash,
        'destination_policy_hash',NEW.destination_policy_hash,
        'temporal_validity_policy_hash',NEW.temporal_validity_policy_hash
    )::TEXT);
    NEW.receipt_authority_hash := expected_receipt_hash;
    RETURN NEW;
END;
$$;
