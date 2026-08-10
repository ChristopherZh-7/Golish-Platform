-- Application Model publication must compare a forked manifest against the
-- exact predecessor handoffs adopted by the fork. The original current-model
-- trigger only enumerated handoffs owned by the target operation, so an exact
-- fork manifest with four valid source inputs was incorrectly compared with
-- an empty expected set.

CREATE OR REPLACE FUNCTION application_model_expected_source_handoff_ids(
    requested_manifest_id UUID
)
RETURNS UUID[]
STABLE
LANGUAGE SQL
AS $$
    SELECT COALESCE(array_agg(source.id ORDER BY source.id), '{}'::UUID[])
      FROM application_model_manifests AS manifest
      JOIN stage_handoffs AS source
        ON source.organization_id=manifest.organization_id
      JOIN stage_run_units AS source_unit
        ON source_unit.id=source.source_stage_run_unit_id
       AND source_unit.operation_id=source.operation_id
       AND source_unit.stage_execution_id=source.stage_execution_id
       AND source_unit.organization_id=source.organization_id
       AND source_unit.stage_kind=source.from_stage_kind
     WHERE manifest.id=requested_manifest_id
       AND source.from_stage_kind IN (
           'target_intel',
           'external_attack_surface',
           'enumeration',
           'vuln_triage'
       )
       AND source.invalidated_at IS NULL
       AND source_unit.status='passed'
       AND (
            (
                source.operation_id=manifest.operation_id
                AND source.scope_snapshot_id=manifest.scope_snapshot_id
            )
            OR EXISTS(
                SELECT 1
                  FROM operation_stage_fork_inputs AS fork_input
                  JOIN operation_stage_forks AS fork
                    ON fork.operation_id=fork_input.operation_id
                   AND fork.source_operation_id=fork_input.source_operation_id
                  JOIN operation_state AS source_operation
                    ON source_operation.operation_id=fork_input.source_operation_id
                   AND source_operation.superseded_by IS NULL
                 WHERE fork_input.operation_id=manifest.operation_id
                   AND fork_input.target_scope_snapshot_id=manifest.scope_snapshot_id
                   AND fork_input.organization_id=manifest.organization_id
                   AND fork_input.source_handoff_id=source.id
                   AND fork_input.source_operation_id=source.operation_id
                   AND fork_input.source_scope_snapshot_id=source.scope_snapshot_id
                   AND fork_input.source_stage_kind=source.from_stage_kind
                   AND fork_input.source_stage_execution_id=source.stage_execution_id
                   AND fork_input.source_stage_run_unit_id=source.source_stage_run_unit_id
                   AND fork_input.source_deliverable_submission_id=
                        source.deliverable_submission_id
                   AND fork_input.source_scope_hash=source.scope_hash
                   AND fork_input.source_payload=source.payload
                   AND fork_input.source_payload_sha256=source.payload_sha256
                   AND fork_input.source_evidence_ids=source.evidence_ids
                   AND fork_input.source_coverage_watermark=source.coverage_watermark
                   AND fork_input.source_unit_gate_decision_hash=
                        source.unit_gate_decision_hash
                   AND fork_input.source_aggregate_pass_token_hash
                        IS NOT DISTINCT FROM source.aggregate_pass_token_hash
                   AND fork_input.source_gate_passed_at=source.gate_passed_at
            )
       )
$$;

DO $migration$
DECLARE
    function_definition TEXT;
    old_query TEXT := $old_query$
    SELECT COALESCE(array_agg(source.id ORDER BY source.id), '{}'::UUID[])
      INTO expected_source_handoff_ids
      FROM stage_handoffs AS source
      JOIN stage_run_units AS source_unit
        ON source_unit.id=source.source_stage_run_unit_id
       AND source_unit.operation_id=source.operation_id
       AND source_unit.stage_execution_id=source.stage_execution_id
       AND source_unit.organization_id=source.organization_id
       AND source_unit.stage_kind=source.from_stage_kind
     WHERE source.operation_id=manifest.operation_id
       AND source.scope_snapshot_id=manifest.scope_snapshot_id
       AND source.organization_id=manifest.organization_id
       AND source.from_stage_kind IN (
           'target_intel',
           'external_attack_surface',
           'enumeration',
           'vuln_triage'
       )
       AND source.invalidated_at IS NULL
       AND source_unit.status='passed';
$old_query$;
    new_query TEXT := $new_query$
    SELECT application_model_expected_source_handoff_ids(manifest.id)
      INTO expected_source_handoff_ids;
$new_query$;
    source_matches INTEGER;
BEGIN
    SELECT pg_get_functiondef(
               'application_model_validate_current_revision()'::REGPROCEDURE
           )
      INTO STRICT function_definition;

    source_matches := (
        length(function_definition) -
        length(replace(function_definition, old_query, ''))
    ) / length(old_query);
    IF source_matches <> 1 THEN
        RAISE EXCEPTION
            'APPLICATION_MODEL_FORK_EXPECTED_SET_SOURCE_MISMATCH: expected 1, found %',
            source_matches;
    END IF;

    function_definition := replace(function_definition, old_query, new_query);
    IF position(new_query IN function_definition) = 0 THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FORK_EXPECTED_SET_REPLACEMENT_FAILED';
    END IF;
    EXECUTE function_definition;
END;
$migration$;
