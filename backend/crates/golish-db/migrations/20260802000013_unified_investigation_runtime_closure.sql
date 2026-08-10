-- Durable runtime closure authority for unified Investigation.
--
-- This is a forward-only schema seam.  It deliberately reuses StageTeam's
-- stage_team_plans/stage_work_items/stage_worker_requests/stage_worker_runs as
-- the physical actor adapter and adds only the PentAGI logical identities,
-- exact censuses, stage-level stop denominator, and deterministic closure
-- authority that those generic tables do not own.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION unified_investigation_runtime_head_sha256(
    authority_id UUID,
    run_state TEXT,
    admission_open BOOLEAN,
    stop_epoch BIGINT,
    change_seq BIGINT,
    head_version BIGINT
)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT 'sha256:' || encode(
        digest(
            convert_to(
                concat_ws(':',
                    'golish.investigation_run_head.v1', authority_id::TEXT,
                    run_state, admission_open::TEXT, stop_epoch::TEXT,
                    change_seq::TEXT, head_version::TEXT
                ),
                'UTF8'
            ),
            'sha256'
        ),
        'hex'
    )
$$;

CREATE FUNCTION unified_investigation_work_state_terminal(state TEXT)
RETURNS BOOLEAN
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT state IN (
        'completed','cancelled','blocked','residual','recovery_required',
        'fixed_point','superseded'
    )
$$;

CREATE TABLE investigation_run_heads (
    authority_id UUID PRIMARY KEY,
    stable_start_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL CHECK (
        btrim(owning_stage_run_request_id)<>'' AND length(owning_stage_run_request_id)<=512
    ),
    scope_snapshot_id UUID NOT NULL,
    run_state TEXT NOT NULL CHECK (
        run_state IN ('running','stop_pending','draining','closed','abandoned')
    ),
    admission_open BOOLEAN NOT NULL,
    stop_epoch BIGINT NOT NULL CHECK (stop_epoch>=0),
    change_seq BIGINT NOT NULL CHECK (change_seq>=0),
    head_version BIGINT NOT NULL CHECK (head_version>=0),
    head_sha256 TEXT NOT NULL CHECK (head_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    latest_event_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        head_sha256=unified_investigation_runtime_head_sha256(
            authority_id,run_state,admission_open,stop_epoch,change_seq,head_version
        )
    ),
    CHECK (
        (run_state='running' AND admission_open)
        OR (run_state<>'running' AND NOT admission_open)
    ),
    UNIQUE(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ),
    UNIQUE(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id
    ),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_stage_run_authorities(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_run_state_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL REFERENCES investigation_run_heads(authority_id) ON DELETE RESTRICT,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>0),
    expected_head_sha256 TEXT NOT NULL CHECK (expected_head_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    from_state TEXT NOT NULL CHECK (
        from_state IN ('running','stop_pending','draining','closed','abandoned')
    ),
    to_state TEXT NOT NULL CHECK (
        to_state IN ('stop_pending','draining','closed','abandoned')
    ),
    stop_epoch BIGINT NOT NULL CHECK (stop_epoch>=0),
    change_seq BIGINT NOT NULL CHECK (change_seq>=0),
    event_sha256 TEXT NOT NULL CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(authority_id,event_ordinal),
    UNIQUE(event_id,authority_id,event_ordinal)
);

ALTER TABLE investigation_run_heads
    ADD CONSTRAINT investigation_run_heads_latest_event_fk
    FOREIGN KEY(latest_event_id,authority_id,head_version)
    REFERENCES investigation_run_state_events(event_id,authority_id,event_ordinal)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION unified_investigation_guard_run_head_write()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF current_setting('golish.investigation_run_head_write',TRUE) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'INVESTIGATION_RUN_HEAD_EVENT_ONLY' USING ERRCODE='23514';
    END IF;
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_RUN_HEAD_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_run_heads_event_only
BEFORE INSERT OR UPDATE OR DELETE ON investigation_run_heads
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_run_head_write();

CREATE TRIGGER investigation_run_state_events_append_only
BEFORE UPDATE OR DELETE ON investigation_run_state_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION register_investigation_run_v1(
    p_authority_id UUID,
    p_stable_start_request_id UUID,
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_owning_stage_run_request_id TEXT,
    p_scope_snapshot_id UUID,
    p_initial_change_seq BIGINT
)
RETURNS investigation_run_heads
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_run_heads%ROWTYPE;
    result investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO existing FROM investigation_run_heads
     WHERE stable_start_request_id=p_stable_start_request_id;
    IF FOUND THEN
        IF ROW(existing.authority_id,existing.operation_id,existing.stage_execution_id,
               existing.owning_stage_run_request_id,existing.scope_snapshot_id,
               existing.change_seq)
           IS DISTINCT FROM
           ROW(p_authority_id,p_operation_id,p_stage_execution_id,
               p_owning_stage_run_request_id,p_scope_snapshot_id,p_initial_change_seq)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_RUN_START_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    IF p_initial_change_seq<0 THEN
        RAISE EXCEPTION 'INVESTIGATION_RUN_CHANGE_SEQ_INVALID' USING ERRCODE='23514';
    END IF;
    PERFORM set_config('golish.investigation_run_head_write','on',TRUE);
    INSERT INTO investigation_run_heads(
        authority_id,stable_start_request_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
        stop_epoch,change_seq,head_version,head_sha256
    ) VALUES(
        p_authority_id,p_stable_start_request_id,p_operation_id,p_stage_execution_id,
        p_owning_stage_run_request_id,p_scope_snapshot_id,'running',TRUE,
        0,p_initial_change_seq,0,
        unified_investigation_runtime_head_sha256(
            p_authority_id,'running',TRUE,0,p_initial_change_seq,0
        )
    ) RETURNING * INTO result;
    PERFORM set_config('golish.investigation_run_head_write','off',TRUE);
    RETURN result;
END;
$$;

-- Every async writer must register its stage-owned durable work before it can
-- dispatch.  This inventory is the one exact stop denominator; category-local
-- tables remain their own business authority.
CREATE TABLE investigation_run_work_items (
    work_id UUID PRIMARY KEY,
    stable_work_key_sha256 TEXT NOT NULL CHECK (stable_work_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    work_kind TEXT NOT NULL CHECK (work_kind IN (
        'analysis','read_session','query','enrichment','outbox','verification_task',
        'pentagi_subtask','worker_request','campaign','prepared_action',
        'action_execution','fact_delta','consolidation'
    )),
    external_identity_sha256 TEXT NOT NULL CHECK (external_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    current_state TEXT NOT NULL CHECK (current_state IN (
        'queued','running','waiting_authorization','unknown','stop_pending','draining',
        'completed','cancelled','blocked','residual','recovery_required',
        'fixed_point','superseded'
    )),
    observed_stop_epoch BIGINT NOT NULL CHECK (observed_stop_epoch>=0),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK (head_version>=0),
    latest_event_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(authority_id,stable_work_key_sha256),
    UNIQUE(work_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_run_heads(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_run_unit_id,operation_id,stage_execution_id,organization_id
    ) REFERENCES stage_run_units(
        id,operation_id,stage_execution_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_run_work_state_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    work_id UUID NOT NULL REFERENCES investigation_run_work_items(work_id) ON DELETE RESTRICT,
    expected_head_version BIGINT NOT NULL CHECK (expected_head_version>=0),
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>0),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    observed_stop_epoch BIGINT NOT NULL CHECK (observed_stop_epoch>=0),
    reason_code TEXT NOT NULL CHECK (btrim(reason_code)<>''),
    event_sha256 TEXT NOT NULL CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(work_id,event_ordinal),
    UNIQUE(event_id,work_id,event_ordinal)
);

ALTER TABLE investigation_run_work_items
    ADD CONSTRAINT investigation_run_work_latest_event_fk
    FOREIGN KEY(latest_event_id,work_id,head_version)
    REFERENCES investigation_run_work_state_events(event_id,work_id,event_ordinal)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION unified_investigation_work_transition_allowed(previous TEXT, next TEXT)
RETURNS BOOLEAN
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT (previous,next) IN (
        ('queued','running'),('queued','cancelled'),('queued','blocked'),
        ('running','waiting_authorization'),('running','stop_pending'),
        ('running','completed'),('running','blocked'),('running','residual'),
        ('running','recovery_required'),('running','unknown'),
        ('waiting_authorization','running'),('waiting_authorization','stop_pending'),
        ('waiting_authorization','cancelled'),('waiting_authorization','blocked'),
        ('unknown','recovery_required'),('unknown','completed'),
        ('stop_pending','draining'),('stop_pending','cancelled'),
        ('draining','cancelled'),('draining','blocked'),('draining','residual'),
        ('draining','recovery_required'),('draining','completed'),
        ('recovery_required','blocked'),('recovery_required','completed')
    )
$$;

CREATE FUNCTION unified_investigation_guard_work_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE head investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=NEW.authority_id FOR SHARE;
    IF head.run_state<>'running' OR NOT head.admission_open
       OR NEW.observed_stop_epoch<>head.stop_epoch
    THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_ADMISSION_CLOSED' USING ERRCODE='23514';
    END IF;
    IF ROW(NEW.operation_id,NEW.stage_execution_id,NEW.owning_stage_run_request_id,
           NEW.scope_snapshot_id)
       IS DISTINCT FROM
       ROW(head.operation_id,head.stage_execution_id,head.owning_stage_run_request_id,
           head.scope_snapshot_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_STAGE_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_run_work_items_admission_guard
BEFORE INSERT ON investigation_run_work_items
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_work_insert();

CREATE FUNCTION unified_investigation_guard_work_write()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF current_setting('golish.investigation_work_event_apply',TRUE) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_EVENT_ONLY' USING ERRCODE='23514';
    END IF;
    IF ROW(NEW.work_id,NEW.stable_work_key_sha256,NEW.authority_id,NEW.operation_id,
           NEW.stage_execution_id,NEW.owning_stage_run_request_id,NEW.stage_run_unit_id,
           NEW.scope_snapshot_id,NEW.organization_id,NEW.work_kind,
           NEW.external_identity_sha256,NEW.created_at)
       IS DISTINCT FROM
       ROW(OLD.work_id,OLD.stable_work_key_sha256,OLD.authority_id,OLD.operation_id,
           OLD.stage_execution_id,OLD.owning_stage_run_request_id,OLD.stage_run_unit_id,
           OLD.scope_snapshot_id,OLD.organization_id,OLD.work_kind,
           OLD.external_identity_sha256,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_WORK_IDENTITY_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_run_work_items_event_only
BEFORE UPDATE OR DELETE ON investigation_run_work_items
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_work_write();

CREATE FUNCTION unified_investigation_apply_work_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    work investigation_run_work_items%ROWTYPE;
    run investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO STRICT work FROM investigation_run_work_items
     WHERE work_id=NEW.work_id FOR UPDATE;
    SELECT * INTO STRICT run FROM investigation_run_heads
     WHERE authority_id=work.authority_id FOR SHARE;
    IF NEW.expected_head_version<>work.head_version
       OR NEW.event_ordinal<>work.head_version+1
       OR NEW.from_state<>work.current_state
       OR NOT unified_investigation_work_transition_allowed(work.current_state,NEW.to_state)
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

CREATE TRIGGER investigation_run_work_state_events_apply
BEFORE INSERT ON investigation_run_work_state_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_apply_work_event();
CREATE TRIGGER investigation_run_work_state_events_append_only
BEFORE UPDATE OR DELETE ON investigation_run_work_state_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- PentAGI task plan authority. One existing StageTeam plan remains the Unit
-- governance envelope; many tagged PentAGI plans may execute beneath it.
CREATE TABLE investigation_pentagi_task_plans (
    task_plan_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('analysis_attempt','verification_task')),
    subject_id UUID NOT NULL,
    subject_fingerprint_sha256 TEXT NOT NULL CHECK (subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    task_plan_version INTEGER NOT NULL CHECK (task_plan_version>0),
    task_plan_sha256 TEXT NOT NULL CHECK (task_plan_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    allowed_role_catalog JSONB NOT NULL CHECK (stage_team_json_string_array_is_valid(allowed_role_catalog)),
    cognitive_tool_envelope_sha256 TEXT NOT NULL CHECK (cognitive_tool_envelope_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','sealed')),
    subtask_count BIGINT,
    subtask_set_sha256 TEXT CHECK (subtask_set_sha256 IS NULL OR subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CHECK (
        (status='open' AND subtask_count IS NULL AND subtask_set_sha256 IS NULL AND sealed_at IS NULL)
        OR (status='sealed' AND subtask_count IS NOT NULL AND subtask_count>0
            AND subtask_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    UNIQUE(subject_kind,subject_id,subject_fingerprint_sha256,task_plan_version),
    UNIQUE(task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id),
    UNIQUE(task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_run_heads(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) REFERENCES stage_team_plans(
        id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_run_unit_id,operation_id,stage_execution_id,organization_id
    ) REFERENCES stage_run_units(
        id,operation_id,stage_execution_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE pentagi_task_run_requests (
    run_request_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('analysis_attempt','verification_task')),
    subject_id UUID NOT NULL,
    subject_fingerprint_sha256 TEXT NOT NULL CHECK (subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    request_sha256 TEXT NOT NULL CHECK (request_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(subject_kind,subject_id,subject_fingerprint_sha256),
    FOREIGN KEY(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_pentagi_task_plans(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id
    ) REFERENCES investigation_run_heads(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION unified_investigation_guard_pentagi_plan_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE head investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=NEW.authority_id FOR SHARE;
    IF head.run_state<>'running' OR NOT head.admission_open
       OR ROW(NEW.operation_id,NEW.stage_execution_id,NEW.owning_stage_run_request_id,
              NEW.scope_snapshot_id)
          IS DISTINCT FROM
          ROW(head.operation_id,head.stage_execution_id,head.owning_stage_run_request_id,
              head.scope_snapshot_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_ADMISSION_CLOSED' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_task_plans_admission_guard
BEFORE INSERT ON investigation_pentagi_task_plans
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_pentagi_plan_insert();

CREATE TABLE investigation_pentagi_subtasks (
    subtask_id UUID PRIMARY KEY,
    task_plan_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    subtask_ordinal INTEGER NOT NULL CHECK (subtask_ordinal>=0),
    label TEXT NOT NULL CHECK (btrim(label)<>'' AND length(label)<=512),
    runnable BOOLEAN NOT NULL,
    input_manifest_sha256 TEXT NOT NULL CHECK (input_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    expected_output_schema TEXT NOT NULL CHECK (btrim(expected_output_schema)<>''),
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_plan_id,subtask_ordinal),
    UNIQUE(task_plan_id,member_sha256),
    UNIQUE(subtask_id,task_plan_id),
    FOREIGN KEY(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_pentagi_task_plans(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION unified_investigation_guard_open_pentagi_plan_child()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan investigation_pentagi_task_plans%ROWTYPE;
    head investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=plan.authority_id FOR SHARE;
    IF TG_TABLE_NAME='investigation_pentagi_pipeline_events' THEN
        IF head.run_state='closed' THEN
            RAISE EXCEPTION 'INVESTIGATION_PENTAGI_LATE_TERMINAL_RECEIPT' USING ERRCODE='23514';
        END IF;
    ELSIF head.run_state<>'running' OR NOT head.admission_open THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_CHILD_ADMISSION_CLOSED' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_subtasks_open_plan
BEFORE INSERT ON investigation_pentagi_subtasks
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_open_pentagi_plan_child();
CREATE TRIGGER investigation_pentagi_subtasks_append_only
BEFORE UPDATE OR DELETE ON investigation_pentagi_subtasks
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE pentagi_logical_dispatch_receipts (
    dispatch_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    logical_dispatch_key_sha256 TEXT NOT NULL UNIQUE CHECK (logical_dispatch_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    task_plan_id UUID NOT NULL,
    subtask_id UUID,
    parent_dispatch_receipt_id UUID,
    dispatch_ordinal INTEGER NOT NULL CHECK (dispatch_ordinal>=0),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('primary','worker','nested_worker')),
    stage_work_item_id UUID NOT NULL,
    stage_worker_request_id UUID,
    worker_run_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    transcript_request_id TEXT NOT NULL CHECK (btrim(transcript_request_id)<>''),
    parent_actor_transcript_request_id TEXT,
    parent_dispatch_tool_request_id TEXT,
    snapshot_sha256 TEXT NOT NULL CHECK (snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (actor_kind='primary' AND subtask_id IS NULL
            AND parent_dispatch_receipt_id IS NULL AND stage_worker_request_id IS NULL
            AND parent_actor_transcript_request_id IS NULL
            AND parent_dispatch_tool_request_id IS NULL)
        OR (actor_kind='worker' AND subtask_id IS NOT NULL
            AND parent_dispatch_receipt_id IS NOT NULL
            AND stage_worker_request_id IS NOT NULL
            AND parent_actor_transcript_request_id IS NOT NULL
            AND parent_dispatch_tool_request_id IS NOT NULL)
        OR (actor_kind='nested_worker' AND subtask_id IS NOT NULL
            AND parent_dispatch_receipt_id IS NOT NULL
            AND stage_worker_request_id IS NOT NULL
            AND parent_actor_transcript_request_id IS NOT NULL
            AND parent_dispatch_tool_request_id IS NOT NULL)
    ),
    UNIQUE(
        dispatch_receipt_id,task_plan_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ),
    UNIQUE(dispatch_receipt_id,task_plan_id),
    FOREIGN KEY(subtask_id,task_plan_id)
        REFERENCES investigation_pentagi_subtasks(subtask_id,task_plan_id) ON DELETE RESTRICT,
    FOREIGN KEY(parent_dispatch_receipt_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES stage_work_items(
        id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_worker_request_id)
        REFERENCES stage_worker_requests(id) ON DELETE RESTRICT,
    FOREIGN KEY(
        worker_run_id,stage_work_item_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ) REFERENCES stage_worker_runs(
        id,work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_pentagi_task_plans(
        task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX pentagi_one_primary_per_task_plan
    ON pentagi_logical_dispatch_receipts(task_plan_id)
    WHERE actor_kind='primary';
CREATE UNIQUE INDEX pentagi_logical_dispatch_identity_unique
    ON pentagi_logical_dispatch_receipts(
        task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal
    ) NULLS NOT DISTINCT;

CREATE FUNCTION unified_investigation_guard_dispatch_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan investigation_pentagi_task_plans%ROWTYPE;
    head investigation_run_heads%ROWTYPE;
    parent pentagi_logical_dispatch_receipts%ROWTYPE;
    worker stage_worker_runs%ROWTYPE;
BEGIN
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=plan.authority_id FOR SHARE;
    IF head.run_state<>'running' OR NOT head.admission_open THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_DISPATCH_ADMISSION_CLOSED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT worker FROM stage_worker_runs
     WHERE id=NEW.worker_run_id FOR SHARE;
    IF worker.parent_request_id IS DISTINCT FROM NEW.transcript_request_id THEN
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

CREATE TRIGGER pentagi_logical_dispatch_receipts_contract
BEFORE INSERT ON pentagi_logical_dispatch_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_dispatch_receipt();
CREATE TRIGGER pentagi_logical_dispatch_receipts_append_only
BEFORE UPDATE OR DELETE ON pentagi_logical_dispatch_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE pentagi_logical_dispatch_attempts (
    dispatch_attempt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    dispatch_receipt_id UUID NOT NULL REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id) ON DELETE RESTRICT,
    attempt_epoch BIGINT NOT NULL CHECK (attempt_epoch>=0),
    lease_token UUID NOT NULL,
    fence_sha256 TEXT NOT NULL CHECK (fence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    outcome TEXT NOT NULL CHECK (outcome IN (
        'completed','blocked','residual','recovery_required','unknown_held'
    )),
    result_sha256 TEXT NOT NULL CHECK (result_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(dispatch_receipt_id,attempt_epoch),
    UNIQUE(dispatch_attempt_id,dispatch_receipt_id)
);

CREATE TRIGGER pentagi_logical_dispatch_attempts_append_only
BEFORE UPDATE OR DELETE ON pentagi_logical_dispatch_attempts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_pentagi_pipeline_events (
    pipeline_event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL REFERENCES investigation_pentagi_task_plans(task_plan_id) ON DELETE RESTRICT,
    subtask_id UUID,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>=0),
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'generator_sealed','refiner_patch','reflector_attempt','result_barrier','primary_synthesis'
    )),
    actor_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    parent_dispatch_receipt_id UUID NOT NULL REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id) ON DELETE RESTRICT,
    event_sha256 TEXT NOT NULL CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_plan_id,event_ordinal),
    FOREIGN KEY(subtask_id,task_plan_id)
        REFERENCES investigation_pentagi_subtasks(subtask_id,task_plan_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_pentagi_pipeline_events_open_plan
BEFORE INSERT ON investigation_pentagi_pipeline_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_open_pentagi_plan_child();
CREATE TRIGGER investigation_pentagi_pipeline_events_append_only
BEFORE UPDATE OR DELETE ON investigation_pentagi_pipeline_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_pentagi_delegation_census_seals (
    census_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL UNIQUE REFERENCES investigation_pentagi_task_plans(task_plan_id) ON DELETE RESTRICT,
    primary_dispatch_receipt_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL UNIQUE,
    runnable_subtask_count BIGINT NOT NULL CHECK (runnable_subtask_count>=0),
    runnable_subtask_set_sha256 TEXT NOT NULL CHECK (runnable_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dispatch_count BIGINT NOT NULL CHECK (dispatch_count>0),
    dispatch_set_sha256 TEXT NOT NULL CHECK (dispatch_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    pipeline_event_count BIGINT NOT NULL CHECK (pipeline_event_count>=0),
    pipeline_event_set_sha256 TEXT NOT NULL CHECK (pipeline_event_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    seal_sha256 TEXT NOT NULL CHECK (seal_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(primary_dispatch_receipt_id,task_plan_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id,task_plan_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(primary_worker_run_id)
        REFERENCES stage_worker_runs(id) ON DELETE RESTRICT
);

CREATE FUNCTION unified_investigation_guard_delegation_census_seal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    primary_count BIGINT;
    actual_primary_receipt UUID;
    actual_primary_worker UUID;
    runnable_count BIGINT;
    runnable_hash TEXT;
    dispatch_count BIGINT;
    dispatch_hash TEXT;
    pipeline_count BIGINT;
    pipeline_hash TEXT;
BEGIN
    PERFORM 1 FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),
           (array_agg(dispatch_receipt_id ORDER BY dispatch_receipt_id))[1],
           (array_agg(worker_run_id ORDER BY dispatch_receipt_id))[1]
      INTO primary_count,actual_primary_receipt,actual_primary_worker
      FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=NEW.task_plan_id AND actor_kind='primary';
    IF primary_count<>1
       OR NEW.primary_dispatch_receipt_id<>actual_primary_receipt
       OR NEW.primary_worker_run_id<>actual_primary_worker
    THEN
        RAISE EXCEPTION 'PENTAGI_CENSUS_REQUIRES_SINGLE_PRIMARY' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM investigation_pentagi_subtasks subtask
         WHERE subtask.task_plan_id=NEW.task_plan_id AND subtask.runnable
           AND NOT EXISTS(
               SELECT 1 FROM pentagi_logical_dispatch_receipts dispatch
                WHERE dispatch.task_plan_id=NEW.task_plan_id
                  AND dispatch.subtask_id=subtask.subtask_id
                  AND dispatch.actor_kind IN ('worker','nested_worker')
                  AND dispatch.worker_run_id<>actual_primary_worker
           )
    ) THEN
        RAISE EXCEPTION 'PENTAGI_RUNNABLE_SUBTASK_REQUIRES_DISTINCT_WORKER' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM pentagi_logical_dispatch_receipts dispatch
         WHERE dispatch.task_plan_id=NEW.task_plan_id
           AND NOT EXISTS(
               SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
                WHERE attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                  AND attempt.outcome<>'unknown_held'
           )
    ) OR EXISTS(
        SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
        JOIN pentagi_logical_dispatch_receipts dispatch
          ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
        WHERE dispatch.task_plan_id=NEW.task_plan_id
          AND attempt.outcome='unknown_held'
    ) THEN
        RAISE EXCEPTION 'PENTAGI_CENSUS_HAS_UNSETTLED_DISPATCH' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_runnable_subtasks.v1',
               COALESCE(array_agg(member_sha256 ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           )
      INTO runnable_count,runnable_hash
      FROM investigation_pentagi_subtasks
     WHERE task_plan_id=NEW.task_plan_id AND runnable;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'pentagi_logical_dispatch_receipts.v1',
               COALESCE(array_agg(receipt_sha256 ORDER BY dispatch_receipt_id),ARRAY[]::TEXT[])
           )
      INTO dispatch_count,dispatch_hash
      FROM pentagi_logical_dispatch_receipts WHERE task_plan_id=NEW.task_plan_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_pipeline_events.v1',
               COALESCE(array_agg(event_sha256 ORDER BY pipeline_event_id),ARRAY[]::TEXT[])
           )
      INTO pipeline_count,pipeline_hash
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=NEW.task_plan_id;
    IF ROW(NEW.runnable_subtask_count,NEW.runnable_subtask_set_sha256,
           NEW.dispatch_count,NEW.dispatch_set_sha256,
           NEW.pipeline_event_count,NEW.pipeline_event_set_sha256)
       IS DISTINCT FROM
       ROW(runnable_count,runnable_hash,dispatch_count,dispatch_hash,pipeline_count,pipeline_hash)
    THEN
        RAISE EXCEPTION 'PENTAGI_DELEGATION_CENSUS_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_delegation_census_contract
BEFORE INSERT ON investigation_pentagi_delegation_census_seals
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_delegation_census_seal();
CREATE TRIGGER investigation_pentagi_delegation_census_append_only
BEFORE UPDATE OR DELETE ON investigation_pentagi_delegation_census_seals
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION unified_investigation_guard_pentagi_plan_transition()
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
       OR ROW(NEW.task_plan_id,NEW.stable_request_id,NEW.authority_id,NEW.stage_team_plan_id,
              NEW.operation_id,NEW.stage_execution_id,NEW.owning_stage_run_request_id,
              NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
              NEW.subject_kind,NEW.subject_id,NEW.subject_fingerprint_sha256,
              NEW.task_plan_version,NEW.task_plan_sha256,NEW.allowed_role_catalog,
              NEW.cognitive_tool_envelope_sha256,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.task_plan_id,OLD.stable_request_id,OLD.authority_id,OLD.stage_team_plan_id,
              OLD.operation_id,OLD.stage_execution_id,OLD.owning_stage_run_request_id,
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
            SELECT 1 FROM pentagi_task_run_requests
             WHERE task_plan_id=NEW.task_plan_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_EXACT_SEAL_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_pentagi_task_plans_contract
BEFORE UPDATE OR DELETE ON investigation_pentagi_task_plans
FOR EACH ROW EXECUTE FUNCTION unified_investigation_guard_pentagi_plan_transition();

CREATE TRIGGER pentagi_task_run_requests_append_only
BEFORE UPDATE OR DELETE ON pentagi_task_run_requests
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- Stop freezes the exact inventory while holding the run head lock. Inserts of
-- new work take a conflicting share lock and therefore cannot cross the fence.
CREATE TABLE investigation_stop_intents (
    stop_intent_id UUID PRIMARY KEY,
    idempotency_key UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL REFERENCES investigation_run_heads(authority_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    expected_run_head_sha256 TEXT NOT NULL CHECK (expected_run_head_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    expected_change_seq BIGINT NOT NULL CHECK (expected_change_seq>=0),
    stop_epoch BIGINT NOT NULL CHECK (stop_epoch>0),
    frozen_work_count BIGINT NOT NULL CHECK (frozen_work_count>=0),
    frozen_work_set_sha256 TEXT NOT NULL CHECK (frozen_work_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(authority_id,stop_epoch),
    UNIQUE(stop_intent_id,authority_id,stop_epoch),
    FOREIGN KEY(authority_id,operation_id,stage_execution_id,owning_stage_run_request_id)
        REFERENCES investigation_run_heads(
            authority_id,operation_id,stage_execution_id,owning_stage_run_request_id
        ) ON DELETE RESTRICT
);

CREATE TABLE investigation_stop_work_members (
    stop_member_id UUID PRIMARY KEY,
    stop_intent_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    stop_epoch BIGINT NOT NULL,
    work_id UUID NOT NULL,
    work_kind TEXT NOT NULL,
    external_identity_sha256 TEXT NOT NULL CHECK (external_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    frozen_state TEXT NOT NULL,
    frozen_head_version BIGINT NOT NULL CHECK (frozen_head_version>=0),
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(stop_intent_id,work_id),
    UNIQUE(stop_intent_id,member_sha256),
    FOREIGN KEY(stop_intent_id,authority_id,stop_epoch)
        REFERENCES investigation_stop_intents(stop_intent_id,authority_id,stop_epoch)
        ON DELETE RESTRICT,
    FOREIGN KEY(work_id)
        REFERENCES investigation_run_work_items(work_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_stop_intents_append_only
BEFORE UPDATE OR DELETE ON investigation_stop_intents
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER investigation_stop_work_members_append_only
BEFORE UPDATE OR DELETE ON investigation_stop_work_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION investigation_request_stop_v1(
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
    SELECT * INTO STRICT head FROM investigation_run_heads
     WHERE authority_id=p_authority_id FOR UPDATE;
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
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_stop_open_work.v1',
               COALESCE(array_agg(
                   'sha256:' || encode(digest(convert_to(
                       concat_ws(':','golish.investigation_stop_member.v1',work_id::TEXT,
                           external_identity_sha256,current_state,head_version::TEXT),
                       'UTF8'),'sha256'),'hex')
                   ORDER BY work_kind,work_id
               ),ARRAY[]::TEXT[])
           )
      INTO work_count,work_hash
      FROM investigation_run_work_items
     WHERE authority_id=p_authority_id
       AND NOT unified_investigation_work_state_terminal(current_state);
    receipt_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_stop_intent.v1',p_stop_intent_id::TEXT,
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

CREATE TABLE investigation_run_closures (
    closure_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL UNIQUE REFERENCES investigation_run_heads(authority_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stop_intent_id UUID NOT NULL UNIQUE,
    stop_epoch BIGINT NOT NULL CHECK (stop_epoch>0),
    disposition TEXT NOT NULL CHECK (disposition IN ('pass','pass_with_gaps','stopped')),
    work_count BIGINT NOT NULL CHECK (work_count>=0),
    work_set_sha256 TEXT NOT NULL CHECK (work_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    task_plan_count BIGINT NOT NULL CHECK (task_plan_count>=0),
    task_plan_set_sha256 TEXT NOT NULL CHECK (task_plan_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dispatch_count BIGINT NOT NULL CHECK (dispatch_count>=0),
    dispatch_set_sha256 TEXT NOT NULL CHECK (dispatch_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_sha256 TEXT NOT NULL CHECK (residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    closure_sha256 TEXT NOT NULL CHECK (closure_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    closed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(stop_intent_id,authority_id,stop_epoch)
        REFERENCES investigation_stop_intents(stop_intent_id,authority_id,stop_epoch)
        ON DELETE RESTRICT,
    FOREIGN KEY(authority_id,operation_id,stage_execution_id,owning_stage_run_request_id)
        REFERENCES investigation_run_heads(
            authority_id,operation_id,stage_execution_id,owning_stage_run_request_id
        ) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_run_closures_append_only
BEFORE UPDATE OR DELETE ON investigation_run_closures
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION seal_investigation_run_closure_v1(
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
               'investigation_run_closure_work.v1',
               COALESCE(array_agg(
                   work_id::TEXT || ':' || external_identity_sha256 || ':' ||
                   current_state || ':' || head_version::TEXT ORDER BY work_kind,work_id
               ),ARRAY[]::TEXT[])
           )
      INTO work_count,work_hash
      FROM investigation_run_work_items WHERE authority_id=p_authority_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_run_closure_task_plans.v1',
               COALESCE(array_agg(task_plan_sha256 ORDER BY task_plan_id),ARRAY[]::TEXT[])
           )
      INTO plan_count,plan_hash
      FROM investigation_pentagi_task_plans WHERE authority_id=p_authority_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_run_closure_dispatches.v1',
               COALESCE(array_agg(dispatch.receipt_sha256 ORDER BY dispatch.dispatch_receipt_id),ARRAY[]::TEXT[])
           )
      INTO dispatch_count,dispatch_hash
      FROM pentagi_logical_dispatch_receipts dispatch
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
     WHERE plan.authority_id=p_authority_id;
    closure_hash := 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_run_closure.v1',p_closure_id::TEXT,
            p_authority_id::TEXT,head.stop_epoch::TEXT,p_disposition,work_hash,
            plan_hash,dispatch_hash,p_residual_set_sha256),
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
    );
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
    SELECT * INTO STRICT result FROM investigation_run_closures
     WHERE closure_id=p_closure_id;
    RETURN result;
END;
$$;
