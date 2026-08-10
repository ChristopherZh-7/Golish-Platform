-- Synchronize the deferred Application Model current-authority trigger with
-- the finalizer's exact AU internal-control policy.
--
-- This is deliberately a function-only forward migration: it changes no
-- table, column, constraint, or persisted row. The source guard is matched
-- exactly once and the migration fails closed if the installed predecessor
-- function differs from the reviewed definition.
DO $migration$
DECLARE
    function_definition TEXT;
    old_guard TEXT := $old_guard$
       OR EXISTS (
           SELECT 1 FROM tool_calls AS tool
            WHERE tool.operation_id=manifest.operation_id
              AND tool.stage_execution_id=manifest.stage_execution_id
              AND tool.stage_run_unit_id=manifest.stage_run_unit_id
              AND tool.name NOT IN ('submit_stage_deliverable','update_plan')
       )
$old_guard$;
    new_guard TEXT := $new_guard$
       OR EXISTS (
           SELECT 1 FROM tool_calls AS tool
            WHERE tool.operation_id=manifest.operation_id
              AND tool.stage_execution_id=manifest.stage_execution_id
              AND tool.stage_run_unit_id=manifest.stage_run_unit_id
              AND tool.name NOT IN ('submit_stage_deliverable','update_plan')
              AND NOT (
                  tool.name='submit_result'
                  AND EXISTS (
                      SELECT 1
                        FROM stage_worker_runs AS producer
                        JOIN stage_work_items AS item
                          ON item.id=producer.work_item_id
                         AND item.operation_id=producer.operation_id
                         AND item.stage_execution_id=producer.stage_execution_id
                         AND item.stage_run_unit_id=producer.stage_run_unit_id
                         AND item.organization_id=producer.organization_id
                        JOIN stage_team_plans AS plan
                          ON plan.id=item.team_plan_id
                         AND plan.operation_id=item.operation_id
                         AND plan.stage_execution_id=item.stage_execution_id
                         AND plan.stage_run_unit_id=item.stage_run_unit_id
                         AND plan.organization_id=item.organization_id
                       WHERE producer.id=tool.worker_run_id
                         AND producer.operation_id=tool.operation_id
                         AND producer.stage_execution_id=tool.stage_execution_id
                         AND producer.stage_run_unit_id=tool.stage_run_unit_id
                         AND producer.organization_id=tool.organization_id
                         AND plan.stage_kind='application_understanding'
                         AND plan.dynamic_request_policy->>'formulaic_worklist_executor'=
                             'application_model_v1'
                         AND (
                             (item.role='application_model_worker'
                              AND item.output_schema='application_model_work_item_output.v1')
                             OR
                             (item.role='application_model_synthesizer'
                              AND item.output_schema='application_model_proposal.v1')
                         )
                  )
              )
       )
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
            'APPLICATION_MODEL_CURRENT_AUTHORITY_GUARD_SOURCE_MISMATCH: expected 1, found %',
            source_matches;
    END IF;

    function_definition := replace(function_definition, old_guard, new_guard);
    IF position(new_guard IN function_definition) = 0 THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_AUTHORITY_GUARD_REPLACEMENT_FAILED';
    END IF;
    EXECUTE function_definition;
END;
$migration$;
