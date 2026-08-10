-- Preserve the same immutable stage-fork predecessor authority from manifest
-- seeding through final Application Model publication. Ordinary operations
-- still require same-operation/same-snapshot ownership; a fork is admitted
-- only when every copied seal field remains exact and the source operation is
-- still the live source named by the fork header.

CREATE OR REPLACE FUNCTION application_model_manifest_input_source_owner_is_authorized(
    requested_manifest_id UUID,
    requested_source_handoff_id UUID
)
RETURNS BOOLEAN
STABLE
LANGUAGE SQL
AS $$
    SELECT EXISTS(
        SELECT 1
          FROM application_model_manifests AS manifest
          JOIN application_model_manifest_inputs AS input
            ON input.manifest_id=manifest.id
           AND input.source_handoff_id=requested_source_handoff_id
          JOIN stage_handoffs AS source
            ON source.id=input.source_handoff_id
         WHERE manifest.id=requested_manifest_id
           AND (
                (
                    source.operation_id=manifest.operation_id
                    AND source.scope_snapshot_id=manifest.scope_snapshot_id
                    AND source.organization_id=manifest.organization_id
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
    )
$$;

DO $migration$
DECLARE
    function_definition TEXT;
    old_guard TEXT := $old_guard$
                  source.invalidated_at IS NOT NULL
                  OR source_unit.status<>'passed'
                  OR source.operation_id<>manifest.operation_id
                  OR source.scope_snapshot_id<>manifest.scope_snapshot_id
                  OR source.organization_id<>manifest.organization_id
                  OR input.source_kind<>source.from_stage_kind
$old_guard$;
    new_guard TEXT := $new_guard$
                  source.invalidated_at IS NOT NULL
                  OR source_unit.status<>'passed'
                  OR NOT application_model_manifest_input_source_owner_is_authorized(
                        manifest.id,
                        source.id
                     )
                  OR input.source_kind<>source.from_stage_kind
$new_guard$;
    source_matches INTEGER;
BEGIN
    SELECT pg_get_functiondef(
               'application_model_validate_current_revision()'::REGPROCEDURE
           )
      INTO STRICT function_definition;

    source_matches := (
        length(function_definition) -
        length(replace(function_definition, old_guard, ''))
    ) / length(old_guard);
    IF source_matches <> 1 THEN
        RAISE EXCEPTION
            'APPLICATION_MODEL_FORK_PUBLICATION_GUARD_SOURCE_MISMATCH: expected 1, found %',
            source_matches;
    END IF;

    function_definition := replace(function_definition, old_guard, new_guard);
    IF position(new_guard IN function_definition) = 0 THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FORK_PUBLICATION_GUARD_REPLACEMENT_FAILED';
    END IF;
    EXECUTE function_definition;
END;
$migration$;
