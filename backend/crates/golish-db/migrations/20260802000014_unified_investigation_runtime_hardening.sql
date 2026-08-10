-- Unified Investigation runtime hardening.
--
-- This forward-only patch keeps 00013 replayable while correcting the
-- production StageTeam identity mapping and the stop/closure denominator.
-- It also makes PentAGI task admission request-first: the automatic scheduler
-- writes one immutable request, then the sole TaskOrchestrator writer creates
-- the plan that consumes it.

-- Scheduler-owned task requests exist before a PentAGI plan.  Historical rows
-- retain task_plan_id; new rows bind through the plan's run_request_id.
ALTER TABLE pentagi_task_run_requests
    ALTER COLUMN task_plan_id DROP NOT NULL;

DO $$
DECLARE constraint_name TEXT;
BEGIN
    SELECT conname INTO constraint_name
      FROM pg_constraint
     WHERE conrelid='pentagi_task_run_requests'::regclass
       AND contype='f'
       AND conkey=ARRAY[
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='task_plan_id'),
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='authority_id'),
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='operation_id'),
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='stage_execution_id'),
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='stage_run_unit_id'),
           (SELECT attnum FROM pg_attribute WHERE attrelid='pentagi_task_run_requests'::regclass AND attname='organization_id')
       ]::SMALLINT[];
    IF constraint_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE pentagi_task_run_requests DROP CONSTRAINT %I',constraint_name);
    END IF;
END;
$$;

ALTER TABLE investigation_pentagi_task_plans
    ADD COLUMN run_request_id UUID;

UPDATE investigation_pentagi_task_plans plan
   SET run_request_id=request.run_request_id
  FROM pentagi_task_run_requests request
 WHERE request.task_plan_id=plan.task_plan_id;

ALTER TABLE investigation_pentagi_task_plans
    ADD CONSTRAINT investigation_pentagi_plan_run_request_unique UNIQUE(run_request_id),
    ADD CONSTRAINT investigation_pentagi_plan_run_request_fk
        FOREIGN KEY(run_request_id) REFERENCES pentagi_task_run_requests(run_request_id)
        ON DELETE RESTRICT;

CREATE FUNCTION unified_investigation_guard_plan_run_request_v2()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE request pentagi_task_run_requests%ROWTYPE;
BEGIN
    IF NEW.run_request_id IS NULL THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_RUN_REQUEST_REQUIRED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT request FROM pentagi_task_run_requests
     WHERE run_request_id=NEW.run_request_id FOR UPDATE;
    IF request.task_plan_id IS NOT NULL
       OR ROW(request.authority_id,request.operation_id,request.stage_execution_id,
              request.owning_stage_run_request_id,request.stage_run_unit_id,
              request.organization_id,request.subject_kind,request.subject_id,
              request.subject_fingerprint_sha256)
          IS DISTINCT FROM
          ROW(NEW.authority_id,NEW.operation_id,NEW.stage_execution_id,
              NEW.owning_stage_run_request_id,NEW.stage_run_unit_id,
              NEW.organization_id,NEW.subject_kind,NEW.subject_id,
              NEW.subject_fingerprint_sha256)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_RUN_REQUEST_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_task_plans_run_request_v2
BEFORE INSERT ON investigation_pentagi_task_plans
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_plan_run_request_v2();

CREATE OR REPLACE FUNCTION unified_investigation_guard_pentagi_plan_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF OLD.status<>'open' OR NEW.status<>'sealed' OR NEW.row_version<>OLD.row_version+1
       OR ROW(NEW.task_plan_id,NEW.stable_request_id,NEW.run_request_id,
              NEW.authority_id,NEW.stage_team_plan_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.owning_stage_run_request_id,
              NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
              NEW.subject_kind,NEW.subject_id,NEW.subject_fingerprint_sha256,
              NEW.task_plan_version,NEW.task_plan_sha256,NEW.allowed_role_catalog,
              NEW.cognitive_tool_envelope_sha256,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.task_plan_id,OLD.stable_request_id,OLD.run_request_id,
              OLD.authority_id,OLD.stage_team_plan_id,OLD.operation_id,
              OLD.stage_execution_id,OLD.owning_stage_run_request_id,
              OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
              OLD.subject_kind,OLD.subject_id,OLD.subject_fingerprint_sha256,
              OLD.task_plan_version,OLD.task_plan_sha256,OLD.allowed_role_catalog,
              OLD.cognitive_tool_envelope_sha256,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_subtasks.v1',
               COALESCE(array_agg(member_sha256 ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM investigation_pentagi_subtasks WHERE task_plan_id=NEW.task_plan_id;
    IF actual_count=0 OR NEW.subtask_count<>actual_count OR NEW.subtask_set_sha256<>actual_hash
       OR NOT EXISTS(
            SELECT 1 FROM investigation_pentagi_delegation_census_seals
             WHERE task_plan_id=NEW.task_plan_id
       )
       OR NOT EXISTS(
            SELECT 1 FROM pentagi_task_run_requests request
             WHERE request.run_request_id=NEW.run_request_id
               AND request.task_plan_id IS NULL
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_EXACT_SEAL_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

-- A StageTeam worker stores the parent dispatch tool request, not its own
-- transcript request.  Derive the actual child identity exactly as runtime
-- does instead of comparing unlike columns.
CREATE OR REPLACE FUNCTION unified_investigation_guard_dispatch_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan investigation_pentagi_task_plans%ROWTYPE;
    parent pentagi_logical_dispatch_receipts%ROWTYPE;
    worker stage_worker_runs%ROWTYPE;
    expected_transcript_request_id TEXT;
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
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_TOOL_IDENTITY_MISSING' USING ERRCODE='23514';
        END IF;
        expected_transcript_request_id := concat(
            worker.parent_request_id,'::worker:',worker.id::TEXT
        );
        IF worker.parent_request_id<>NEW.parent_dispatch_tool_request_id THEN
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_TOOL_IDENTITY_MISMATCH' USING ERRCODE='23514';
        END IF;
    END IF;
    IF expected_transcript_request_id<>NEW.transcript_request_id THEN
        RAISE EXCEPTION 'PENTAGI_TRANSCRIPT_WORKER_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF NEW.actor_kind<>'primary' THEN
        SELECT * INTO STRICT parent FROM pentagi_logical_dispatch_receipts
         WHERE dispatch_receipt_id=NEW.parent_dispatch_receipt_id FOR SHARE;
        IF parent.task_plan_id<>NEW.task_plan_id
           OR parent.transcript_request_id<>NEW.parent_actor_transcript_request_id
        THEN
            RAISE EXCEPTION 'PENTAGI_PARENT_DISPATCH_IDENTITY_MISMATCH' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

-- Stop freezes only work that still needs draining.  Work that was already
-- terminal before the stop is part of the closure census but is not a drain
-- member and must not be misclassified as late admission.
CREATE OR REPLACE FUNCTION seal_investigation_run_closure_v1(
    p_closure_id UUID,
    p_stable_request_id UUID,
    p_authority_id UUID,
    p_expected_run_head_sha256 TEXT,
    p_disposition TEXT,
    p_residual_set_sha256 TEXT
)
RETURNS investigation_run_closures
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_run_closures%ROWTYPE;
    head investigation_run_heads%ROWTYPE;
    stop investigation_stop_intents%ROWTYPE;
    result investigation_run_closures%ROWTYPE;
    work_count BIGINT;
    work_hash TEXT;
    plan_count BIGINT;
    plan_hash TEXT;
    dispatch_count BIGINT;
    dispatch_hash TEXT;
    next_version BIGINT;
    next_change_seq BIGINT;
    event_id UUID;
    event_hash TEXT;
    closure_hash TEXT;
BEGIN
    SELECT * INTO existing FROM investigation_run_closures
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.closure_id,existing.authority_id,existing.disposition,
               existing.residual_set_sha256)
           IS DISTINCT FROM
           ROW(p_closure_id,p_authority_id,p_disposition,p_residual_set_sha256)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=p_authority_id FOR UPDATE;
    IF head.run_state NOT IN ('stop_pending','draining') OR head.admission_open
       OR head.head_sha256<>p_expected_run_head_sha256
       OR p_disposition NOT IN ('pass','pass_with_gaps','stopped')
       OR p_residual_set_sha256 !~ '^sha256:[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT stop FROM investigation_stop_intents
     WHERE authority_id=p_authority_id AND stop_epoch=head.stop_epoch FOR SHARE;
    IF EXISTS(
        SELECT 1 FROM investigation_stop_work_members member
        JOIN investigation_run_work_items work ON work.work_id=member.work_id
        WHERE member.stop_intent_id=stop.stop_intent_id
          AND NOT unified_investigation_work_state_terminal(work.current_state)
    ) OR EXISTS(
        SELECT 1 FROM investigation_run_work_items work
         WHERE work.authority_id=p_authority_id
           AND NOT unified_investigation_work_state_terminal(work.current_state)
           AND NOT EXISTS(
               SELECT 1 FROM investigation_stop_work_members member
                WHERE member.stop_intent_id=stop.stop_intent_id
                  AND member.work_id=work.work_id
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_WORK_NOT_DRAINED' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM investigation_pentagi_task_plans plan
         WHERE plan.authority_id=p_authority_id AND plan.status<>'sealed'
    ) OR EXISTS(
        SELECT 1 FROM investigation_pentagi_task_plans plan
         WHERE plan.authority_id=p_authority_id
           AND NOT EXISTS(
               SELECT 1 FROM investigation_pentagi_delegation_census_seals census
                WHERE census.task_plan_id=plan.task_plan_id
           )
    ) OR EXISTS(
        SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
        JOIN pentagi_logical_dispatch_receipts dispatch
          ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
        JOIN investigation_pentagi_task_plans plan
          ON plan.task_plan_id=dispatch.task_plan_id
        WHERE plan.authority_id=p_authority_id AND attempt.outcome='unknown_held'
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_DELEGATION_NOT_CLOSED' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
        JOIN hypothesis_verification_task_state_heads task_head ON task_head.task_id=task.task_id
        WHERE task.stage_execution_id=head.stage_execution_id
          AND task.operation_id=head.operation_id
          AND task_head.current_state NOT IN ('cancelled','blocked','recovery_required','terminal')
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_TASK_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM investigation_fuel_budgets budget
        JOIN investigation_fuel_budget_heads fuel ON fuel.budget_id=budget.budget_id
        WHERE budget.authority_id=p_authority_id
          AND (fuel.reserved_amount<>0 OR fuel.unknown_held_amount<>0)
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_FUEL_NOT_SETTLED' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_run_work_items.v1',
               COALESCE(array_agg(stable_work_key_sha256 ORDER BY work_kind,work_id),ARRAY[]::TEXT[])
           ) INTO work_count,work_hash
      FROM investigation_run_work_items WHERE authority_id=p_authority_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_task_plans.v1',
               COALESCE(array_agg(task_plan_sha256 ORDER BY task_plan_id),ARRAY[]::TEXT[])
           ) INTO plan_count,plan_hash
      FROM investigation_pentagi_task_plans WHERE authority_id=p_authority_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'pentagi_logical_dispatch_receipts.v1',
               COALESCE(array_agg(dispatch.receipt_sha256 ORDER BY dispatch.dispatch_receipt_id),ARRAY[]::TEXT[])
           ) INTO dispatch_count,dispatch_hash
      FROM pentagi_logical_dispatch_receipts dispatch
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
     WHERE plan.authority_id=p_authority_id;
    closure_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_run_closure.v1',p_closure_id::TEXT,
            p_authority_id::TEXT,head.stop_epoch::TEXT,p_disposition,work_count::TEXT,
            work_hash,plan_count::TEXT,plan_hash,dispatch_count::TEXT,dispatch_hash,
            p_residual_set_sha256),
        'UTF8'),'sha256'),'hex');
    INSERT INTO investigation_run_closures(
        closure_id,stable_request_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stop_intent_id,stop_epoch,disposition,
        work_count,work_set_sha256,task_plan_count,task_plan_set_sha256,
        dispatch_count,dispatch_set_sha256,residual_set_sha256,closure_sha256
    ) VALUES(
        p_closure_id,p_stable_request_id,p_authority_id,head.operation_id,
        head.stage_execution_id,head.owning_stage_run_request_id,stop.stop_intent_id,
        head.stop_epoch,p_disposition,work_count,work_hash,plan_count,plan_hash,
        dispatch_count,dispatch_hash,p_residual_set_sha256,closure_hash
    ) RETURNING * INTO result;
    next_version := head.head_version+1;
    next_change_seq := head.change_seq+1;
    event_id := gen_random_uuid();
    event_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_run_event.v1',event_id::TEXT,
            head.head_sha256,'closed',head.stop_epoch::TEXT,next_change_seq::TEXT),
        'UTF8'),'sha256'),'hex');
    INSERT INTO investigation_run_state_events(
        event_id,stable_request_id,authority_id,event_ordinal,expected_head_sha256,
        from_state,to_state,stop_epoch,change_seq,event_sha256
    ) VALUES(
        event_id,p_stable_request_id,p_authority_id,next_version,head.head_sha256,
        head.run_state,'closed',head.stop_epoch,next_change_seq,event_hash
    );
    PERFORM set_config('golish.investigation_run_head_write','on',TRUE);
    UPDATE investigation_run_heads
       SET run_state='closed',admission_open=FALSE,change_seq=next_change_seq,
           head_version=next_version,latest_event_id=event_id,
           head_sha256=unified_investigation_runtime_head_sha256(
               authority_id,'closed',FALSE,stop_epoch,next_change_seq,next_version
           ),updated_at=statement_timestamp()
     WHERE authority_id=p_authority_id;
    PERFORM set_config('golish.investigation_run_head_write','off',TRUE);
    RETURN result;
END;
$$;
