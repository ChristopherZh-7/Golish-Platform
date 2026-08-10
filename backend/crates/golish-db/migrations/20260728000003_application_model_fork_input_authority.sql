-- Admit immutable stage-fork predecessor seals as Application Model manifest
-- inputs without weakening the ordinary same-operation authority contract.
--
-- A fork row is accepted only when every frozen identity and seal field still
-- matches the original non-invalidated Handoff and its passed source Unit.

CREATE OR REPLACE FUNCTION application_model_validate_manifest_input_source()
RETURNS trigger AS $$
DECLARE
    manifest application_model_manifests%ROWTYPE;
    handoff stage_handoffs%ROWTYPE;
    source_unit_status TEXT;
    direct_authority BOOLEAN;
    fork_authority BOOLEAN;
BEGIN
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id = NEW.manifest_id
     FOR SHARE;
    IF manifest.authority_kind <> 'model' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_INPUTS';
    END IF;
    IF ROW(
        NEW.operation_id,
        NEW.scope_snapshot_id,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id,
        NEW.organization_id
    ) IS DISTINCT FROM ROW(
        manifest.operation_id,
        manifest.scope_snapshot_id,
        manifest.stage_execution_id,
        manifest.stage_run_unit_id,
        manifest.organization_id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_INPUT_OWNER_MISMATCH';
    END IF;
    SELECT * INTO STRICT handoff
      FROM stage_handoffs
     WHERE id = NEW.source_handoff_id
     FOR SHARE;
    SELECT status INTO STRICT source_unit_status
      FROM stage_run_units
     WHERE id = handoff.source_stage_run_unit_id
       AND operation_id = handoff.operation_id
       AND stage_execution_id = handoff.stage_execution_id
       AND organization_id = handoff.organization_id
       AND stage_kind = handoff.from_stage_kind
     FOR SHARE;

    direct_authority :=
        handoff.operation_id = manifest.operation_id
        AND handoff.scope_snapshot_id = manifest.scope_snapshot_id
        AND handoff.organization_id = manifest.organization_id;

    SELECT EXISTS(
        SELECT 1
          FROM operation_stage_fork_inputs AS input
          JOIN operation_stage_forks AS fork
            ON fork.operation_id = input.operation_id
           AND fork.source_operation_id = input.source_operation_id
          JOIN operation_state AS source_operation
            ON source_operation.operation_id = input.source_operation_id
           AND source_operation.superseded_by IS NULL
         WHERE input.operation_id = manifest.operation_id
           AND input.target_scope_snapshot_id = manifest.scope_snapshot_id
           AND input.organization_id = manifest.organization_id
           AND input.source_handoff_id = handoff.id
           AND input.source_operation_id = handoff.operation_id
           AND input.source_scope_snapshot_id = handoff.scope_snapshot_id
           AND input.source_stage_kind = handoff.from_stage_kind
           AND input.source_stage_execution_id = handoff.stage_execution_id
           AND input.source_stage_run_unit_id = handoff.source_stage_run_unit_id
           AND input.source_deliverable_submission_id = handoff.deliverable_submission_id
           AND input.source_scope_hash = handoff.scope_hash
           AND input.source_payload = handoff.payload
           AND input.source_payload_sha256 = handoff.payload_sha256
           AND input.source_evidence_ids = handoff.evidence_ids
           AND input.source_coverage_watermark = handoff.coverage_watermark
           AND input.source_unit_gate_decision_hash = handoff.unit_gate_decision_hash
           AND input.source_aggregate_pass_token_hash
                   IS NOT DISTINCT FROM handoff.aggregate_pass_token_hash
           AND input.source_gate_passed_at = handoff.gate_passed_at
    ) INTO fork_authority;

    IF NOT (direct_authority OR fork_authority)
       OR handoff.invalidated_at IS NOT NULL
       OR source_unit_status <> 'passed'
       OR NEW.source_kind <> handoff.from_stage_kind
       OR NEW.source_id <> handoff.id::TEXT
       OR NEW.source_version <> handoff.schema_version
       OR NEW.source_payload <> handoff.payload
       OR ('sha256:' || handoff.payload_sha256) <> NEW.source_payload_hash
       OR NOT NEW.evidence_ids <@ handoff.evidence_ids
       OR application_model_sha256_jsonb(NEW.source_payload) <> NEW.source_payload_hash
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_INPUT_SOURCE_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
