-- Permit only the normal completed Analysis Primary to replay a sealed typed
-- synthesis after the compiler failed before committing a canonical decision.
-- Ordinary Blocked work and every partial compiler artifact remain terminal.

CREATE FUNCTION unified_investigation_primary_post_synthesis_analysis_rearm_allowed(
    work investigation_run_work_items,
    rearm investigation_run_work_state_events
)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
STRICT
AS $$
    SELECT work.work_kind='analysis'
       AND work.current_state='blocked'
       AND rearm.from_state='blocked'
       AND rearm.to_state='running'
       AND split_part(rearm.reason_code,'|',1)=
           'post_synthesis_analysis_primary_recovery.v1'
       AND split_part(rearm.reason_code,'|',2) ~ '^sha256:[0-9a-f]{64}$'
       AND split_part(rearm.reason_code,'|',3) ~ '^sha256:[0-9a-f]{64}$'
       AND split_part(rearm.reason_code,'|',4)=''
       AND EXISTS(
           SELECT 1
             FROM investigation_run_work_state_events blocked
             JOIN investigation_analysis_attempt_bindings binding
               ON binding.work_id=work.work_id
              AND binding.authority_id=work.authority_id
             JOIN investigation_pentagi_task_plans task_plan
               ON task_plan.authority_id=work.authority_id
              AND task_plan.operation_id=work.operation_id
              AND task_plan.stage_execution_id=work.stage_execution_id
              AND task_plan.stage_run_unit_id=work.stage_run_unit_id
              AND task_plan.organization_id=work.organization_id
              AND task_plan.subject_kind='analysis_attempt'
              AND task_plan.subject_id=binding.analysis_attempt_id
              AND task_plan.status='sealed'
             JOIN investigation_refiner_plan_ledger_seals refiner_seal
               ON refiner_seal.task_plan_id=task_plan.task_plan_id
             JOIN investigation_pentagi_delegation_census_seals census
               ON census.task_plan_id=task_plan.task_plan_id
             JOIN investigation_pentagi_pipeline_events synthesis
               ON synthesis.task_plan_id=task_plan.task_plan_id
              AND synthesis.event_kind='primary_synthesis'
              AND synthesis.actor_worker_run_id=census.primary_worker_run_id
              AND synthesis.parent_dispatch_receipt_id=
                  census.primary_dispatch_receipt_id
              AND synthesis.event_sha256=split_part(rearm.reason_code,'|',2)
             JOIN pentagi_logical_dispatch_receipts dispatch
               ON dispatch.dispatch_receipt_id=census.primary_dispatch_receipt_id
              AND dispatch.task_plan_id=task_plan.task_plan_id
              AND dispatch.actor_kind='primary'
              AND dispatch.subtask_id IS NULL
              AND dispatch.worker_run_id=census.primary_worker_run_id
             JOIN stage_work_items primary_item
               ON primary_item.id=dispatch.stage_work_item_id
              AND primary_item.team_plan_id=task_plan.stage_team_plan_id
              AND primary_item.stable_key='leader:primary'
              AND primary_item.kind='investigation_primary'
              AND primary_item.role=(
                  SELECT leader_role FROM stage_team_plans
                   WHERE id=task_plan.stage_team_plan_id
              )
              AND primary_item.created_by='server_seed'
              AND primary_item.required_for_barrier=FALSE
              AND primary_item.status='completed'
              AND primary_item.terminal_at IS NOT NULL
             JOIN stage_worker_runs primary_worker
               ON primary_worker.id=census.primary_worker_run_id
              AND primary_worker.work_item_id=primary_item.id
              AND primary_worker.status='passed'
              AND primary_worker.terminal_at IS NOT NULL
              AND primary_worker.lease_token IS NULL
              AND primary_worker.active_tool_call_id IS NULL
              AND jsonb_typeof(primary_worker.checkpoint)='array'
              AND tool_truth_sha256(primary_worker.checkpoint::TEXT)=
                  split_part(rearm.reason_code,'|',3)
            WHERE blocked.event_id=work.latest_event_id
              AND blocked.work_id=work.work_id
              AND blocked.to_state='blocked'
              AND blocked.reason_code IN (
                  'investigation_analysis_host_infrastructure',
                  'investigation_analysis_host_authority_mismatch'
              )
              AND NOT EXISTS(
                  SELECT 1
                    FROM investigation_hypothesis_compilation_decisions decision
                   WHERE decision.binding_id=binding.binding_id
                      OR decision.task_plan_id=task_plan.task_plan_id
              )
              AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                    WHERE all_worker.work_item_id=primary_item.id)=1
              AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                    WHERE event.task_plan_id=task_plan.task_plan_id
                      AND event.event_kind='primary_synthesis')=1
              AND (SELECT COUNT(*) FROM jsonb_path_query(
                    primary_worker.checkpoint,
                    'strict $.** ? (@.name == "submit_result")'))=1
              AND jsonb_path_exists(
                    primary_worker.checkpoint,
                    'strict $.** ? (@.name == "submit_result" && @.arguments.result.schema_version == 1)'
                  )
       )
$$;

CREATE OR REPLACE FUNCTION unified_investigation_apply_work_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    work investigation_run_work_items%ROWTYPE;
    run investigation_run_heads%ROWTYPE;
    transition_allowed BOOLEAN;
BEGIN
    SELECT * INTO STRICT work FROM investigation_run_work_items
     WHERE work_id=NEW.work_id FOR UPDATE;
    SELECT * INTO STRICT run FROM investigation_run_heads
     WHERE authority_id=work.authority_id FOR SHARE;
    transition_allowed :=
        unified_investigation_work_transition_allowed(work.current_state,NEW.to_state)
        OR unified_investigation_post_synthesis_analysis_rearm_allowed(work,NEW)
        OR unified_investigation_primary_post_synthesis_analysis_rearm_allowed(work,NEW);
    IF NEW.expected_head_version<>work.head_version
       OR NEW.event_ordinal<>work.head_version+1
       OR NEW.from_state<>work.current_state
       OR NOT transition_allowed
       OR NEW.observed_stop_epoch<>run.stop_epoch
    THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_STATE_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    IF NOT run.admission_open
       AND NEW.to_state IN ('queued','running','waiting_authorization')
    THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_REACTIVATION_AFTER_STOP' USING ERRCODE='23514';
    END IF;
    PERFORM set_config('golish.investigation_work_event_apply','on',TRUE);
    UPDATE investigation_run_work_items
       SET current_state=NEW.to_state,observed_stop_epoch=NEW.observed_stop_epoch,
           head_version=NEW.event_ordinal,latest_event_id=NEW.event_id,
           updated_at=statement_timestamp()
     WHERE work_id=NEW.work_id;
    PERFORM set_config('golish.investigation_work_event_apply','off',TRUE);
    RETURN NEW;
END;
$$;
