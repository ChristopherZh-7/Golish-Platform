-- The advisory header is evidence authority, so its accepted-output set must
-- contain the immutable StageWorkerOutput hashes consumed by Primary
-- synthesis. Logical-dispatch attempt hashes are transport receipts and must
-- not be substituted for those evidence-bearing outputs.

CREATE FUNCTION unified_investigation_verification_accepted_output_hashes(
    input_task_plan_id UUID
)
RETURNS TEXT[]
LANGUAGE plpgsql
STABLE
STRICT
AS $$
DECLARE
    expected_dispatch_count BIGINT;
    row_count BIGINT;
    receipt_count BIGINT;
    work_item_count BIGINT;
    raw_output_hashes TEXT[];
    canonical_output_hashes TEXT[];
BEGIN
    SELECT COUNT(*)
      INTO expected_dispatch_count
      FROM pentagi_logical_dispatch_receipts dispatch
     WHERE dispatch.task_plan_id=input_task_plan_id
       AND dispatch.actor_kind IN ('worker','nested_worker');

    SELECT COUNT(*),
           COUNT(DISTINCT durable.dispatch_receipt_id),
           COUNT(DISTINCT durable.stage_work_item_id),
           COALESCE(array_agg(durable.output_hash ORDER BY durable.output_hash),
                    ARRAY[]::TEXT[])
      INTO row_count,receipt_count,work_item_count,raw_output_hashes
      FROM (
          SELECT dispatch.dispatch_receipt_id,
                 dispatch.stage_work_item_id,
                 output.output_hash
            FROM pentagi_logical_dispatch_receipts dispatch
            JOIN investigation_pentagi_task_plans task_plan
              ON task_plan.task_plan_id=dispatch.task_plan_id
             AND task_plan.operation_id=dispatch.operation_id
             AND task_plan.stage_execution_id=dispatch.stage_execution_id
             AND task_plan.stage_run_unit_id=dispatch.stage_run_unit_id
             AND task_plan.scope_snapshot_id=dispatch.scope_snapshot_id
             AND task_plan.organization_id=dispatch.organization_id
            JOIN stage_team_plans team_plan
              ON team_plan.id=task_plan.stage_team_plan_id
             AND team_plan.operation_id=task_plan.operation_id
             AND team_plan.stage_execution_id=task_plan.stage_execution_id
             AND team_plan.stage_run_unit_id=task_plan.stage_run_unit_id
             AND team_plan.scope_snapshot_id=task_plan.scope_snapshot_id
             AND team_plan.organization_id=task_plan.organization_id
            JOIN stage_work_items item
              ON item.id=dispatch.stage_work_item_id
             AND item.team_plan_id=task_plan.stage_team_plan_id
             AND item.operation_id=task_plan.operation_id
             AND item.stage_execution_id=task_plan.stage_execution_id
             AND item.stage_run_unit_id=task_plan.stage_run_unit_id
             AND item.scope_snapshot_id=task_plan.scope_snapshot_id
             AND item.organization_id=task_plan.organization_id
             AND item.terminal_at IS NOT NULL
            JOIN stage_worker_outputs output
              ON output.work_item_id=dispatch.stage_work_item_id
             AND output.team_plan_id=task_plan.stage_team_plan_id
             AND output.operation_id=task_plan.operation_id
             AND output.stage_execution_id=task_plan.stage_execution_id
             AND output.stage_run_unit_id=task_plan.stage_run_unit_id
             AND output.scope_snapshot_id=task_plan.scope_snapshot_id
             AND output.organization_id=task_plan.organization_id
            JOIN stage_worker_runs output_worker
              ON output_worker.id=output.worker_run_id
             AND output_worker.work_item_id=dispatch.stage_work_item_id
             AND output_worker.operation_id=task_plan.operation_id
             AND output_worker.stage_execution_id=task_plan.stage_execution_id
             AND output_worker.stage_run_unit_id=task_plan.stage_run_unit_id
             AND output_worker.organization_id=task_plan.organization_id
             AND output_worker.terminal_at IS NOT NULL
             AND output_worker.active_tool_call_id IS NULL
             AND (
                 (output_worker.status='passed' AND item.status='completed')
                 OR (
                     output_worker.status='failed'
                     AND item.status='exhausted'
                     AND output.business_disposition='blocked'
                 )
             )
           WHERE dispatch.task_plan_id=input_task_plan_id
             AND dispatch.actor_kind IN ('worker','nested_worker')
      ) durable;

    IF row_count<>expected_dispatch_count
       OR receipt_count<>expected_dispatch_count
       OR work_item_count<>expected_dispatch_count
       OR EXISTS(
            SELECT 1 FROM unnest(raw_output_hashes) output_hash(value)
             WHERE value IS NULL OR value !~ '^sha256:[0-9a-f]{64}$'
       )
    THEN
        RAISE EXCEPTION
            'INVESTIGATION_VERIFICATION_DURABLE_OUTPUT_MANIFEST_MISMATCH';
    END IF;

    SELECT COALESCE(array_agg(DISTINCT value ORDER BY value),ARRAY[]::TEXT[])
      INTO canonical_output_hashes
      FROM unnest(raw_output_hashes) output_hash(value);
    RETURN canonical_output_hashes;
END;
$$;

DO $$
DECLARE
    definition TEXT;
    old_fragment TEXT := $old_fragment$        SELECT COUNT(*),
               unified_investigation_exact_set_hash(
                   'investigation_verification_accepted_outputs.v1',
                   COALESCE(array_agg(latest.result_sha256 ORDER BY latest.result_sha256),
                            ARRAY[]::TEXT[])
               )
          INTO expected_accepted_output_count,expected_accepted_output_set_sha256
          FROM (
              SELECT DISTINCT dispatch_latest.result_sha256
                FROM (
                    SELECT DISTINCT ON(dispatch.dispatch_receipt_id)
                           dispatch.dispatch_receipt_id,attempt.result_sha256
                      FROM pentagi_logical_dispatch_receipts dispatch
                      JOIN pentagi_logical_dispatch_attempts attempt
                        ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                     WHERE dispatch.task_plan_id=NEW.task_plan_id
                       AND dispatch.actor_kind IN ('worker','nested_worker')
                     ORDER BY dispatch.dispatch_receipt_id,attempt.attempt_epoch DESC
                ) dispatch_latest
          ) latest;
$old_fragment$;
    new_fragment TEXT := $new_fragment$        SELECT cardinality(manifest.output_hashes),
               unified_investigation_exact_set_hash(
                   'investigation_verification_accepted_outputs.v1',
                   manifest.output_hashes
               )
          INTO expected_accepted_output_count,expected_accepted_output_set_sha256
          FROM (
              SELECT unified_investigation_verification_accepted_output_hashes(
                         NEW.task_plan_id
                     ) AS output_hashes
          ) manifest;
$new_fragment$;
BEGIN
    SELECT pg_get_functiondef(
               'enforce_investigation_verification_advisory_header'::REGPROC
           )
      INTO STRICT definition;
    IF strpos(definition,old_fragment)=0 THEN
        RAISE EXCEPTION
            'INVESTIGATION_VERIFICATION_ADVISORY_TRIGGER_SOURCE_DRIFT';
    END IF;
    definition := replace(definition,old_fragment,new_fragment);
    IF strpos(definition,old_fragment)<>0
       OR strpos(
            definition,
            'unified_investigation_verification_accepted_output_hashes'
          )=0
    THEN
        RAISE EXCEPTION
            'INVESTIGATION_VERIFICATION_ADVISORY_TRIGGER_REWRITE_FAILED';
    END IF;
    EXECUTE definition;
END;
$$;
