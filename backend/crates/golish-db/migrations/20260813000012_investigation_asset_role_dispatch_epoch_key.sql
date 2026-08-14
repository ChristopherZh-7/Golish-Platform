-- The durable Analysis subtask label is epoch-neutral so the same Asset
-- Primary can reuse its plan lineage. The server-seeded WorkItem key carries
-- the evolution epoch. Bind those two exact identities instead of comparing
-- them byte-for-byte.

CREATE OR REPLACE FUNCTION unified_investigation_guard_dispatch_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan investigation_pentagi_task_plans%ROWTYPE;
    parent pentagi_logical_dispatch_receipts%ROWTYPE;
    worker stage_worker_runs%ROWTYPE;
    expected_transcript_request_id TEXT;
    exact_asset_role_dispatch BOOLEAN := FALSE;
BEGIN
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    SELECT * INTO STRICT worker FROM stage_worker_runs
     WHERE id=NEW.worker_run_id FOR SHARE;
    IF NEW.actor_kind='primary' THEN
        expected_transcript_request_id := concat(
            plan.owning_stage_run_request_id,'::team:',plan.organization_id::TEXT,
            '::lead:',worker.id::TEXT
        );
    ELSE
        IF worker.parent_request_id IS NULL OR btrim(worker.parent_request_id)='' THEN
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_TOOL_IDENTITY_MISSING'
                USING ERRCODE='23514';
        END IF;
        expected_transcript_request_id := concat(
            worker.parent_request_id,'::worker:',worker.id::TEXT
        );
        IF worker.parent_request_id<>NEW.parent_dispatch_tool_request_id THEN
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_TOOL_IDENTITY_MISMATCH'
                USING ERRCODE='23514';
        END IF;
    END IF;
    IF expected_transcript_request_id<>NEW.transcript_request_id THEN
        RAISE EXCEPTION 'PENTAGI_TRANSCRIPT_WORKER_IDENTITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    IF NEW.actor_kind<>'primary' THEN
        SELECT * INTO STRICT parent FROM pentagi_logical_dispatch_receipts
         WHERE dispatch_receipt_id=NEW.parent_dispatch_receipt_id FOR SHARE;
        IF parent.task_plan_id<>NEW.task_plan_id
           OR parent.transcript_request_id<>NEW.parent_actor_transcript_request_id
        THEN
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_IDENTITY_MISMATCH'
                USING ERRCODE='23514';
        END IF;
    END IF;

    IF NEW.actor_kind='worker' AND NEW.stage_worker_request_id IS NULL THEN
        SELECT EXISTS(
            SELECT 1
              FROM investigation_asset_primary_schedules schedule
              JOIN stage_work_items item
                ON item.id=NEW.stage_work_item_id
               AND item.team_plan_id=schedule.stage_team_plan_id
               AND item.dispatch_epoch=schedule.resume_dispatch_epoch
              JOIN investigation_pentagi_subtasks subtask
                ON subtask.subtask_id=NEW.subtask_id
               AND subtask.task_plan_id=NEW.task_plan_id
             WHERE schedule.status='applied'
               AND schedule.stage_team_plan_id=plan.stage_team_plan_id
               AND schedule.operation_id=NEW.operation_id
               AND schedule.stage_execution_id=NEW.stage_execution_id
               AND schedule.stage_run_unit_id=NEW.stage_run_unit_id
               AND schedule.scope_snapshot_id=NEW.scope_snapshot_id
               AND schedule.organization_id=NEW.organization_id
               AND item.id=CASE subtask.subtask_ordinal
                    WHEN 0 THEN schedule.browser_work_item_id
                    WHEN 1 THEN schedule.researcher_work_item_id
                    WHEN 2 THEN schedule.pentester_work_item_id
                    WHEN 3 THEN schedule.adviser_work_item_id
                    ELSE NULL END
               AND item.kind='investigation_asset_role'
               AND item.created_by='server_phase_transition'
               AND item.required_for_barrier
               AND item.role=CASE subtask.subtask_ordinal
                    WHEN 0 THEN 'browser'
                    WHEN 1 THEN 'researcher'
                    WHEN 2 THEN 'pentester'
                    WHEN 3 THEN 'adviser'
                    ELSE NULL END
               AND item.stable_key=concat(
                    subtask.label,':',schedule.evolution_epoch::TEXT
               )
               AND item.output_schema=subtask.expected_output_schema
               AND subtask.runnable
               AND worker.work_item_id=item.id
               AND worker.status='running'
               AND worker.lease_token IS NOT NULL
               AND worker.lease_expires_at>statement_timestamp()
               AND worker.active_tool_call_id IS NULL
        ) INTO exact_asset_role_dispatch;
        IF NOT exact_asset_role_dispatch THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_ROLE_LOGICAL_DISPATCH_AUTHORITY_MISMATCH'
                USING ERRCODE='23514';
        END IF;
    ELSIF NEW.actor_kind<>'primary' AND NEW.stage_worker_request_id IS NULL THEN
        RAISE EXCEPTION 'PENTAGI_WORKER_REQUEST_IDENTITY_MISSING'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
