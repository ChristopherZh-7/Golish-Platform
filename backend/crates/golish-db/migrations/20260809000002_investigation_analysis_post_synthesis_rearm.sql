-- Permit exactly one append-only Analysis work rearm after the historical
-- authority-mismatch crash window: the typed Primary synthesis is already in
-- the current deterministic v2 recovery Worker's checkpoint and its canonical
-- pipeline event/task census are sealed, but compilation did not persist.

CREATE FUNCTION unified_investigation_post_synthesis_analysis_rearm_allowed(
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
           'post_synthesis_analysis_recovery.v1'
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
             JOIN stage_worker_runs source_worker
               ON source_worker.id=census.primary_worker_run_id
              AND source_worker.status='failed'
              AND source_worker.terminal_at IS NOT NULL
              AND source_worker.lease_token IS NULL
              AND source_worker.active_tool_call_id IS NULL
             JOIN stage_work_items source_item
               ON source_item.id=source_worker.work_item_id
              AND source_item.team_plan_id=task_plan.stage_team_plan_id
              AND source_item.stable_key='leader:primary'
              AND source_item.status='exhausted'
             JOIN stage_worker_outputs source_output
               ON source_output.team_plan_id=source_item.team_plan_id
              AND source_output.work_item_id=source_item.id
              AND source_output.worker_run_id=source_worker.id
              AND source_output.business_disposition='blocked'
              AND source_output.canonical_output->>'kind'=
                  'stage_team_attempts_exhausted'
              AND source_output.canonical_output->>'failure_code'=
                  'stage_team_worker_lease_expired'
              AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                  ANY(source_output.blocker_codes)
             JOIN stage_work_items recovery_v1
               ON recovery_v1.id=uuid_generate_v5(
                      source_item.id,
                      'sealed-investigation-synthesis-recovery-primary-v1'
                  )
              AND recovery_v1.team_plan_id=source_item.team_plan_id
              AND recovery_v1.stable_key=
                  'leader:synthesis-recovery:' || source_item.id::TEXT
              AND recovery_v1.kind=source_item.kind
              AND recovery_v1.status='exhausted'
              AND recovery_v1.terminal_at IS NOT NULL
             JOIN stage_worker_runs recovery_v1_worker
               ON recovery_v1_worker.work_item_id=recovery_v1.id
              AND recovery_v1_worker.status='failed'
              AND recovery_v1_worker.terminal_at IS NOT NULL
              AND recovery_v1_worker.lease_token IS NULL
              AND recovery_v1_worker.active_tool_call_id IS NULL
              AND recovery_v1_worker.checkpoint #>>
                  '{stage_team_execution_failure,code}'=
                  'stage_team_worker_lease_expired'
             JOIN stage_worker_outputs recovery_v1_output
               ON recovery_v1_output.team_plan_id=source_item.team_plan_id
              AND recovery_v1_output.work_item_id=recovery_v1.id
              AND recovery_v1_output.worker_run_id=recovery_v1_worker.id
              AND recovery_v1_output.business_disposition='blocked'
              AND recovery_v1_output.canonical_output->>'kind'=
                  'stage_team_attempts_exhausted'
              AND recovery_v1_output.canonical_output->>'failure_code'=
                  'stage_team_worker_lease_expired'
              AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                  ANY(recovery_v1_output.blocker_codes)
             JOIN stage_work_items recovery_v2
               ON recovery_v2.id=uuid_generate_v5(
                      recovery_v1.id,
                      'sealed-investigation-synthesis-recovery-primary-v2'
                  )
              AND recovery_v2.team_plan_id=source_item.team_plan_id
              AND recovery_v2.stable_key=recovery_v1.stable_key
              AND recovery_v2.kind='investigation_primary_recovery'
              AND recovery_v2.status='running'
              AND recovery_v2.terminal_at IS NULL
             JOIN stage_worker_runs recovery_worker
               ON recovery_worker.work_item_id=recovery_v2.id
              AND recovery_worker.status='running'
              AND recovery_worker.terminal_at IS NULL
              AND recovery_worker.lease_token IS NOT NULL
              AND recovery_worker.active_tool_call_id IS NULL
              AND jsonb_typeof(recovery_worker.checkpoint)='array'
              AND tool_truth_sha256(recovery_worker.checkpoint::TEXT)=
                  split_part(rearm.reason_code,'|',3)
            WHERE blocked.event_id=work.latest_event_id
              AND blocked.work_id=work.work_id
              AND blocked.to_state='blocked'
              AND blocked.reason_code=
                  'investigation_analysis_host_authority_mismatch'
              AND NOT EXISTS(
                  SELECT 1
                    FROM investigation_hypothesis_compilation_decisions decision
                   WHERE decision.binding_id=binding.binding_id
                      OR decision.task_plan_id=task_plan.task_plan_id
              )
              AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                    WHERE event.task_plan_id=task_plan.task_plan_id
                      AND event.event_kind='primary_synthesis')=1
              AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                    WHERE all_worker.work_item_id=recovery_v2.id)=1
              AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                    WHERE all_worker.work_item_id=recovery_v1.id)=1
              AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                    WHERE all_output.work_item_id=recovery_v1.id)=1
              AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                    WHERE all_output.work_item_id=source_item.id)=1
              AND (SELECT COUNT(*)
                     FROM jsonb_path_query(
                         recovery_worker.checkpoint,
                         'strict $.** ? (@.name == "submit_result")'
                     ))=1
              AND jsonb_path_exists(
                    recovery_worker.checkpoint,
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
        OR unified_investigation_post_synthesis_analysis_rearm_allowed(work,NEW);
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
