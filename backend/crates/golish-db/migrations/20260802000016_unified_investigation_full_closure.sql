-- Upgrade the unified Investigation close receipt from the narrow stop/work
-- checksum to the complete `golish_core::InvestigationRunClosureV1` authority.
-- Every census below is recomputed from exact stage-owned rows while the run
-- head is locked. The caller supplies identity/CAS only.

CREATE TABLE investigation_stage_fixed_point_receipts (
    fixed_point_receipt_id UUID PRIMARY KEY,
    authority_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    source_receipt_count BIGINT NOT NULL CHECK(source_receipt_count>0),
    source_receipt_set_sha256 TEXT NOT NULL CHECK(source_receipt_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_sha256 TEXT NOT NULL CHECK(residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(fixed_point_receipt_id,authority_id),
    UNIQUE(fixed_point_receipt_id,authority_id,receipt_sha256),
    FOREIGN KEY(authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,scope_snapshot_id)
        REFERENCES investigation_run_heads(
            authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,scope_snapshot_id
        ) ON DELETE RESTRICT
);

ALTER TABLE investigation_run_closures
    ADD CONSTRAINT investigation_run_closures_id_authority_unique
    UNIQUE(closure_id,authority_id);

CREATE TRIGGER investigation_stage_fixed_point_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_stage_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- The stop receipt freezes every durable open-work authority, not only the
-- registration ledger.  `source_kind` keeps independently fenced authorities
-- distinct when one logical unit has both a runtime registration and a domain
-- row.  The public stop receipt count/hash is derived from this exact set.
CREATE TABLE investigation_stop_denominator_members (
    stop_denominator_member_id UUID PRIMARY KEY,
    stop_intent_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    stop_epoch BIGINT NOT NULL CHECK(stop_epoch>0),
    work_class TEXT NOT NULL CHECK(work_class IN(
        'analysis','read_session','query','enrichment','outbox',
        'verification_task','pentagi_plan','pentagi_subtask','stage_work_item',
        'worker_request','worker_run','campaign','prepared_action',
        'action_execution','fact_delta','consolidation'
    )),
    source_kind TEXT NOT NULL CHECK(btrim(source_kind)<>''),
    source_id UUID NOT NULL,
    source_identity_sha256 TEXT NOT NULL
        CHECK(source_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    frozen_state TEXT NOT NULL CHECK(btrim(frozen_state)<>''),
    frozen_head_version BIGINT NOT NULL CHECK(frozen_head_version>=0),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(stop_intent_id,source_kind,source_id),
    UNIQUE(stop_intent_id,member_sha256),
    FOREIGN KEY(stop_intent_id,authority_id,stop_epoch)
        REFERENCES investigation_stop_intents(stop_intent_id,authority_id,stop_epoch)
        ON DELETE RESTRICT
);

CREATE TRIGGER investigation_stop_denominator_members_append_only
BEFORE UPDATE OR DELETE ON investigation_stop_denominator_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- Forward fence for domain rows that historically did not carry the unified
-- authority id. New admission/work may only start while the stage head is
-- running; drain-time terminal receipts may still be written until `closed`.
CREATE FUNCTION investigation_assert_stage_accepts_new_work(
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_scope_snapshot_id UUID
)
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    head investigation_run_heads%ROWTYPE;
BEGIN
    SELECT candidate.* INTO head
      FROM investigation_run_heads candidate
     WHERE candidate.operation_id=p_operation_id
       AND candidate.stage_execution_id=p_stage_execution_id
       AND candidate.scope_snapshot_id=p_scope_snapshot_id
     FOR SHARE;
    IF FOUND AND (head.run_state<>'running' OR NOT head.admission_open) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_LATE_WORK_REJECTED' USING ERRCODE='23514';
    END IF;
END;
$$;

CREATE FUNCTION investigation_assert_stage_not_closed(
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_scope_snapshot_id UUID
)
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    head investigation_run_heads%ROWTYPE;
BEGIN
    SELECT candidate.* INTO head
      FROM investigation_run_heads candidate
     WHERE candidate.operation_id=p_operation_id
       AND candidate.stage_execution_id=p_stage_execution_id
       AND candidate.scope_snapshot_id=p_scope_snapshot_id
     FOR SHARE;
    IF FOUND AND head.run_state='closed' THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_LATE_TERMINAL_RECEIPT_REJECTED' USING ERRCODE='23514';
    END IF;
END;
$$;

CREATE FUNCTION investigation_guard_late_admission_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM investigation_assert_stage_accepts_new_work(
        NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id
    );
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_admission_sets_closure_fence
BEFORE INSERT ON verification_admission_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_admission_set();

CREATE FUNCTION investigation_guard_late_verification_task()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM investigation_assert_stage_accepts_new_work(
        NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id
    );
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_tasks_closure_fence
BEFORE INSERT ON hypothesis_verification_tasks
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_verification_task();

CREATE FUNCTION investigation_guard_late_task_campaign()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.task_id FOR SHARE;
    PERFORM investigation_assert_stage_accepts_new_work(
        task.operation_id,task.stage_execution_id,task.scope_snapshot_id
    );
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_campaigns_closure_fence
BEFORE INSERT ON hypothesis_verification_task_campaigns
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_task_campaign();

CREATE FUNCTION investigation_guard_late_actual_campaign()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM hypothesis_verification_task_campaigns reservation
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE reservation.campaign_id=NEW.campaign_id FOR SHARE OF task_row;
    IF FOUND THEN
        IF task.operation_id<>NEW.operation_id
           OR task.organization_id<>NEW.organization_id
           OR task.hypothesis_revision_id<>NEW.hypothesis_revision_id
           OR task.verification_plan_id<>NEW.verification_plan_id
        THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_CAMPAIGN_AUTHORITY_MISMATCH' USING ERRCODE='23514';
        END IF;
        PERFORM investigation_assert_stage_accepts_new_work(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_campaigns_closure_fence
BEFORE INSERT ON verification_campaigns
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_actual_campaign();

CREATE FUNCTION investigation_guard_late_campaign_child()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM hypothesis_verification_task_campaigns reservation
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE reservation.campaign_id=NEW.campaign_id FOR SHARE OF task_row;
    IF FOUND THEN
        IF TG_OP='INSERT'
           OR (TG_TABLE_NAME='verification_prepared_actions'
               AND NEW.state IN('authorized','started'))
        THEN
            PERFORM investigation_assert_stage_accepts_new_work(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        ELSE
            PERFORM investigation_assert_stage_not_closed(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_prepared_actions_closure_fence
BEFORE INSERT OR UPDATE ON verification_prepared_actions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_campaign_child();

CREATE TRIGGER verification_fact_delta_bundles_closure_fence
BEFORE INSERT ON verification_fact_delta_bundles
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_campaign_child();

CREATE FUNCTION investigation_guard_late_action_execution()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM verification_prepared_actions action
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=action.campaign_id
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE action.prepared_action_id=NEW.prepared_action_id FOR SHARE OF task_row;
    IF FOUND THEN
        PERFORM investigation_assert_stage_accepts_new_work(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_action_executions_closure_fence
BEFORE INSERT ON verification_action_executions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_late_action_execution();

-- Every domain writer takes the same run-head share lock as the generic work
-- ledger.  Stop owns the conflicting row lock, so the frozen census and the
-- closed admission fence become visible atomically.
CREATE FUNCTION investigation_guard_read_session_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    session_row investigation_main_read_sessions%ROWTYPE;
BEGIN
    IF TG_TABLE_NAME='investigation_main_read_sessions' THEN
        PERFORM investigation_assert_stage_accepts_new_work(
            NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id
        );
    ELSE
        SELECT * INTO STRICT session_row FROM investigation_main_read_sessions
         WHERE main_read_session_id=NEW.main_read_session_id FOR SHARE;
        PERFORM investigation_assert_stage_not_closed(
            session_row.operation_id,session_row.stage_execution_id,session_row.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_main_read_sessions_stop_fence
BEFORE INSERT ON investigation_main_read_sessions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_read_session_writer();
CREATE TRIGGER investigation_main_read_session_receipts_stop_fence
BEFORE INSERT ON investigation_main_read_session_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_read_session_writer();

CREATE FUNCTION investigation_guard_pentagi_drain_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan investigation_pentagi_task_plans%ROWTYPE;
    dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
BEGIN
    IF TG_TABLE_NAME='pentagi_task_run_requests' THEN
        PERFORM investigation_assert_stage_accepts_new_work(
            NEW.operation_id,NEW.stage_execution_id,
            (SELECT scope_snapshot_id FROM investigation_run_heads
              WHERE authority_id=NEW.authority_id)
        );
    ELSE
        IF TG_TABLE_NAME='pentagi_logical_dispatch_attempts' THEN
            SELECT * INTO STRICT dispatch FROM pentagi_logical_dispatch_receipts
             WHERE dispatch_receipt_id=NEW.dispatch_receipt_id FOR SHARE;
            SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
             WHERE task_plan_id=dispatch.task_plan_id FOR SHARE;
        ELSE
            SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
             WHERE task_plan_id=NEW.task_plan_id FOR SHARE;
        END IF;
        PERFORM investigation_assert_stage_not_closed(
            plan.operation_id,plan.stage_execution_id,plan.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_run_requests_stop_fence
BEFORE INSERT ON pentagi_task_run_requests
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pentagi_drain_writer();
CREATE TRIGGER investigation_pentagi_dispatch_attempts_stop_fence
BEFORE INSERT ON pentagi_logical_dispatch_attempts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pentagi_drain_writer();
CREATE TRIGGER investigation_pentagi_census_stop_fence
BEFORE INSERT ON investigation_pentagi_delegation_census_seals
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pentagi_drain_writer();
CREATE TRIGGER investigation_pentagi_plan_seal_stop_fence
BEFORE UPDATE ON investigation_pentagi_task_plans
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pentagi_drain_writer();

CREATE FUNCTION investigation_guard_stage_team_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    unit stage_run_units%ROWTYPE;
BEGIN
    SELECT * INTO unit FROM stage_run_units
     WHERE id=NEW.stage_run_unit_id
       AND operation_id=NEW.operation_id
       AND stage_execution_id=NEW.stage_execution_id
       AND organization_id=NEW.organization_id
     FOR SHARE;
    IF FOUND AND unit.stage_kind='investigation' THEN
        IF TG_OP='INSERT'
           OR (TG_TABLE_NAME='stage_work_items' AND NEW.status IN(
               'queued','claimed','running','waiting_dependency','retry_pending'
           ))
           OR (TG_TABLE_NAME='stage_worker_runs' AND NEW.status IN(
               'queued','running','waiting_background','gate_blocked'
           ))
        THEN
            PERFORM investigation_assert_stage_accepts_new_work(
                NEW.operation_id,NEW.stage_execution_id,unit.scope_snapshot_id
            );
        ELSE
            PERFORM investigation_assert_stage_not_closed(
                NEW.operation_id,NEW.stage_execution_id,unit.scope_snapshot_id
            );
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_stage_work_items_stop_fence
BEFORE INSERT OR UPDATE ON stage_work_items
FOR EACH ROW EXECUTE FUNCTION investigation_guard_stage_team_writer();
CREATE TRIGGER investigation_stage_worker_requests_stop_fence
BEFORE INSERT ON stage_worker_requests
FOR EACH ROW EXECUTE FUNCTION investigation_guard_stage_team_writer();
CREATE TRIGGER investigation_stage_worker_runs_stop_fence
BEFORE INSERT OR UPDATE ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION investigation_guard_stage_team_writer();

CREATE FUNCTION investigation_guard_task_state_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.task_id FOR SHARE;
    IF NEW.to_state IN(
        'admitted','queued','planning','running','awaiting_authorization','consolidating'
    ) THEN
        PERFORM investigation_assert_stage_accepts_new_work(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    ELSE
        PERFORM investigation_assert_stage_not_closed(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_task_state_events_stop_fence
BEFORE INSERT ON hypothesis_verification_task_state_events
FOR EACH ROW EXECUTE FUNCTION investigation_guard_task_state_writer();

CREATE FUNCTION investigation_guard_campaign_state_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM hypothesis_verification_task_campaigns reservation
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE reservation.campaign_id=NEW.campaign_id FOR SHARE OF task_row;
    IF FOUND THEN
        IF NEW.state IN('admitted','running') THEN
            PERFORM investigation_assert_stage_accepts_new_work(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        ELSE
            PERFORM investigation_assert_stage_not_closed(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_campaign_updates_stop_fence
BEFORE UPDATE ON verification_campaigns
FOR EACH ROW EXECUTE FUNCTION investigation_guard_campaign_state_writer();

CREATE FUNCTION investigation_guard_action_execution_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM verification_prepared_actions action
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=action.campaign_id
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE action.prepared_action_id=NEW.prepared_action_id FOR SHARE OF task_row;
    IF FOUND THEN
        PERFORM investigation_assert_stage_not_closed(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_action_execution_updates_stop_fence
BEFORE UPDATE ON verification_action_executions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_action_execution_update();

CREATE FUNCTION investigation_guard_fact_delta_consumption()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM verification_fact_delta_bundles delta
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=delta.campaign_id
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE delta.fact_delta_bundle_id=NEW.fact_delta_bundle_id FOR SHARE OF task_row;
    IF FOUND THEN
        PERFORM investigation_assert_stage_not_closed(
            task.operation_id,task.stage_execution_id,task.scope_snapshot_id
        );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_fact_delta_consumptions_stop_fence
BEFORE INSERT ON fact_delta_consumptions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_fact_delta_consumption();

CREATE FUNCTION investigation_guard_consolidation_writer()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    admission verification_admission_sets%ROWTYPE;
    batch hypothesis_consolidation_batches%ROWTYPE;
BEGIN
    IF TG_TABLE_NAME='hypothesis_consolidation_batches' THEN
        SELECT * INTO admission FROM verification_admission_sets
         WHERE generation_id=NEW.generation_id ORDER BY created_at DESC LIMIT 1 FOR SHARE;
        IF FOUND THEN
            IF TG_OP='INSERT' THEN
                PERFORM investigation_assert_stage_accepts_new_work(
                    admission.operation_id,admission.stage_execution_id,admission.scope_snapshot_id
                );
            ELSE
                PERFORM investigation_assert_stage_not_closed(
                    admission.operation_id,admission.stage_execution_id,admission.scope_snapshot_id
                );
            END IF;
        END IF;
    ELSE
        SELECT * INTO STRICT batch FROM hypothesis_consolidation_batches
         WHERE consolidation_batch_id=NEW.consolidation_batch_id FOR SHARE;
        SELECT * INTO admission FROM verification_admission_sets
         WHERE generation_id=batch.generation_id ORDER BY created_at DESC LIMIT 1 FOR SHARE;
        IF FOUND THEN
            PERFORM investigation_assert_stage_not_closed(
                admission.operation_id,admission.stage_execution_id,admission.scope_snapshot_id
            );
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_consolidation_batches_stop_fence
BEFORE INSERT OR UPDATE ON hypothesis_consolidation_batches
FOR EACH ROW EXECUTE FUNCTION investigation_guard_consolidation_writer();
CREATE TRIGGER investigation_consolidation_receipts_stop_fence
BEFORE INSERT ON hypothesis_consolidation_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_consolidation_writer();

CREATE OR REPLACE FUNCTION investigation_request_stop_v1(
    p_stop_intent_id UUID,
    p_idempotency_key UUID,
    p_authority_id UUID,
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_owning_stage_run_request_id TEXT,
    p_expected_run_head_sha256 TEXT,
    p_expected_change_seq BIGINT
)
RETURNS investigation_stop_intents
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_stop_intents%ROWTYPE;
    head investigation_run_heads%ROWTYPE;
    result investigation_stop_intents%ROWTYPE;
    next_epoch BIGINT;
    next_version BIGINT;
    next_change_seq BIGINT;
    work_count BIGINT;
    work_hash TEXT;
    event_id UUID;
    event_hash TEXT;
    receipt_hash TEXT;
BEGIN
    SELECT * INTO existing FROM investigation_stop_intents
     WHERE idempotency_key=p_idempotency_key;
    IF FOUND THEN
        IF ROW(existing.stop_intent_id,existing.authority_id,existing.operation_id,
               existing.stage_execution_id,existing.owning_stage_run_request_id,
               existing.expected_run_head_sha256,existing.expected_change_seq)
           IS DISTINCT FROM
           ROW(p_stop_intent_id,p_authority_id,p_operation_id,p_stage_execution_id,
               p_owning_stage_run_request_id,p_expected_run_head_sha256,p_expected_change_seq)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_STOP_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;

    -- One row lock is the transaction head/CAS and conflicts with every
    -- admission/domain-writer share lock installed above.
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=p_authority_id FOR UPDATE;
    -- A concurrent identical caller may have committed while this transaction
    -- waited for the head. Re-check under the acquired lock so it receives the
    -- exact persisted receipt instead of a stale-head CAS failure.
    SELECT * INTO existing FROM investigation_stop_intents
     WHERE idempotency_key=p_idempotency_key;
    IF FOUND THEN
        IF ROW(existing.stop_intent_id,existing.authority_id,existing.operation_id,
               existing.stage_execution_id,existing.owning_stage_run_request_id,
               existing.expected_run_head_sha256,existing.expected_change_seq)
           IS DISTINCT FROM
           ROW(p_stop_intent_id,p_authority_id,p_operation_id,p_stage_execution_id,
               p_owning_stage_run_request_id,p_expected_run_head_sha256,p_expected_change_seq)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_STOP_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    IF head.run_state<>'running' OR NOT head.admission_open
       OR head.operation_id<>p_operation_id
       OR head.stage_execution_id<>p_stage_execution_id
       OR head.owning_stage_run_request_id<>p_owning_stage_run_request_id
       OR head.head_sha256<>p_expected_run_head_sha256
       OR head.change_seq<>p_expected_change_seq
    THEN
        RAISE EXCEPTION 'INVESTIGATION_STOP_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    next_epoch := head.stop_epoch+1;
    next_version := head.head_version+1;
    next_change_seq := head.change_seq+1;

    CREATE TEMP TABLE IF NOT EXISTS investigation_stop_freeze_v1(
        work_class TEXT NOT NULL,
        source_kind TEXT NOT NULL,
        source_id UUID NOT NULL,
        source_identity_sha256 TEXT NOT NULL,
        frozen_state TEXT NOT NULL,
        frozen_head_version BIGINT NOT NULL,
        member_sha256 TEXT,
        PRIMARY KEY(source_kind,source_id)
    ) ON COMMIT DROP;
    TRUNCATE investigation_stop_freeze_v1;

    INSERT INTO investigation_stop_freeze_v1(
        work_class,source_kind,source_id,source_identity_sha256,
        frozen_state,frozen_head_version
    )
    SELECT work.work_kind,'runtime_work',work.work_id,work.external_identity_sha256,
           work.current_state,work.head_version
      FROM investigation_run_work_items work
     WHERE work.authority_id=p_authority_id
       AND NOT unified_investigation_work_state_terminal(work.current_state)
    UNION ALL
    SELECT 'read_session','main_read_session',session_row.main_read_session_id,
           session_row.member_sha256,'awaiting_receipt',0
      FROM investigation_main_read_sessions session_row
     WHERE session_row.authority_id=p_authority_id
       AND NOT EXISTS(
           SELECT 1 FROM investigation_main_read_session_receipts receipt
            WHERE receipt.main_read_session_id=session_row.main_read_session_id
       )
    UNION ALL
    SELECT 'verification_task','verification_task',task.task_id,
           task.stable_task_key_sha256,COALESCE(task_head.current_state,'missing_state_head'),
           COALESCE(task_head.head_version,0)
      FROM hypothesis_verification_tasks task
      LEFT JOIN hypothesis_verification_task_state_heads task_head ON task_head.task_id=task.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id
       AND (task_head.task_id IS NULL OR task_head.current_state NOT IN(
           'cancelled','blocked','terminal'
       ))
    UNION ALL
    SELECT 'pentagi_plan','pentagi_plan',plan.task_plan_id,plan.task_plan_sha256,
           plan.status,plan.row_version
      FROM investigation_pentagi_task_plans plan
     WHERE plan.authority_id=p_authority_id AND plan.status='open'
    UNION ALL
    SELECT 'pentagi_subtask','pentagi_subtask',subtask.subtask_id,subtask.member_sha256,
           'plan_open',plan.row_version
      FROM investigation_pentagi_subtasks subtask
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=subtask.task_plan_id
     WHERE plan.authority_id=p_authority_id AND plan.status='open'
    UNION ALL
    SELECT 'stage_work_item','stage_work_item',item.id,item.input_manifest_hash,
           item.status,item.row_version
      FROM stage_work_items item
     WHERE item.operation_id=head.operation_id
       AND item.stage_execution_id=head.stage_execution_id
       AND item.scope_snapshot_id=head.scope_snapshot_id
       AND item.status NOT IN('completed','exhausted','superseded')
    UNION ALL
    SELECT 'worker_request','stage_worker_request',request.id,request.request_payload_hash,
           request.status || ':' || item.status,item.row_version
      FROM stage_worker_requests request
      JOIN stage_work_items item ON item.id=request.accepted_work_item_id
     WHERE request.operation_id=head.operation_id
       AND request.stage_execution_id=head.stage_execution_id
       AND request.scope_snapshot_id=head.scope_snapshot_id
       AND request.status='accepted'
       AND item.status NOT IN('completed','exhausted','superseded')
    UNION ALL
    SELECT 'worker_run','stage_worker_run',worker.id,
           'sha256:' || encode(digest(convert_to(
               concat_ws(':','golish.investigation_worker_run.v1',worker.id::TEXT,
                   worker.work_item_id::TEXT,worker.attempt_epoch::TEXT),
               'UTF8'),'sha256'),'hex'),
           worker.status,worker.checkpoint_version
      FROM stage_worker_runs worker
      JOIN stage_run_units unit ON unit.id=worker.stage_run_unit_id
     WHERE worker.operation_id=head.operation_id
       AND worker.stage_execution_id=head.stage_execution_id
       AND unit.scope_snapshot_id=head.scope_snapshot_id
       AND worker.status NOT IN('passed','failed','exhausted','superseded')
    UNION ALL
    SELECT 'campaign','campaign',campaign.campaign_id,campaign.source_snapshot_hash,
           campaign.state,campaign.row_version
      FROM verification_campaigns campaign
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=campaign.campaign_id
      JOIN hypothesis_verification_tasks task ON task.task_id=reservation.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id
       AND campaign.state NOT IN('terminal','superseded')
    UNION ALL
    SELECT 'prepared_action','prepared_action',action.prepared_action_id,
           action.private_manifest_hash,action.state,action.row_version
      FROM verification_prepared_actions action
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=action.campaign_id
      JOIN hypothesis_verification_tasks task ON task.task_id=reservation.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id
       AND action.state IN('pending_authorization','authorized','started','outcome_unknown')
    UNION ALL
    SELECT 'action_execution','action_execution',execution.action_execution_id,
           execution.durable_begin_hash,execution.state,execution.row_version
      FROM verification_action_executions execution
      JOIN verification_prepared_actions action
        ON action.prepared_action_id=execution.prepared_action_id
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=action.campaign_id
      JOIN hypothesis_verification_tasks task ON task.task_id=reservation.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id
       AND execution.state IN('started','outcome_unknown')
    UNION ALL
    SELECT 'fact_delta','fact_delta',delta.fact_delta_bundle_id,delta.fact_delta_hash,
           'unconsumed',0
      FROM verification_fact_delta_bundles delta
      JOIN hypothesis_verification_task_campaigns reservation
        ON reservation.campaign_id=delta.campaign_id
      JOIN hypothesis_verification_tasks task ON task.task_id=reservation.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id
       AND NOT EXISTS(
           SELECT 1 FROM fact_delta_consumptions consumption
            WHERE consumption.fact_delta_bundle_id=delta.fact_delta_bundle_id
       )
    UNION ALL
    SELECT 'consolidation','consolidation',batch.consolidation_batch_id,
           batch.source_snapshot_hash,
           CASE WHEN batch.sealed_at IS NULL THEN 'open' ELSE 'awaiting_receipt' END,0
      FROM hypothesis_consolidation_batches batch
      JOIN verification_admission_sets admission ON admission.generation_id=batch.generation_id
     WHERE admission.operation_id=head.operation_id
       AND admission.stage_execution_id=head.stage_execution_id
       AND admission.scope_snapshot_id=head.scope_snapshot_id
       AND (batch.sealed_at IS NULL OR NOT EXISTS(
           SELECT 1 FROM hypothesis_consolidation_receipts receipt
            WHERE receipt.consolidation_batch_id=batch.consolidation_batch_id
       ));

    UPDATE investigation_stop_freeze_v1 frozen
       SET member_sha256='sha256:' || encode(digest(convert_to(
           concat_ws(':','golish.investigation_stop_denominator_member.v1',
               frozen.work_class,frozen.source_kind,frozen.source_id::TEXT,
               frozen.source_identity_sha256,frozen.frozen_state,
               frozen.frozen_head_version::TEXT),
           'UTF8'),'sha256'),'hex');
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_stop_denominator.v1',
               COALESCE(array_agg(member_sha256 ORDER BY source_kind,source_id),ARRAY[]::TEXT[])
           )
      INTO work_count,work_hash FROM investigation_stop_freeze_v1;

    receipt_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_stop_intent.v2',p_stop_intent_id::TEXT,
            p_authority_id::TEXT,next_epoch::TEXT,work_count::TEXT,work_hash),
        'UTF8'),'sha256'),'hex');
    INSERT INTO investigation_stop_intents(
        stop_intent_id,idempotency_key,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,expected_run_head_sha256,expected_change_seq,
        stop_epoch,frozen_work_count,frozen_work_set_sha256,receipt_sha256
    ) VALUES(
        p_stop_intent_id,p_idempotency_key,p_authority_id,p_operation_id,
        p_stage_execution_id,p_owning_stage_run_request_id,p_expected_run_head_sha256,
        p_expected_change_seq,next_epoch,work_count,work_hash,receipt_hash
    );
    INSERT INTO investigation_stop_denominator_members(
        stop_denominator_member_id,stop_intent_id,authority_id,stop_epoch,
        work_class,source_kind,source_id,source_identity_sha256,frozen_state,
        frozen_head_version,member_sha256
    )
    SELECT gen_random_uuid(),p_stop_intent_id,p_authority_id,next_epoch,
           work_class,source_kind,source_id,source_identity_sha256,frozen_state,
           frozen_head_version,member_sha256
      FROM investigation_stop_freeze_v1 ORDER BY source_kind,source_id;
    INSERT INTO investigation_stop_work_members(
        stop_member_id,stop_intent_id,authority_id,stop_epoch,work_id,work_kind,
        external_identity_sha256,frozen_state,frozen_head_version,member_sha256
    )
    SELECT gen_random_uuid(),p_stop_intent_id,p_authority_id,next_epoch,work_id,work_kind,
           external_identity_sha256,current_state,head_version,
           'sha256:' || encode(digest(convert_to(
               concat_ws(':','golish.investigation_stop_member.v1',work_id::TEXT,
                   external_identity_sha256,current_state,head_version::TEXT),
               'UTF8'),'sha256'),'hex')
      FROM investigation_run_work_items
     WHERE authority_id=p_authority_id
       AND NOT unified_investigation_work_state_terminal(current_state)
     ORDER BY work_kind,work_id;

    event_id := gen_random_uuid();
    event_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_run_event.v1',event_id::TEXT,
            head.head_sha256,'stop_pending',next_epoch::TEXT,next_change_seq::TEXT),
        'UTF8'),'sha256'),'hex');
    INSERT INTO investigation_run_state_events(
        event_id,stable_request_id,authority_id,event_ordinal,expected_head_sha256,
        from_state,to_state,stop_epoch,change_seq,event_sha256
    ) VALUES(
        event_id,p_idempotency_key,p_authority_id,next_version,head.head_sha256,
        head.run_state,'stop_pending',next_epoch,next_change_seq,event_hash
    );
    PERFORM set_config('golish.investigation_run_head_write','on',TRUE);
    UPDATE investigation_run_heads
       SET run_state='stop_pending',admission_open=FALSE,stop_epoch=next_epoch,
           change_seq=next_change_seq,head_version=next_version,latest_event_id=event_id,
           head_sha256=unified_investigation_runtime_head_sha256(
               authority_id,'stop_pending',FALSE,next_epoch,next_change_seq,next_version
           ),updated_at=statement_timestamp()
     WHERE authority_id=p_authority_id;
    PERFORM set_config('golish.investigation_run_head_write','off',TRUE);
    SELECT * INTO STRICT result FROM investigation_stop_intents
     WHERE stop_intent_id=p_stop_intent_id;
    RETURN result;
END;
$$;

CREATE FUNCTION investigation_stop_denominator_member_is_terminal(
    p_source_kind TEXT,
    p_source_id UUID
)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    terminal BOOLEAN;
BEGIN
    CASE p_source_kind
        WHEN 'runtime_work' THEN
            SELECT unified_investigation_work_state_terminal(current_state)
              INTO terminal FROM investigation_run_work_items WHERE work_id=p_source_id;
        WHEN 'main_read_session' THEN
            SELECT EXISTS(SELECT 1 FROM investigation_main_read_session_receipts
                           WHERE main_read_session_id=p_source_id) INTO terminal;
        WHEN 'verification_task' THEN
            SELECT EXISTS(
                SELECT 1 FROM hypothesis_verification_task_state_heads
                 WHERE task_id=p_source_id AND current_state IN('cancelled','blocked','terminal')
            ) INTO terminal;
        WHEN 'pentagi_plan' THEN
            SELECT status='sealed' INTO terminal FROM investigation_pentagi_task_plans
             WHERE task_plan_id=p_source_id;
        WHEN 'pentagi_subtask' THEN
            SELECT plan.status='sealed' INTO terminal
              FROM investigation_pentagi_subtasks subtask
              JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=subtask.task_plan_id
             WHERE subtask.subtask_id=p_source_id;
        WHEN 'stage_work_item' THEN
            SELECT status IN('completed','exhausted','superseded') INTO terminal
              FROM stage_work_items WHERE id=p_source_id;
        WHEN 'stage_worker_request' THEN
            SELECT item.status IN('completed','exhausted','superseded') INTO terminal
              FROM stage_worker_requests request
              JOIN stage_work_items item ON item.id=request.accepted_work_item_id
             WHERE request.id=p_source_id AND request.status='accepted';
        WHEN 'stage_worker_run' THEN
            SELECT status IN('passed','failed','exhausted','superseded') INTO terminal
              FROM stage_worker_runs WHERE id=p_source_id;
        WHEN 'campaign' THEN
            SELECT state IN('terminal','superseded') INTO terminal
              FROM verification_campaigns WHERE campaign_id=p_source_id;
        WHEN 'prepared_action' THEN
            SELECT state NOT IN('pending_authorization','authorized','started','outcome_unknown')
              INTO terminal FROM verification_prepared_actions WHERE prepared_action_id=p_source_id;
        WHEN 'action_execution' THEN
            SELECT state IN('succeeded','failed') INTO terminal
              FROM verification_action_executions WHERE action_execution_id=p_source_id;
        WHEN 'fact_delta' THEN
            SELECT EXISTS(SELECT 1 FROM fact_delta_consumptions
                           WHERE fact_delta_bundle_id=p_source_id) INTO terminal;
        WHEN 'consolidation' THEN
            SELECT batch.sealed_at IS NOT NULL AND EXISTS(
                       SELECT 1 FROM hypothesis_consolidation_receipts receipt
                        WHERE receipt.consolidation_batch_id=batch.consolidation_batch_id
                   )
              INTO terminal FROM hypothesis_consolidation_batches batch
             WHERE batch.consolidation_batch_id=p_source_id;
        ELSE
            terminal := FALSE;
    END CASE;
    RETURN COALESCE(terminal,FALSE);
END;
$$;

CREATE TABLE investigation_run_closure_v1_authorities (
    closure_id UUID PRIMARY KEY,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    run_state_head_version BIGINT NOT NULL CHECK(run_state_head_version>=0),
    stop_epoch BIGINT NOT NULL CHECK(stop_epoch>0),
    snapshot_member_count BIGINT NOT NULL CHECK(snapshot_member_count>0),
    snapshot_member_set_sha256 TEXT NOT NULL CHECK(snapshot_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    main_read_session_member_count BIGINT NOT NULL CHECK(main_read_session_member_count>0),
    main_read_session_member_set_sha256 TEXT NOT NULL CHECK(main_read_session_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    generation_member_count BIGINT NOT NULL CHECK(generation_member_count>0),
    generation_member_set_sha256 TEXT NOT NULL CHECK(generation_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    admission_member_count BIGINT NOT NULL CHECK(admission_member_count>0),
    admission_member_set_sha256 TEXT NOT NULL CHECK(admission_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_task_member_count BIGINT NOT NULL CHECK(verification_task_member_count>=0),
    verification_task_member_set_sha256 TEXT NOT NULL CHECK(verification_task_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    objective_assignment_member_count BIGINT NOT NULL CHECK(objective_assignment_member_count>=0),
    objective_assignment_member_set_sha256 TEXT NOT NULL CHECK(objective_assignment_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    objective_outcome_member_count BIGINT NOT NULL CHECK(objective_outcome_member_count>=0),
    objective_outcome_member_set_sha256 TEXT NOT NULL CHECK(objective_outcome_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    work_total_count BIGINT NOT NULL CHECK(work_total_count>=0),
    work_terminal_count BIGINT NOT NULL CHECK(work_terminal_count>=0),
    work_cancelled_before_start_count BIGINT NOT NULL CHECK(work_cancelled_before_start_count>=0),
    work_recovery_required_count BIGINT NOT NULL CHECK(work_recovery_required_count>=0),
    work_member_set_sha256 TEXT NOT NULL CHECK(work_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    campaign_total_count BIGINT NOT NULL CHECK(campaign_total_count>=0),
    campaign_terminal_count BIGINT NOT NULL CHECK(campaign_terminal_count>=0),
    campaign_cancelled_before_start_count BIGINT NOT NULL CHECK(campaign_cancelled_before_start_count>=0),
    campaign_recovery_required_count BIGINT NOT NULL CHECK(campaign_recovery_required_count>=0),
    campaign_member_set_sha256 TEXT NOT NULL CHECK(campaign_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    prepared_action_total_count BIGINT NOT NULL CHECK(prepared_action_total_count>=0),
    prepared_action_terminal_count BIGINT NOT NULL CHECK(prepared_action_terminal_count>=0),
    prepared_action_cancelled_before_start_count BIGINT NOT NULL CHECK(prepared_action_cancelled_before_start_count>=0),
    prepared_action_recovery_required_count BIGINT NOT NULL CHECK(prepared_action_recovery_required_count>=0),
    prepared_action_member_set_sha256 TEXT NOT NULL CHECK(prepared_action_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fact_delta_total_count BIGINT NOT NULL CHECK(fact_delta_total_count>=0),
    fact_delta_terminal_count BIGINT NOT NULL CHECK(fact_delta_terminal_count>=0),
    fact_delta_cancelled_before_start_count BIGINT NOT NULL CHECK(fact_delta_cancelled_before_start_count>=0),
    fact_delta_recovery_required_count BIGINT NOT NULL CHECK(fact_delta_recovery_required_count>=0),
    fact_delta_member_set_sha256 TEXT NOT NULL CHECK(fact_delta_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    delegation_task_count BIGINT NOT NULL CHECK(delegation_task_count>=0),
    delegation_primary_count BIGINT NOT NULL CHECK(delegation_primary_count>=0),
    delegation_runnable_subtask_count BIGINT NOT NULL CHECK(delegation_runnable_subtask_count>=0),
    delegation_independently_dispatched_subtask_count BIGINT NOT NULL CHECK(delegation_independently_dispatched_subtask_count>=0),
    delegation_logical_dispatch_count BIGINT NOT NULL CHECK(delegation_logical_dispatch_count>=0),
    delegation_unique_logical_dispatch_count BIGINT NOT NULL CHECK(delegation_unique_logical_dispatch_count>=0),
    delegation_sealed_task_census_count BIGINT NOT NULL CHECK(delegation_sealed_task_census_count>=0),
    delegation_member_set_sha256 TEXT NOT NULL CHECK(delegation_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fuel_reservation_count BIGINT NOT NULL CHECK(fuel_reservation_count>=0),
    fuel_consumed_count BIGINT NOT NULL CHECK(fuel_consumed_count>=0),
    fuel_refunded_count BIGINT NOT NULL CHECK(fuel_refunded_count>=0),
    fuel_unknown_held_count BIGINT NOT NULL CHECK(fuel_unknown_held_count>=0),
    fuel_open_count BIGINT NOT NULL CHECK(fuel_open_count>=0),
    fuel_semantic_cycle_count BIGINT NOT NULL CHECK(fuel_semantic_cycle_count>=0),
    fuel_reservation_set_sha256 TEXT NOT NULL CHECK(fuel_reservation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fuel_semantic_cycle_set_sha256 TEXT NOT NULL CHECK(fuel_semantic_cycle_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fixed_point_receipt_id UUID NOT NULL UNIQUE,
    fixed_point_receipt_sha256 TEXT NOT NULL CHECK(fixed_point_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_member_count BIGINT NOT NULL CHECK(residual_member_count>=0),
    residual_member_set_sha256 TEXT NOT NULL CHECK(residual_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK(disposition IN('pass','pass_with_gaps')),
    contract_version TEXT NOT NULL CHECK(contract_version='investigation_run_closure.v1'),
    closure_sha256 TEXT NOT NULL CHECK(closure_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(fixed_point_receipt_id,authority_id)
        REFERENCES investigation_stage_fixed_point_receipts(fixed_point_receipt_id,authority_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(closure_id,authority_id)
        REFERENCES investigation_run_closures(closure_id,authority_id)
        ON DELETE RESTRICT
);

CREATE TRIGGER investigation_run_closure_v1_authorities_append_only
BEFORE UPDATE OR DELETE ON investigation_run_closure_v1_authorities
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

DROP FUNCTION seal_investigation_run_closure_v1(UUID,UUID,UUID,TEXT,TEXT,TEXT);

CREATE FUNCTION seal_investigation_run_closure_v1(
    p_closure_id UUID,
    p_stable_request_id UUID,
    p_authority_id UUID,
    p_expected_run_head_sha256 TEXT
)
RETURNS investigation_run_closure_v1_authorities
LANGUAGE plpgsql
AS $$
DECLARE
    existing_header investigation_run_closures%ROWTYPE;
    existing investigation_run_closure_v1_authorities%ROWTYPE;
    head investigation_run_heads%ROWTYPE;
    stop investigation_stop_intents%ROWTYPE;
    session_set investigation_main_session_sets%ROWTYPE;
    snapshot_count BIGINT;
    snapshot_hash TEXT;
    session_count BIGINT;
    session_hash TEXT;
    generation_count BIGINT;
    generation_hash TEXT;
    admission_count BIGINT;
    admission_hash TEXT;
    verification_task_count BIGINT;
    verification_task_hash TEXT;
    assignment_count BIGINT;
    assignment_hash TEXT;
    outcome_count BIGINT;
    outcome_hash TEXT;
    work_total BIGINT;
    work_terminal BIGINT;
    work_cancelled BIGINT;
    work_recovery BIGINT;
    work_hash TEXT;
    campaign_total BIGINT;
    campaign_terminal BIGINT;
    campaign_cancelled BIGINT;
    campaign_recovery BIGINT;
    campaign_hash TEXT;
    action_total BIGINT;
    action_terminal BIGINT;
    action_cancelled BIGINT;
    action_recovery BIGINT;
    action_hash TEXT;
    delta_total BIGINT;
    delta_terminal BIGINT;
    delta_cancelled BIGINT;
    delta_recovery BIGINT;
    delta_hash TEXT;
    delegation_tasks BIGINT;
    delegation_primaries BIGINT;
    delegation_runnable BIGINT;
    delegation_independent BIGINT;
    delegation_dispatches BIGINT;
    delegation_unique_dispatches BIGINT;
    delegation_sealed BIGINT;
    delegation_hash TEXT;
    plan_hash TEXT;
    fuel_reservations BIGINT;
    fuel_consumed BIGINT;
    fuel_refunded BIGINT;
    fuel_unknown BIGINT;
    fuel_open BIGINT;
    fuel_hash TEXT;
    cycle_count BIGINT;
    cycle_hash TEXT;
    latest_generation_count BIGINT;
    source_fixed_point_count BIGINT;
    source_fixed_point_hash TEXT;
    source_fixed_point_residual_hash TEXT;
    stage_fixed_point_id UUID;
    stage_fixed_point_hash TEXT;
    residual_count BIGINT;
    residual_hash TEXT;
    closure_disposition TEXT;
    closure_hash TEXT;
    next_version BIGINT;
    next_change_seq BIGINT;
    event_id UUID;
    event_hash TEXT;
BEGIN
    SELECT * INTO existing_header FROM investigation_run_closures
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF existing_header.closure_id<>p_closure_id
           OR existing_header.authority_id<>p_authority_id
        THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        SELECT * INTO existing FROM investigation_run_closure_v1_authorities
         WHERE closure_id=existing_header.closure_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_V1_AUTHORITY_MISSING' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;

    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=p_authority_id FOR UPDATE;
    IF head.run_state NOT IN('stop_pending','draining') OR head.admission_open
       OR head.head_sha256<>p_expected_run_head_sha256
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT stop FROM investigation_stop_intents
     WHERE authority_id=p_authority_id AND stop_epoch=head.stop_epoch FOR SHARE;

    -- Freeze every source relation used below until the closed head is
    -- committed. This prevents a concurrent terminal update/late insert from
    -- changing a census between two SELECTs in this function.
    LOCK TABLE investigation_main_session_sets,
        investigation_main_read_sessions,
        investigation_main_read_session_receipts,
        investigation_analysis_snapshot_authorities,
        verification_admission_sets,
        verification_admission_members,
        hypothesis_generations,
        hypothesis_generation_members,
        hypothesis_generation_seals,
        hypothesis_verification_tasks,
        hypothesis_verification_task_state_heads,
        hypothesis_verification_task_assignment_sets,
        hypothesis_verification_task_assignment_members,
        hypothesis_verification_task_campaigns,
        hypothesis_verification_task_outcome_sets,
        hypothesis_verification_task_outcome_members,
        verification_campaigns,
        verification_campaign_terminal_decisions,
        verification_prepared_actions,
        verification_action_executions,
        verification_fact_delta_bundles,
        fact_delta_consumptions,
        hypothesis_consolidation_batches,
        hypothesis_consolidation_receipts,
        stage_work_items,
        stage_worker_requests,
        stage_worker_runs,
        investigation_run_work_items,
        investigation_stop_work_members,
        investigation_stop_denominator_members,
        investigation_pentagi_task_plans,
        investigation_pentagi_subtasks,
        pentagi_logical_dispatch_receipts,
        pentagi_logical_dispatch_attempts,
        investigation_pentagi_pipeline_events,
        investigation_pentagi_delegation_census_seals,
        investigation_fuel_budgets,
        investigation_fuel_budget_heads,
        investigation_fuel_reservations,
        investigation_semantic_cycle_receipts,
        hypothesis_fixed_point_receipts,
        hypothesis_residual_risks
        IN SHARE MODE;

    SELECT * INTO session_set FROM investigation_main_session_sets candidate
     WHERE candidate.authority_id=p_authority_id
     ORDER BY candidate.session_set_ordinal DESC LIMIT 1;
    IF NOT FOUND OR session_set.status<>'sealed' OR EXISTS(
        SELECT 1 FROM investigation_main_session_sets candidate
         WHERE candidate.authority_id=p_authority_id AND candidate.status<>'sealed'
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_MAIN_SESSION_SET_NOT_SEALED' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_analysis_snapshot_authorities.v1',
               COALESCE(array_agg(snapshot.snapshot_id::TEXT || ':' || snapshot.snapshot_sha256
                                  ORDER BY snapshot.organization_id,snapshot.snapshot_id),ARRAY[]::TEXT[])
           ) INTO snapshot_count,snapshot_hash
      FROM investigation_main_read_sessions session_row
      JOIN investigation_analysis_snapshot_authorities snapshot
        ON snapshot.snapshot_id=session_row.snapshot_id
       AND snapshot.authority_id=session_row.authority_id
     WHERE session_row.session_set_id=session_set.session_set_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_main_read_sessions.v1',
               COALESCE(array_agg(main_read_session_id::TEXT || ':' || member_sha256
                                  ORDER BY organization_id,main_read_session_id),ARRAY[]::TEXT[])
           ) INTO session_count,session_hash
      FROM investigation_main_read_sessions WHERE session_set_id=session_set.session_set_id;
    IF snapshot_count=0 OR snapshot_count<>session_count
       OR session_set.member_count<>session_count
       OR session_set.member_set_sha256<>session_hash
       OR EXISTS(
           SELECT 1 FROM investigation_main_read_sessions session_row
            WHERE session_row.session_set_id=session_set.session_set_id
              AND NOT EXISTS(
                  SELECT 1 FROM investigation_main_read_session_receipts receipt
                   WHERE receipt.main_read_session_id=session_row.main_read_session_id
              )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_SNAPSHOT_SESSION_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;

    IF EXISTS(
        SELECT 1 FROM verification_admission_sets admission
         WHERE admission.operation_id=head.operation_id
           AND admission.stage_execution_id=head.stage_execution_id
           AND admission.scope_snapshot_id=head.scope_snapshot_id
           AND admission.status<>'sealed'
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_ADMISSION_SET_NOT_SEALED' USING ERRCODE='23514';
    END IF;
    -- The current generation is selected from the Registry, never inferred
    -- from whatever admission rows happen to exist. Recompute both immutable
    -- exact sets so a newer generation without a disposition, or a corrupt
    -- historical seal, cannot be hidden behind an older admitted generation.
    IF EXISTS(
        WITH current_generation AS (
            SELECT DISTINCT ON(generation.organization_id)
                   generation.organization_id,generation.generation_id,
                   generation.generation_ordinal
              FROM hypothesis_generations generation
             WHERE generation.operation_id=head.operation_id
             ORDER BY generation.organization_id,generation.generation_ordinal DESC,
                      generation.generation_id
        )
        SELECT 1
          FROM current_generation current
          LEFT JOIN hypothesis_generation_seals generation_seal
            ON generation_seal.generation_id=current.generation_id
          LEFT JOIN verification_admission_sets admission
            ON admission.generation_id=current.generation_id
           AND admission.operation_id=head.operation_id
           AND admission.stage_execution_id=head.stage_execution_id
           AND admission.scope_snapshot_id=head.scope_snapshot_id
           AND admission.organization_id=current.organization_id
         WHERE generation_seal.seal_id IS NULL
            OR admission.admission_set_id IS NULL
            OR admission.status<>'sealed'
            OR generation_seal.member_count<>(
                SELECT COUNT(*) FROM hypothesis_generation_members member
                 WHERE member.generation_id=current.generation_id
            )
            OR generation_seal.member_set_hash<>(
                SELECT investigation_exact_member_set_hash(
                           'hypothesis_generation_members.v1',
                           COALESCE(array_agg(member.member_hash ORDER BY member.ordinal),ARRAY[]::TEXT[])
                       )
                  FROM hypothesis_generation_members member
                 WHERE member.generation_id=current.generation_id
            )
            OR admission.member_count<>(
                SELECT COUNT(*) FROM verification_admission_members member
                 WHERE member.admission_set_id=admission.admission_set_id
            )
            OR admission.member_set_sha256<>(
                SELECT unified_investigation_exact_set_hash(
                           'verification_admission_members.v1',
                           COALESCE(array_agg(member.member_sha256
                                              ORDER BY member.hypothesis_revision_id),ARRAY[]::TEXT[])
                       )
                  FROM verification_admission_members member
                 WHERE member.admission_set_id=admission.admission_set_id
            )
            OR EXISTS(
                SELECT generation_member_id FROM hypothesis_generation_members
                 WHERE generation_id=current.generation_id
                EXCEPT
                SELECT generation_member_id FROM verification_admission_members
                 WHERE admission_set_id=admission.admission_set_id
            )
            OR EXISTS(
                SELECT generation_member_id FROM verification_admission_members
                 WHERE admission_set_id=admission.admission_set_id
                EXCEPT
                SELECT generation_member_id FROM hypothesis_generation_members
                 WHERE generation_id=current.generation_id
            )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_GENERATION_ADMISSION_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    WITH current_generation AS (
        SELECT DISTINCT ON(generation.organization_id)
               generation.organization_id,generation.generation_id,
               generation.generation_ordinal
          FROM hypothesis_generations generation
         WHERE generation.operation_id=head.operation_id
         ORDER BY generation.organization_id,generation.generation_ordinal DESC,
                  generation.generation_id
    ), latest AS (
        SELECT admission.admission_set_id,current.generation_id,
               admission.member_set_sha256,current.generation_ordinal,
               generation_seal.generation_hash
          FROM current_generation current
          JOIN hypothesis_generation_seals generation_seal
            ON generation_seal.generation_id=current.generation_id
          JOIN verification_admission_sets admission
            ON admission.generation_id=current.generation_id
           AND admission.operation_id=head.operation_id
           AND admission.stage_execution_id=head.stage_execution_id
           AND admission.scope_snapshot_id=head.scope_snapshot_id
           AND admission.organization_id=current.organization_id
           AND admission.status='sealed'
    )
    SELECT COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_current_generations.v1',
               COALESCE(array_agg(latest.generation_id::TEXT || ':' || latest.generation_hash
                                  ORDER BY latest.generation_id),ARRAY[]::TEXT[])),
           COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_current_admission_sets.v1',
               COALESCE(array_agg(latest.admission_set_id::TEXT || ':' || latest.member_set_sha256
                                  ORDER BY latest.admission_set_id),ARRAY[]::TEXT[]))
      INTO generation_count,generation_hash,admission_count,admission_hash
      FROM latest;
    IF generation_count=0 OR generation_count<>admission_count
       OR generation_count<>snapshot_count
       OR EXISTS(
           SELECT organization_id
             FROM investigation_main_read_sessions
            WHERE session_set_id=session_set.session_set_id
           EXCEPT
           SELECT organization_id
             FROM verification_admission_sets
            WHERE operation_id=head.operation_id
              AND stage_execution_id=head.stage_execution_id
              AND scope_snapshot_id=head.scope_snapshot_id
              AND status='sealed'
       )
       OR EXISTS(
           SELECT organization_id
             FROM verification_admission_sets
            WHERE operation_id=head.operation_id
              AND stage_execution_id=head.stage_execution_id
              AND scope_snapshot_id=head.scope_snapshot_id
              AND status='sealed'
           EXCEPT
           SELECT organization_id
             FROM investigation_main_read_sessions
            WHERE session_set_id=session_set.session_set_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_GENERATION_ADMISSION_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;

    IF EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
         LEFT JOIN hypothesis_verification_task_state_heads task_head ON task_head.task_id=task.task_id
        WHERE task.operation_id=head.operation_id
          AND task.stage_execution_id=head.stage_execution_id
          AND task.scope_snapshot_id=head.scope_snapshot_id
          AND (task_head.task_id IS NULL OR task_head.current_state NOT IN('terminal','blocked','cancelled'))
    ) OR EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
         WHERE task.operation_id=head.operation_id
           AND task.stage_execution_id=head.stage_execution_id
           AND task.scope_snapshot_id=head.scope_snapshot_id
           AND NOT EXISTS(
               SELECT 1 FROM verification_admission_members member
                WHERE member.task_id=task.task_id AND member.disposition='scheduled'
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_TASK_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_verification_tasks.v1',
               COALESCE(array_agg(task.task_id::TEXT || ':' || task.stable_task_key_sha256 || ':' ||
                                  task_head.current_state || ':' || task_head.head_version::TEXT
                                  ORDER BY task.task_id),ARRAY[]::TEXT[])
           ) INTO verification_task_count,verification_task_hash
      FROM hypothesis_verification_tasks task
      JOIN hypothesis_verification_task_state_heads task_head ON task_head.task_id=task.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;

    IF EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
         LEFT JOIN hypothesis_verification_task_assignment_sets assignment
           ON assignment.task_id=task.task_id AND assignment.status='sealed'
         LEFT JOIN hypothesis_verification_task_outcome_sets outcome
           ON outcome.task_id=task.task_id AND outcome.status='sealed'
        WHERE task.operation_id=head.operation_id
          AND task.stage_execution_id=head.stage_execution_id
          AND task.scope_snapshot_id=head.scope_snapshot_id
          AND (assignment.assignment_set_id IS NULL OR outcome.outcome_set_id IS NULL)
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_OBJECTIVE_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_objective_assignments.v1',
               COALESCE(array_agg(member.member_sha256 ORDER BY member.task_id,member.plan_objective_id),ARRAY[]::TEXT[])
           ) INTO assignment_count,assignment_hash
      FROM hypothesis_verification_task_assignment_members member
      JOIN hypothesis_verification_tasks task ON task.task_id=member.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_objective_outcomes.v1',
               COALESCE(array_agg(member.member_sha256 ORDER BY member.task_id,member.campaign_id),ARRAY[]::TEXT[])
           ) INTO outcome_count,outcome_hash
      FROM hypothesis_verification_task_outcome_members member
      JOIN hypothesis_verification_tasks task ON task.task_id=member.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;

    SELECT COUNT(*),
           COUNT(*) FILTER(WHERE work.current_state IN('completed','blocked','residual','fixed_point','superseded')
               OR (work.current_state='cancelled' AND EXISTS(
                   SELECT 1 FROM investigation_run_work_state_events event
                    WHERE event.work_id=work.work_id
                      AND event.to_state IN('running','waiting_authorization','unknown','stop_pending','draining')
               ))),
           COUNT(*) FILTER(WHERE work.current_state='cancelled' AND NOT EXISTS(
               SELECT 1 FROM investigation_run_work_state_events event
                WHERE event.work_id=work.work_id
                  AND event.to_state IN('running','waiting_authorization','unknown','stop_pending','draining'))),
           COUNT(*) FILTER(WHERE work.current_state='recovery_required'),
           unified_investigation_exact_set_hash(
               'investigation_run_work_items.v1',
               COALESCE(array_agg(work.work_id::TEXT || ':' || work.external_identity_sha256 || ':' ||
                                  work.current_state || ':' || work.head_version::TEXT
                                  ORDER BY work.work_kind,work.work_id),ARRAY[]::TEXT[])
           ) INTO work_total,work_terminal,work_cancelled,work_recovery,work_hash
      FROM investigation_run_work_items work WHERE work.authority_id=p_authority_id;
    IF work_recovery<>0 OR work_total<>work_terminal+work_cancelled
       OR EXISTS(
           SELECT 1
             FROM investigation_stop_work_members member
             JOIN investigation_run_work_items work ON work.work_id=member.work_id
            WHERE member.stop_intent_id=stop.stop_intent_id
              AND NOT unified_investigation_work_state_terminal(work.current_state)
       )
       OR EXISTS(
           SELECT 1 FROM investigation_stop_denominator_members member
            WHERE member.stop_intent_id=stop.stop_intent_id
              AND NOT investigation_stop_denominator_member_is_terminal(
                  member.source_kind,member.source_id
              )
       )
       OR stop.frozen_work_count<>(
           SELECT COUNT(*) FROM investigation_stop_denominator_members member
            WHERE member.stop_intent_id=stop.stop_intent_id
       )
       OR stop.frozen_work_set_sha256<>(
           SELECT unified_investigation_exact_set_hash(
                      'investigation_stop_denominator.v1',
                      COALESCE(array_agg(member.member_sha256
                                         ORDER BY member.source_kind,member.source_id),ARRAY[]::TEXT[])
                  )
             FROM investigation_stop_denominator_members member
            WHERE member.stop_intent_id=stop.stop_intent_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_WORK_NOT_DRAINED' USING ERRCODE='23514';
    END IF;

    IF EXISTS(
        SELECT 1 FROM hypothesis_verification_task_outcome_members outcome_member
        JOIN hypothesis_verification_tasks task ON task.task_id=outcome_member.task_id
        LEFT JOIN verification_campaign_terminal_decisions terminal
          ON terminal.campaign_id=outcome_member.campaign_id
         AND terminal.campaign_terminal_decision_id=outcome_member.terminal_receipt_id
         AND terminal.terminal_hash=outcome_member.terminal_receipt_sha256
        LEFT JOIN investigation_stop_intents campaign_stop
          ON campaign_stop.stop_intent_id=outcome_member.terminal_receipt_id
         AND campaign_stop.receipt_sha256=outcome_member.terminal_receipt_sha256
        LEFT JOIN verification_campaigns actual_campaign
          ON actual_campaign.campaign_id=outcome_member.campaign_id
        WHERE task.operation_id=head.operation_id
          AND task.stage_execution_id=head.stage_execution_id
          AND task.scope_snapshot_id=head.scope_snapshot_id
          AND ((outcome_member.outcome_kind IN('completed','blocked') AND (
                   terminal.campaign_id IS NULL
                   OR actual_campaign.campaign_id IS NULL
                   OR actual_campaign.operation_id<>task.operation_id
                   OR actual_campaign.organization_id<>task.organization_id
                   OR actual_campaign.hypothesis_revision_id<>task.hypothesis_revision_id
                   OR actual_campaign.verification_plan_id<>task.verification_plan_id
               ))
            OR (outcome_member.outcome_kind='cancelled_before_start' AND (
                   campaign_stop.stop_intent_id IS NULL
                   OR actual_campaign.campaign_id IS NOT NULL
               )))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_CAMPAIGN_TERMINAL_RECEIPT_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind IN('completed','blocked')),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind='cancelled_before_start'),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind='recovery_required'),
           unified_investigation_exact_set_hash(
               'investigation_campaign_work.v1',
               COALESCE(array_agg(campaign.campaign_id::TEXT || ':' || campaign.reservation_sha256 || ':' ||
                                  outcome_member.outcome_kind || ':' || outcome_member.member_sha256
                                  ORDER BY campaign.campaign_id),ARRAY[]::TEXT[])
           ) INTO campaign_total,campaign_terminal,campaign_cancelled,campaign_recovery,campaign_hash
      FROM hypothesis_verification_task_campaigns campaign
      JOIN hypothesis_verification_tasks task ON task.task_id=campaign.task_id
      JOIN hypothesis_verification_task_outcome_members outcome_member
        ON outcome_member.task_id=campaign.task_id AND outcome_member.campaign_id=campaign.campaign_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;
    IF campaign_recovery<>0 OR campaign_total<>campaign_terminal+campaign_cancelled THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_CAMPAIGN_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),
           COUNT(*) FILTER(WHERE action.state IN('succeeded','failed') OR (
               action.state='manually_blocked' AND EXISTS(
                   SELECT 1 FROM verification_action_executions execution
                    WHERE execution.prepared_action_id=action.prepared_action_id
               )
           )),
           COUNT(*) FILTER(WHERE action.state IN(
               'compile_rejected','denied','expired','superseded','manually_blocked'
           ) AND NOT EXISTS(
               SELECT 1 FROM verification_action_executions execution
                WHERE execution.prepared_action_id=action.prepared_action_id
           )),
           COUNT(*) FILTER(WHERE action.state='outcome_unknown'),
           unified_investigation_exact_set_hash(
               'investigation_prepared_action_work.v1',
               COALESCE(array_agg(action.prepared_action_id::TEXT || ':' || action.private_manifest_hash || ':' ||
                                  action.state || ':' || action.row_version::TEXT || ':' ||
                                  CASE WHEN EXISTS(
                                      SELECT 1 FROM verification_action_executions execution
                                       WHERE execution.prepared_action_id=action.prepared_action_id
                                  ) THEN 'begun' ELSE 'not_begun' END
                                  ORDER BY action.prepared_action_id),ARRAY[]::TEXT[])
           ) INTO action_total,action_terminal,action_cancelled,action_recovery,action_hash
      FROM verification_prepared_actions action
      JOIN hypothesis_verification_task_campaigns task_campaign ON task_campaign.campaign_id=action.campaign_id
      JOIN hypothesis_verification_tasks task ON task.task_id=task_campaign.task_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;
    IF action_recovery<>0 OR action_total<>action_terminal+action_cancelled THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PREPARED_ACTION_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;

    IF EXISTS(
        SELECT 1 FROM hypothesis_verification_task_campaigns task_campaign
        JOIN hypothesis_verification_tasks task ON task.task_id=task_campaign.task_id
        JOIN hypothesis_verification_task_outcome_members outcome_member
          ON outcome_member.task_id=task_campaign.task_id AND outcome_member.campaign_id=task_campaign.campaign_id
        LEFT JOIN verification_fact_delta_bundles delta ON delta.campaign_id=task_campaign.campaign_id
        WHERE task.operation_id=head.operation_id
          AND task.stage_execution_id=head.stage_execution_id
          AND task.scope_snapshot_id=head.scope_snapshot_id
          AND outcome_member.outcome_kind IN('completed','blocked')
          AND (delta.fact_delta_bundle_id IS NULL OR NOT EXISTS(
              SELECT 1 FROM fact_delta_consumptions consumption
               WHERE consumption.fact_delta_bundle_id=delta.fact_delta_bundle_id
          ))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_FACT_DELTA_MISSING' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind IN('completed','blocked')
               AND delta.fact_delta_bundle_id IS NOT NULL AND EXISTS(
                   SELECT 1 FROM fact_delta_consumptions consumption
                    WHERE consumption.fact_delta_bundle_id=delta.fact_delta_bundle_id
               )),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind='cancelled_before_start'),
           COUNT(*) FILTER(WHERE outcome_member.outcome_kind='recovery_required'),
           unified_investigation_exact_set_hash(
               'investigation_fact_delta_work.v1',
               COALESCE(array_agg(task_campaign.campaign_id::TEXT || ':' || outcome_member.outcome_kind || ':' ||
                                  COALESCE(delta.fact_delta_hash,outcome_member.member_sha256)
                                  ORDER BY task_campaign.campaign_id),ARRAY[]::TEXT[])
           ) INTO delta_total,delta_terminal,delta_cancelled,delta_recovery,delta_hash
      FROM hypothesis_verification_task_campaigns task_campaign
      JOIN hypothesis_verification_tasks task ON task.task_id=task_campaign.task_id
      JOIN hypothesis_verification_task_outcome_members outcome_member
        ON outcome_member.task_id=task_campaign.task_id AND outcome_member.campaign_id=task_campaign.campaign_id
      LEFT JOIN verification_fact_delta_bundles delta ON delta.campaign_id=task_campaign.campaign_id
     WHERE task.operation_id=head.operation_id
       AND task.stage_execution_id=head.stage_execution_id
       AND task.scope_snapshot_id=head.scope_snapshot_id;
    IF delta_recovery<>0 OR delta_total<>delta_terminal+delta_cancelled THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_FACT_DELTA_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM hypothesis_consolidation_batches batch
        JOIN verification_admission_sets admission ON admission.generation_id=batch.generation_id
        WHERE admission.operation_id=head.operation_id
          AND admission.stage_execution_id=head.stage_execution_id
          AND admission.scope_snapshot_id=head.scope_snapshot_id
          AND (batch.sealed_at IS NULL OR NOT EXISTS(
              SELECT 1 FROM hypothesis_consolidation_receipts receipt
               WHERE receipt.consolidation_batch_id=batch.consolidation_batch_id
          ))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_CONSOLIDATION_NOT_TERMINAL' USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_task_plans.v1',
               COALESCE(array_agg(plan.task_plan_sha256 ORDER BY plan.task_plan_id),ARRAY[]::TEXT[])
           ) INTO delegation_tasks,plan_hash
      FROM investigation_pentagi_task_plans plan WHERE plan.authority_id=p_authority_id;
    SELECT COUNT(*) FILTER(WHERE dispatch.actor_kind='primary'),
           COUNT(*),COUNT(DISTINCT dispatch.logical_dispatch_key_sha256),
           unified_investigation_exact_set_hash(
               'pentagi_logical_dispatch_receipts.v1',
               COALESCE(array_agg(dispatch.receipt_sha256 ORDER BY dispatch.dispatch_receipt_id),ARRAY[]::TEXT[])
           ) INTO delegation_primaries,delegation_dispatches,
                  delegation_unique_dispatches,delegation_hash
      FROM pentagi_logical_dispatch_receipts dispatch
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
     WHERE plan.authority_id=p_authority_id;
    SELECT COUNT(*) INTO delegation_runnable
      FROM investigation_pentagi_subtasks subtask
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=subtask.task_plan_id
     WHERE plan.authority_id=p_authority_id AND subtask.runnable;
    SELECT COUNT(DISTINCT dispatch.subtask_id) INTO delegation_independent
      FROM pentagi_logical_dispatch_receipts dispatch
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
     WHERE plan.authority_id=p_authority_id AND dispatch.subtask_id IS NOT NULL
       AND dispatch.actor_kind IN('worker','nested_worker');
    SELECT COUNT(*) INTO delegation_sealed
      FROM investigation_pentagi_delegation_census_seals census
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=census.task_plan_id
     WHERE plan.authority_id=p_authority_id;
    IF delegation_tasks<>delegation_primaries OR delegation_tasks<>delegation_sealed
       OR delegation_runnable<>delegation_independent
       OR delegation_dispatches<>delegation_unique_dispatches
       OR EXISTS(SELECT 1 FROM investigation_pentagi_task_plans plan
                  WHERE plan.authority_id=p_authority_id AND plan.status<>'sealed')
       OR EXISTS(
           SELECT 1 FROM pentagi_logical_dispatch_receipts dispatch
           JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
           WHERE plan.authority_id=p_authority_id
             AND (NOT EXISTS(SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
                              WHERE attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id)
               OR EXISTS(SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
                          WHERE attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                            AND attempt.outcome='unknown_held'))
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_DELEGATION_NOT_CLOSED' USING ERRCODE='23514';
    END IF;
    WITH delegation_members AS (
        SELECT 'plan:' || plan.task_plan_id::TEXT AS member_key,
               plan.task_plan_sha256 AS member_hash
          FROM investigation_pentagi_task_plans plan
         WHERE plan.authority_id=p_authority_id
        UNION ALL
        SELECT 'subtask:' || subtask.subtask_id::TEXT,subtask.member_sha256
          FROM investigation_pentagi_subtasks subtask
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=subtask.task_plan_id
         WHERE plan.authority_id=p_authority_id
        UNION ALL
        SELECT 'dispatch:' || dispatch.dispatch_receipt_id::TEXT,dispatch.receipt_sha256
          FROM pentagi_logical_dispatch_receipts dispatch
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
         WHERE plan.authority_id=p_authority_id
        UNION ALL
        SELECT 'attempt:' || attempt.dispatch_attempt_id::TEXT,
               unified_investigation_exact_set_hash(
                   'pentagi_dispatch_attempt.v1',
                   ARRAY[attempt.fence_sha256,attempt.outcome,attempt.result_sha256]
               )
          FROM pentagi_logical_dispatch_attempts attempt
          JOIN pentagi_logical_dispatch_receipts dispatch
            ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
         JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
         WHERE plan.authority_id=p_authority_id
        UNION ALL
        SELECT 'pipeline:' || pipeline.pipeline_event_id::TEXT,pipeline.event_sha256
          FROM investigation_pentagi_pipeline_events pipeline
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=pipeline.task_plan_id
         WHERE plan.authority_id=p_authority_id
        UNION ALL
        SELECT 'census:' || census.census_seal_id::TEXT,census.seal_sha256
          FROM investigation_pentagi_delegation_census_seals census
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=census.task_plan_id
         WHERE plan.authority_id=p_authority_id
    )
    SELECT unified_investigation_exact_set_hash(
               'investigation_delegation_closure.v1',
               COALESCE(array_agg(member_key || ':' || member_hash ORDER BY member_key),ARRAY[]::TEXT[])
           ) INTO delegation_hash FROM delegation_members;

    SELECT COUNT(*),
           COUNT(*) FILTER(WHERE reservation.state='consumed'),
           COUNT(*) FILTER(WHERE reservation.state='refunded_before_begin'),
           COUNT(*) FILTER(WHERE reservation.state='unknown_held'),
           COUNT(*) FILTER(WHERE reservation.state='reserved'),
           unified_investigation_exact_set_hash(
               'investigation_fuel_reservations.v1',
               COALESCE(array_agg(reservation.reservation_id::TEXT || ':' || reservation.axis || ':' ||
                                  reservation.amount::TEXT || ':' || reservation.state || ':' ||
                                  reservation.row_version::TEXT ORDER BY reservation.reservation_id),ARRAY[]::TEXT[])
           ) INTO fuel_reservations,fuel_consumed,fuel_refunded,fuel_unknown,fuel_open,fuel_hash
      FROM investigation_fuel_reservations reservation
      JOIN investigation_fuel_budgets budget ON budget.budget_id=reservation.budget_id
     WHERE budget.authority_id=p_authority_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_semantic_cycle_receipts.v1',
               COALESCE(array_agg(cycle.receipt_sha256 ORDER BY cycle.semantic_cycle_receipt_id),ARRAY[]::TEXT[])
           ) INTO cycle_count,cycle_hash
      FROM investigation_semantic_cycle_receipts cycle
     WHERE cycle.operation_id=head.operation_id
       AND cycle.stage_execution_id=head.stage_execution_id
       AND cycle.scope_snapshot_id=head.scope_snapshot_id;
    IF fuel_open<>0 OR fuel_unknown<>0 OR fuel_reservations<>fuel_consumed+fuel_refunded
       OR EXISTS(
           SELECT 1 FROM investigation_fuel_budgets budget
           JOIN investigation_fuel_budget_heads fuel ON fuel.budget_id=budget.budget_id
           WHERE budget.authority_id=p_authority_id
             AND (fuel.reserved_amount<>0 OR fuel.unknown_held_amount<>0)
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_FUEL_NOT_SETTLED' USING ERRCODE='23514';
    END IF;

    WITH latest AS (
        SELECT DISTINCT ON(admission.organization_id)
               admission.organization_id,admission.generation_id,generation.generation_ordinal
          FROM verification_admission_sets admission
          JOIN hypothesis_generations generation ON generation.generation_id=admission.generation_id
         WHERE admission.operation_id=head.operation_id
           AND admission.stage_execution_id=head.stage_execution_id
           AND admission.scope_snapshot_id=head.scope_snapshot_id
           AND admission.status='sealed'
         ORDER BY admission.organization_id,generation.generation_ordinal DESC
    )
    SELECT COUNT(*),COUNT(fixed_point.fixed_point_receipt_id),
           unified_investigation_exact_set_hash(
               'investigation_stage_fixed_point_sources.v1',
               COALESCE(array_agg(fixed_point.fixed_point_receipt_id::TEXT || ':' || fixed_point.fixed_point_hash
                                  ORDER BY fixed_point.fixed_point_receipt_id)
                        FILTER(WHERE fixed_point.fixed_point_receipt_id IS NOT NULL),ARRAY[]::TEXT[])),
           unified_investigation_exact_set_hash(
               'investigation_stage_fixed_point_residuals.v1',
               COALESCE(array_agg(fixed_point.residual_set_hash ORDER BY fixed_point.fixed_point_receipt_id)
                        FILTER(WHERE fixed_point.fixed_point_receipt_id IS NOT NULL),ARRAY[]::TEXT[]))
      INTO latest_generation_count,source_fixed_point_count,source_fixed_point_hash,
           source_fixed_point_residual_hash
      FROM latest
      LEFT JOIN hypothesis_fixed_point_receipts fixed_point
        ON fixed_point.generation_id=latest.generation_id;
    IF latest_generation_count=0 OR source_fixed_point_count<>latest_generation_count THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_FIXED_POINT_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    stage_fixed_point_id := gen_random_uuid();
    stage_fixed_point_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','investigation_stage_fixed_point_receipt.v1',stage_fixed_point_id::TEXT,
            p_authority_id::TEXT,source_fixed_point_count::TEXT,source_fixed_point_hash,
            source_fixed_point_residual_hash),'UTF8'),'sha256'),'hex');
    INSERT INTO investigation_stage_fixed_point_receipts(
        fixed_point_receipt_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id,source_receipt_count,
        source_receipt_set_sha256,residual_set_sha256,receipt_sha256
    ) VALUES(
        stage_fixed_point_id,p_authority_id,head.operation_id,head.stage_execution_id,
        head.owning_stage_run_request_id,head.scope_snapshot_id,source_fixed_point_count,
        source_fixed_point_hash,source_fixed_point_residual_hash,stage_fixed_point_hash
    );

    -- Residual rows that cannot be traced to a generation admitted by this
    -- exact stage authority are deliberately not guessed from operation/org.
    -- Their presence is an authority gap and closure must fail closed.
    IF EXISTS(
        SELECT 1
          FROM hypothesis_residual_risks residual
         WHERE residual.operation_id=head.operation_id
           AND residual.closed_at IS NULL
           AND EXISTS(
               SELECT 1 FROM stage_run_units unit
                WHERE unit.stage_execution_id=head.stage_execution_id
                  AND unit.organization_id=residual.organization_id
                  AND unit.stage_kind='investigation'
                  AND unit.status<>'superseded'
           )
           AND NOT EXISTS(
               SELECT 1
                 FROM verification_admission_sets admission
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=admission.generation_id
                 LEFT JOIN hypothesis_generation_members generation_member
                   ON generation_member.generation_id=generation.generation_id
                WHERE admission.operation_id=head.operation_id
                  AND admission.stage_execution_id=head.stage_execution_id
                  AND admission.scope_snapshot_id=head.scope_snapshot_id
                  AND admission.organization_id=residual.organization_id
                  AND admission.status='sealed'
                  AND (generation_member.revision_id=residual.revision_id
                       OR generation.candidate_snapshot_id=residual.snapshot_id)
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_RESIDUAL_AUTHORITY_AMBIGUOUS' USING ERRCODE='23514';
    END IF;

    WITH residual_members AS (
        SELECT 'risk:' || residual.residual_id::TEXT AS member_key,residual.residual_hash AS member_hash
          FROM hypothesis_residual_risks residual
         WHERE residual.operation_id=head.operation_id AND residual.closed_at IS NULL
           AND EXISTS(
               SELECT 1
                 FROM verification_admission_sets admission
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=admission.generation_id
                 LEFT JOIN hypothesis_generation_members generation_member
                   ON generation_member.generation_id=generation.generation_id
                WHERE admission.operation_id=head.operation_id
                  AND admission.stage_execution_id=head.stage_execution_id
                  AND admission.scope_snapshot_id=head.scope_snapshot_id
                  AND admission.organization_id=residual.organization_id
                  AND admission.status='sealed'
                  AND (generation_member.revision_id=residual.revision_id
                       OR generation.candidate_snapshot_id=residual.snapshot_id)
           )
        UNION
        SELECT 'admission:' || member.admission_member_id::TEXT,member.member_sha256
          FROM verification_admission_members member
         WHERE member.operation_id=head.operation_id
           AND member.stage_execution_id=head.stage_execution_id
           AND member.scope_snapshot_id=head.scope_snapshot_id
           AND member.disposition IN('needs_enrichment','deferred','out_of_scope','unsafe')
        UNION
        SELECT 'assignment:' || member.assignment_member_id::TEXT,member.residual_receipt_sha256
          FROM hypothesis_verification_task_assignment_members member
          JOIN hypothesis_verification_tasks task ON task.task_id=member.task_id
         WHERE task.operation_id=head.operation_id
           AND task.stage_execution_id=head.stage_execution_id
           AND task.scope_snapshot_id=head.scope_snapshot_id
           AND member.assignment_kind='residual'
        UNION
        SELECT 'cycle:' || cycle.semantic_cycle_receipt_id::TEXT,cycle.receipt_sha256
          FROM investigation_semantic_cycle_receipts cycle
         WHERE cycle.operation_id=head.operation_id
           AND cycle.stage_execution_id=head.stage_execution_id
           AND cycle.scope_snapshot_id=head.scope_snapshot_id
           AND cycle.disposition IN('residual','stopped')
    )
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_run_residuals.v1',
               COALESCE(array_agg(member_key || ':' || member_hash ORDER BY member_key),ARRAY[]::TEXT[])
           ) INTO residual_count,residual_hash FROM residual_members;
    closure_disposition := CASE WHEN residual_count=0 THEN 'pass' ELSE 'pass_with_gaps' END;

    closure_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'investigation_run_closure.v1',p_closure_id::TEXT,head.operation_id::TEXT,
        head.stage_execution_id::TEXT,head.owning_stage_run_request_id,
        head.scope_snapshot_id::TEXT,head.head_version::TEXT,head.stop_epoch::TEXT,
        snapshot_count::TEXT,snapshot_hash,session_count::TEXT,session_hash,
        generation_count::TEXT,generation_hash,admission_count::TEXT,admission_hash,
        verification_task_count::TEXT,verification_task_hash,assignment_count::TEXT,
        assignment_hash,outcome_count::TEXT,outcome_hash,work_total::TEXT,work_terminal::TEXT,
        work_cancelled::TEXT,work_recovery::TEXT,work_hash,campaign_total::TEXT,
        campaign_terminal::TEXT,campaign_cancelled::TEXT,campaign_recovery::TEXT,campaign_hash,
        action_total::TEXT,action_terminal::TEXT,action_cancelled::TEXT,action_recovery::TEXT,
        action_hash,delta_total::TEXT,delta_terminal::TEXT,delta_cancelled::TEXT,
        delta_recovery::TEXT,delta_hash,delegation_tasks::TEXT,delegation_primaries::TEXT,
        delegation_runnable::TEXT,delegation_independent::TEXT,delegation_dispatches::TEXT,
        delegation_unique_dispatches::TEXT,delegation_sealed::TEXT,delegation_hash,
        fuel_reservations::TEXT,fuel_consumed::TEXT,fuel_refunded::TEXT,fuel_unknown::TEXT,
        fuel_open::TEXT,cycle_count::TEXT,fuel_hash,cycle_hash,stage_fixed_point_id::TEXT,
        stage_fixed_point_hash,residual_count::TEXT,residual_hash,closure_disposition),
        'UTF8'),'sha256'),'hex');

    INSERT INTO investigation_run_closures(
        closure_id,stable_request_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stop_intent_id,stop_epoch,disposition,
        work_count,work_set_sha256,task_plan_count,task_plan_set_sha256,
        dispatch_count,dispatch_set_sha256,residual_set_sha256,closure_sha256
    ) VALUES(
        p_closure_id,p_stable_request_id,p_authority_id,head.operation_id,
        head.stage_execution_id,head.owning_stage_run_request_id,stop.stop_intent_id,
        head.stop_epoch,closure_disposition,work_total,work_hash,delegation_tasks,plan_hash,
        delegation_dispatches,delegation_hash,residual_hash,closure_hash
    ) RETURNING * INTO existing_header;

    INSERT INTO investigation_run_closure_v1_authorities(
        closure_id,authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,
        scope_snapshot_id,run_state_head_version,stop_epoch,
        snapshot_member_count,snapshot_member_set_sha256,
        main_read_session_member_count,main_read_session_member_set_sha256,
        generation_member_count,generation_member_set_sha256,
        admission_member_count,admission_member_set_sha256,
        verification_task_member_count,verification_task_member_set_sha256,
        objective_assignment_member_count,objective_assignment_member_set_sha256,
        objective_outcome_member_count,objective_outcome_member_set_sha256,
        work_total_count,work_terminal_count,work_cancelled_before_start_count,
        work_recovery_required_count,work_member_set_sha256,
        campaign_total_count,campaign_terminal_count,campaign_cancelled_before_start_count,
        campaign_recovery_required_count,campaign_member_set_sha256,
        prepared_action_total_count,prepared_action_terminal_count,
        prepared_action_cancelled_before_start_count,prepared_action_recovery_required_count,
        prepared_action_member_set_sha256,fact_delta_total_count,fact_delta_terminal_count,
        fact_delta_cancelled_before_start_count,fact_delta_recovery_required_count,
        fact_delta_member_set_sha256,delegation_task_count,delegation_primary_count,
        delegation_runnable_subtask_count,delegation_independently_dispatched_subtask_count,
        delegation_logical_dispatch_count,delegation_unique_logical_dispatch_count,
        delegation_sealed_task_census_count,delegation_member_set_sha256,
        fuel_reservation_count,fuel_consumed_count,fuel_refunded_count,
        fuel_unknown_held_count,fuel_open_count,fuel_semantic_cycle_count,
        fuel_reservation_set_sha256,fuel_semantic_cycle_set_sha256,
        fixed_point_receipt_id,fixed_point_receipt_sha256,
        residual_member_count,residual_member_set_sha256,disposition,contract_version,
        closure_sha256
    ) VALUES(
        p_closure_id,p_authority_id,head.operation_id,head.stage_execution_id,
        head.owning_stage_run_request_id,head.scope_snapshot_id,head.head_version,head.stop_epoch,
        snapshot_count,snapshot_hash,session_count,session_hash,generation_count,generation_hash,
        admission_count,admission_hash,verification_task_count,verification_task_hash,
        assignment_count,assignment_hash,outcome_count,outcome_hash,
        work_total,work_terminal,work_cancelled,work_recovery,work_hash,
        campaign_total,campaign_terminal,campaign_cancelled,campaign_recovery,campaign_hash,
        action_total,action_terminal,action_cancelled,action_recovery,action_hash,
        delta_total,delta_terminal,delta_cancelled,delta_recovery,delta_hash,
        delegation_tasks,delegation_primaries,delegation_runnable,delegation_independent,
        delegation_dispatches,delegation_unique_dispatches,delegation_sealed,delegation_hash,
        fuel_reservations,fuel_consumed,fuel_refunded,fuel_unknown,fuel_open,cycle_count,
        fuel_hash,cycle_hash,stage_fixed_point_id,stage_fixed_point_hash,
        residual_count,residual_hash,closure_disposition,'investigation_run_closure.v1',
        closure_hash
    ) RETURNING * INTO existing;

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
    RETURN existing;
END;
$$;
