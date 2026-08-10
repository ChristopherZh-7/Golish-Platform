-- Unified Investigation persistence authority.
--
-- This is deliberately additive.  It binds the one visible Investigation
-- stage execution/request to exact per-organization read partitions, automatic
-- verification admission/tasks, objective assignment/outcome closure, and a
-- CAS fuel ledger.  Cognitive roles and Operators remain outside these truth
-- tables; no table below grants a tool or action authority.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION unified_investigation_exact_set_hash(kind TEXT, members TEXT[])
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT 'sha256:' || encode(
        digest(
            convert_to(
                kind || E'\n' || COALESCE(array_to_string(members, E'\n'), ''),
                'UTF8'
            ),
            'sha256'
        ),
        'hex'
    )
$$;

CREATE FUNCTION unified_investigation_reject_append_only()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'UNIFIED_INVESTIGATION_APPEND_ONLY' USING ERRCODE='23514';
END;
$$;

-- One host-owned request identity for one real Investigation stage execution.
CREATE TABLE investigation_stage_run_authorities (
    authority_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL CHECK (
        btrim(owning_stage_run_request_id)<>''
        AND length(owning_stage_run_request_id)<=512
    ),
    scope_snapshot_id UUID NOT NULL,
    stage_kind TEXT NOT NULL DEFAULT 'investigation' CHECK (stage_kind='investigation'),
    contract_version TEXT NOT NULL DEFAULT 'unified_investigation_authority.v1'
        CHECK (contract_version='unified_investigation_authority.v1'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(stage_execution_id),
    UNIQUE(operation_id,owning_stage_run_request_id),
    UNIQUE(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ),
    FOREIGN KEY(stage_execution_id,operation_id,stage_kind)
        REFERENCES stage_runs(id,operation_id,stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,operation_id)
        REFERENCES operation_org_scope_snapshots(id,operation_id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_stage_run_authority_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1 FROM operation_org_scope_snapshots snapshot
         WHERE snapshot.id=NEW.scope_snapshot_id
           AND snapshot.operation_id=NEW.operation_id
           AND snapshot.sealed_at IS NOT NULL
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_STAGE_AUTHORITY_REQUIRES_SEALED_SCOPE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_stage_run_authorities_sealed_scope
BEFORE INSERT ON investigation_stage_run_authorities
FOR EACH ROW EXECUTE FUNCTION investigation_guard_stage_run_authority_insert();

CREATE TRIGGER investigation_stage_run_authorities_append_only
BEFORE UPDATE OR DELETE ON investigation_stage_run_authorities
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- S1 is immutable and scoped to one exact organization Unit.  The body stays
-- in the partitioned context store; only exact census/hash authority is kept.
CREATE TABLE investigation_analysis_snapshot_authorities (
    snapshot_id UUID PRIMARY KEY,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL DEFAULT 'investigation' CHECK (stage_kind='investigation'),
    snapshot_sha256 TEXT NOT NULL CHECK (snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    context_item_count BIGINT NOT NULL CHECK (context_item_count>=0),
    context_item_set_sha256 TEXT NOT NULL CHECK (context_item_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    methodology_hit_count BIGINT NOT NULL CHECK (methodology_hit_count>=0),
    methodology_result_set_sha256 TEXT NOT NULL CHECK (methodology_result_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    omission_count BIGINT NOT NULL CHECK (omission_count>=0),
    omission_set_sha256 TEXT NOT NULL CHECK (omission_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(
        snapshot_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id
    ),
    UNIQUE(stage_run_unit_id,snapshot_sha256),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_stage_run_authorities(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_run_unit_id,operation_id,stage_execution_id,organization_id,stage_kind
    ) REFERENCES stage_run_units(
        id,operation_id,stage_execution_id,organization_id,stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_analysis_snapshot_authorities_append_only
BEFORE UPDATE OR DELETE ON investigation_analysis_snapshot_authorities
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_main_session_sets (
    session_set_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    session_set_ordinal BIGINT NOT NULL CHECK (session_set_ordinal>=0),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','sealed')),
    member_count BIGINT,
    member_set_sha256 TEXT CHECK (
        member_set_sha256 IS NULL OR member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CHECK (
        (status='open' AND member_count IS NULL AND member_set_sha256 IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND member_count IS NOT NULL AND member_count>0
            AND member_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    UNIQUE(authority_id,session_set_ordinal),
    UNIQUE(
        session_set_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_stage_run_authorities(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_main_read_sessions (
    main_read_session_id UUID PRIMARY KEY,
    session_set_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    snapshot_sha256 TEXT NOT NULL CHECK (snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    context_chain_id UUID NOT NULL UNIQUE,
    transcript_partition_id UUID NOT NULL UNIQUE,
    session_contract_version TEXT NOT NULL
        CHECK (session_contract_version='investigation_main_organization_read_session.v1'),
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (context_chain_id<>transcript_partition_id),
    UNIQUE(session_set_id,stage_run_unit_id),
    UNIQUE(session_set_id,organization_id),
    UNIQUE(session_set_id,snapshot_id),
    UNIQUE(main_read_session_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id),
    FOREIGN KEY(
        session_set_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_main_session_sets(
        session_set_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        snapshot_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id
    ) REFERENCES investigation_analysis_snapshot_authorities(
        snapshot_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_main_read_session_receipts (
    receipt_id UUID PRIMARY KEY,
    main_read_session_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    snapshot_sha256 TEXT NOT NULL CHECK (snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    context_item_count BIGINT NOT NULL CHECK (context_item_count>=0),
    context_item_set_sha256 TEXT NOT NULL CHECK (context_item_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    methodology_hit_count BIGINT NOT NULL CHECK (methodology_hit_count>=0),
    methodology_result_set_sha256 TEXT NOT NULL CHECK (methodology_result_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    omission_count BIGINT NOT NULL CHECK (omission_count>=0),
    omission_set_sha256 TEXT NOT NULL CHECK (omission_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(receipt_id,main_read_session_id,receipt_sha256),
    FOREIGN KEY(
        main_read_session_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_main_read_sessions(
        main_read_session_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_main_session_child_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1 FROM investigation_main_session_sets set_row
         WHERE set_row.session_set_id=NEW.session_set_id AND set_row.status='open'
         FOR UPDATE
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_main_read_sessions_open_set_guard
BEFORE INSERT ON investigation_main_read_sessions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_main_session_child_insert();

CREATE FUNCTION investigation_guard_main_receipt_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    session_row investigation_main_read_sessions%ROWTYPE;
    snapshot_row investigation_analysis_snapshot_authorities%ROWTYPE;
BEGIN
    SELECT * INTO STRICT session_row
      FROM investigation_main_read_sessions
     WHERE main_read_session_id=NEW.main_read_session_id FOR SHARE;
    PERFORM 1 FROM investigation_main_session_sets
     WHERE session_set_id=session_row.session_set_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT snapshot_row
      FROM investigation_analysis_snapshot_authorities
     WHERE snapshot_id=session_row.snapshot_id FOR SHARE;
    IF ROW(
        NEW.snapshot_id,NEW.snapshot_sha256,
        NEW.context_item_count,NEW.context_item_set_sha256,
        NEW.methodology_hit_count,NEW.methodology_result_set_sha256,
        NEW.omission_count,NEW.omission_set_sha256
    ) IS DISTINCT FROM ROW(
        snapshot_row.snapshot_id,snapshot_row.snapshot_sha256,
        snapshot_row.context_item_count,snapshot_row.context_item_set_sha256,
        snapshot_row.methodology_hit_count,snapshot_row.methodology_result_set_sha256,
        snapshot_row.omission_count,snapshot_row.omission_set_sha256
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_READ_RECEIPT_SNAPSHOT_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_main_read_session_receipts_guard
BEFORE INSERT ON investigation_main_read_session_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_main_receipt_insert();

CREATE FUNCTION investigation_guard_main_session_set_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
    receipt_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_SET_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF OLD.status<>'open' OR NEW.status<>'sealed'
       OR NEW.row_version<>OLD.row_version+1
       OR ROW(
            NEW.session_set_id,NEW.stable_request_id,NEW.authority_id,
            NEW.operation_id,NEW.stage_execution_id,NEW.owning_stage_run_request_id,
            NEW.scope_snapshot_id,NEW.session_set_ordinal,NEW.created_at
          ) IS DISTINCT FROM ROW(
            OLD.session_set_id,OLD.stable_request_id,OLD.authority_id,
            OLD.operation_id,OLD.stage_execution_id,OLD.owning_stage_run_request_id,
            OLD.scope_snapshot_id,OLD.session_set_ordinal,OLD.created_at
          )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_SET_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*) INTO expected_count
      FROM stage_run_units unit
     WHERE unit.operation_id=NEW.operation_id
       AND unit.stage_execution_id=NEW.stage_execution_id
       AND unit.scope_snapshot_id=NEW.scope_snapshot_id
       AND unit.stage_kind='investigation'
       AND unit.status<>'superseded';
    SELECT COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_main_read_sessions.v1',
               COALESCE(array_agg(
                   session_row.main_read_session_id::TEXT || ':' || session_row.member_sha256
                   ORDER BY session_row.organization_id,session_row.main_read_session_id
               ),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM investigation_main_read_sessions session_row
     WHERE session_row.session_set_id=NEW.session_set_id;
    SELECT COUNT(*) INTO receipt_count
      FROM investigation_main_read_session_receipts receipt
      JOIN investigation_main_read_sessions session_row
        ON session_row.main_read_session_id=receipt.main_read_session_id
     WHERE session_row.session_set_id=NEW.session_set_id;
    IF expected_count=0 OR actual_count<>expected_count OR receipt_count<>expected_count
       OR EXISTS(
            SELECT unit.id
              FROM stage_run_units unit
             WHERE unit.operation_id=NEW.operation_id
               AND unit.stage_execution_id=NEW.stage_execution_id
               AND unit.scope_snapshot_id=NEW.scope_snapshot_id
               AND unit.stage_kind='investigation'
               AND unit.status<>'superseded'
            EXCEPT
            SELECT session_row.stage_run_unit_id
              FROM investigation_main_read_sessions session_row
             WHERE session_row.session_set_id=NEW.session_set_id
       )
       OR NEW.member_count<>actual_count OR NEW.member_set_sha256<>actual_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_PARTITION_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_main_session_sets_guard
BEFORE UPDATE OR DELETE ON investigation_main_session_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_main_session_set_transition();

CREATE FUNCTION investigation_reject_late_unit_after_main_session_seal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.stage_kind='investigation' AND EXISTS(
        SELECT 1 FROM investigation_main_session_sets set_row
         WHERE set_row.operation_id=NEW.operation_id
           AND set_row.stage_execution_id=NEW.stage_execution_id
           AND set_row.scope_snapshot_id=NEW.scope_snapshot_id
           AND set_row.status='sealed'
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_MAIN_SESSION_PARTITION_FROZEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER stage_run_units_investigation_partition_freeze
BEFORE INSERT OR UPDATE OF operation_id,stage_execution_id,scope_snapshot_id,
    organization_id,stage_kind ON stage_run_units
FOR EACH ROW EXECUTE FUNCTION investigation_reject_late_unit_after_main_session_seal();

CREATE TRIGGER investigation_main_read_sessions_append_only
BEFORE UPDATE OR DELETE ON investigation_main_read_sessions
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER investigation_main_read_session_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_main_read_session_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- Host-signed rerun receipts are optional, but never partially populated.
CREATE TABLE hypothesis_verification_rerun_receipts (
    rerun_receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    rerun_contract_version INTEGER NOT NULL CHECK (rerun_contract_version>0),
    reason_code TEXT NOT NULL CHECK (btrim(reason_code)<>''),
    authority_receipt_sha256 TEXT NOT NULL CHECK (authority_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    rerun_receipt_sha256 TEXT NOT NULL CHECK (rerun_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(hypothesis_revision_id,rerun_contract_version),
    UNIQUE(rerun_receipt_id,rerun_receipt_sha256,rerun_contract_version),
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,organization_id)
        REFERENCES stage_run_units(id,operation_id,stage_execution_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_verification_tasks (
    task_id UUID PRIMARY KEY,
    stable_task_key_sha256 TEXT NOT NULL UNIQUE CHECK (stable_task_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    hypothesis_revision_sha256 TEXT NOT NULL CHECK (hypothesis_revision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_plan_id UUID NOT NULL,
    verification_plan_sha256 TEXT NOT NULL CHECK (verification_plan_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    relevant_evidence_snapshot_id UUID NOT NULL,
    semantic_evidence_set_sha256 TEXT NOT NULL CHECK (semantic_evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    open_obligation_set_sha256 TEXT NOT NULL CHECK (open_obligation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    semantic_attempt_fingerprint TEXT NOT NULL CHECK (semantic_attempt_fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    task_contract_version TEXT NOT NULL CHECK (task_contract_version='hypothesis_verification_task.v1'),
    first_admission_generation_id UUID NOT NULL,
    host_rerun_receipt_id UUID,
    host_rerun_receipt_sha256 TEXT CHECK (host_rerun_receipt_sha256 IS NULL OR host_rerun_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    rerun_contract_version INTEGER CHECK (rerun_contract_version IS NULL OR rerun_contract_version>0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (host_rerun_receipt_id IS NULL AND host_rerun_receipt_sha256 IS NULL AND rerun_contract_version IS NULL)
        OR
        (host_rerun_receipt_id IS NOT NULL AND host_rerun_receipt_sha256 IS NOT NULL AND rerun_contract_version IS NOT NULL)
    ),
    UNIQUE(task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id),
    UNIQUE(task_id,hypothesis_revision_id,verification_plan_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,organization_id)
        REFERENCES stage_run_units(id,operation_id,stage_execution_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,hypothesis_revision_sha256)
        REFERENCES attack_hypothesis_revisions(revision_id,revision_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id,verification_plan_sha256)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id,plan_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(first_admission_generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(host_rerun_receipt_id,host_rerun_receipt_sha256,rerun_contract_version)
        REFERENCES hypothesis_verification_rerun_receipts(
            rerun_receipt_id,rerun_receipt_sha256,rerun_contract_version
        ) ON DELETE RESTRICT
);

CREATE TRIGGER hypothesis_verification_rerun_receipts_append_only
BEFORE UPDATE OR DELETE ON hypothesis_verification_rerun_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER hypothesis_verification_tasks_append_only
BEFORE UPDATE OR DELETE ON hypothesis_verification_tasks
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE hypothesis_verification_task_state_heads (
    task_id UUID PRIMARY KEY REFERENCES hypothesis_verification_tasks(task_id) ON DELETE RESTRICT,
    current_state TEXT NOT NULL CHECK (current_state IN (
        'admitted','queued','planning','running','awaiting_authorization','consolidating',
        'stop_pending','draining','cancelled','blocked','recovery_required','terminal'
    )),
    latest_event_id UUID NOT NULL UNIQUE,
    head_version BIGINT NOT NULL CHECK (head_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_id,current_state,head_version)
);

CREATE TABLE hypothesis_verification_task_state_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_id UUID NOT NULL REFERENCES hypothesis_verification_tasks(task_id) ON DELETE RESTRICT,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>=0),
    expected_head_version BIGINT NOT NULL CHECK (expected_head_version>=0),
    from_state TEXT CHECK (from_state IS NULL OR from_state IN (
        'admitted','queued','planning','running','awaiting_authorization','consolidating',
        'stop_pending','draining','cancelled','blocked','recovery_required','terminal'
    )),
    to_state TEXT NOT NULL CHECK (to_state IN (
        'admitted','queued','planning','running','awaiting_authorization','consolidating',
        'stop_pending','draining','cancelled','blocked','recovery_required','terminal'
    )),
    reason_code TEXT,
    event_sha256 TEXT NOT NULL CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_id,event_ordinal),
    UNIQUE(event_id,task_id,to_state,event_ordinal)
);

ALTER TABLE hypothesis_verification_task_state_heads
    ADD CONSTRAINT hypothesis_verification_task_state_head_event_fk
    FOREIGN KEY(latest_event_id,task_id,current_state,head_version)
    REFERENCES hypothesis_verification_task_state_events(
        event_id,task_id,to_state,event_ordinal
    ) DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION investigation_task_transition_allowed(previous TEXT, next TEXT)
RETURNS BOOLEAN
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT (previous,next) IN (
        ('admitted','queued'),
        ('queued','planning'),('queued','cancelled'),('queued','recovery_required'),
        ('planning','running'),('planning','blocked'),('planning','recovery_required'),
        ('running','awaiting_authorization'),('running','consolidating'),
        ('running','stop_pending'),('running','blocked'),('running','recovery_required'),
        ('awaiting_authorization','running'),('awaiting_authorization','stop_pending'),
        ('awaiting_authorization','blocked'),
        ('consolidating','terminal'),('consolidating','blocked'),
        ('consolidating','recovery_required'),('consolidating','stop_pending'),
        ('stop_pending','draining'),
        ('draining','cancelled'),('draining','recovery_required'),('draining','consolidating'),
        ('recovery_required','queued'),('recovery_required','blocked')
    )
$$;

CREATE FUNCTION investigation_apply_task_state_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    head hypothesis_verification_task_state_heads%ROWTYPE;
    head_found BOOLEAN;
BEGIN
    PERFORM 1 FROM hypothesis_verification_tasks WHERE task_id=NEW.task_id FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_TASK_NOT_FOUND' USING ERRCODE='23503';
    END IF;
    SELECT * INTO head FROM hypothesis_verification_task_state_heads
     WHERE task_id=NEW.task_id FOR UPDATE;
    head_found := FOUND;
    PERFORM set_config('golish.investigation_task_event_apply','on',TRUE);
    IF NOT head_found THEN
        IF NEW.expected_head_version<>0 OR NEW.event_ordinal<>0
           OR NEW.from_state IS NOT NULL OR NEW.to_state<>'admitted'
        THEN
            RAISE EXCEPTION 'INVESTIGATION_TASK_INITIAL_STATE_INVALID' USING ERRCODE='23514';
        END IF;
        INSERT INTO hypothesis_verification_task_state_heads(
            task_id,current_state,latest_event_id,head_version
        ) VALUES(NEW.task_id,NEW.to_state,NEW.event_id,0);
    ELSE
        IF NEW.expected_head_version<>head.head_version
           OR NEW.event_ordinal<>head.head_version+1
           OR NEW.from_state IS DISTINCT FROM head.current_state
           OR NOT investigation_task_transition_allowed(head.current_state,NEW.to_state)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_TASK_STATE_CAS_INVALID' USING ERRCODE='23514';
        END IF;
        UPDATE hypothesis_verification_task_state_heads
           SET current_state=NEW.to_state,latest_event_id=NEW.event_id,
               head_version=NEW.event_ordinal,updated_at=statement_timestamp()
         WHERE task_id=NEW.task_id;
    END IF;
    PERFORM set_config('golish.investigation_task_event_apply','off',TRUE);
    RETURN NEW;
END;
$$;

CREATE FUNCTION investigation_guard_task_state_head_write()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF current_setting('golish.investigation_task_event_apply',TRUE) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'INVESTIGATION_TASK_HEAD_EVENT_ONLY' USING ERRCODE='23514';
    END IF;
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_TASK_HEAD_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_state_events_apply
BEFORE INSERT ON hypothesis_verification_task_state_events
FOR EACH ROW EXECUTE FUNCTION investigation_apply_task_state_event();
CREATE TRIGGER hypothesis_verification_task_state_events_append_only
BEFORE UPDATE OR DELETE ON hypothesis_verification_task_state_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER hypothesis_verification_task_state_heads_event_only
BEFORE INSERT OR UPDATE OR DELETE ON hypothesis_verification_task_state_heads
FOR EACH ROW EXECUTE FUNCTION investigation_guard_task_state_head_write();

-- Generation admission is an exact registry census.  Capacity may queue a
-- scheduled task, but cannot remove a generation member.
CREATE TABLE verification_admission_sets (
    admission_set_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_id UUID NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','sealed')),
    member_count BIGINT,
    member_set_sha256 TEXT CHECK(member_set_sha256 IS NULL OR member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CHECK(
        (status='open' AND member_count IS NULL AND member_set_sha256 IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND member_count IS NOT NULL AND member_count>=0
            AND member_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    UNIQUE(admission_set_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id),
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,organization_id)
        REFERENCES stage_run_units(id,operation_id,stage_execution_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE verification_admission_members (
    admission_member_id UUID PRIMARY KEY,
    admission_set_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_member_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN(
        'scheduled','needs_enrichment','deferred','out_of_scope','unsafe',
        'already_terminal','no_new_obligation'
    )),
    reason_code TEXT NOT NULL CHECK(btrim(reason_code)<>''),
    semantic_attempt_fingerprint TEXT NOT NULL CHECK(semantic_attempt_fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    task_id UUID,
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK((disposition='scheduled')=(task_id IS NOT NULL)),
    UNIQUE(admission_set_id,generation_member_id),
    UNIQUE(admission_set_id,hypothesis_revision_id),
    UNIQUE(admission_set_id,member_sha256),
    FOREIGN KEY(admission_set_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id)
        REFERENCES verification_admission_sets(
            admission_set_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(generation_member_id,hypothesis_revision_id)
        REFERENCES hypothesis_generation_members(generation_member_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(
            task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_admission_member_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM 1 FROM verification_admission_sets
     WHERE admission_set_id=NEW.admission_set_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_ADMISSION_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    IF NEW.disposition='scheduled' AND NOT EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
         WHERE task.task_id=NEW.task_id
           AND task.hypothesis_revision_id=NEW.hypothesis_revision_id
           AND task.semantic_attempt_fingerprint=NEW.semantic_attempt_fingerprint
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ADMISSION_TASK_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_admission_members_open_guard
BEFORE INSERT ON verification_admission_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_admission_member_insert();

CREATE FUNCTION investigation_guard_admission_set_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' OR OLD.status<>'open' OR NEW.status<>'sealed'
       OR NEW.row_version<>OLD.row_version+1
       OR ROW(NEW.admission_set_id,NEW.stable_request_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.generation_id,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.admission_set_id,OLD.stable_request_id,OLD.operation_id,
              OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
              OLD.organization_id,OLD.generation_id,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ADMISSION_SET_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*) INTO expected_count FROM hypothesis_generation_members
     WHERE generation_id=NEW.generation_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
             'verification_admission_members.v1',
             COALESCE(array_agg(member_sha256 ORDER BY hypothesis_revision_id),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM verification_admission_members WHERE admission_set_id=NEW.admission_set_id;
    IF actual_count<>expected_count OR NEW.member_count<>actual_count
       OR NEW.member_set_sha256<>actual_hash
       OR EXISTS(
            SELECT generation_member_id FROM hypothesis_generation_members
             WHERE generation_id=NEW.generation_id
            EXCEPT
            SELECT generation_member_id FROM verification_admission_members
             WHERE admission_set_id=NEW.admission_set_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ADMISSION_EXACT_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_admission_sets_guard
BEFORE UPDATE OR DELETE ON verification_admission_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_admission_set_transition();
CREATE TRIGGER verification_admission_members_append_only
BEFORE UPDATE OR DELETE ON verification_admission_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- Task-level objective denominator.  Campaign rows are reservations, not an
-- action authorization and not a second outer Verification stage.
CREATE TABLE hypothesis_verification_task_assignment_sets (
    assignment_set_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_id UUID NOT NULL UNIQUE,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK(status IN('open','sealed')),
    member_count BIGINT,
    member_set_sha256 TEXT CHECK(member_set_sha256 IS NULL OR member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CHECK(
        (status='open' AND member_count IS NULL AND member_set_sha256 IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND member_count IS NOT NULL AND member_count>0
            AND member_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    UNIQUE(assignment_set_id,task_id),
    UNIQUE(assignment_set_id,task_id,hypothesis_revision_id,verification_plan_id),
    FOREIGN KEY(task_id,hypothesis_revision_id,verification_plan_id)
        REFERENCES hypothesis_verification_tasks(task_id,hypothesis_revision_id,verification_plan_id)
        ON DELETE RESTRICT
);

CREATE UNIQUE INDEX attack_hypothesis_plan_objectives_task_assignment_fk
ON attack_hypothesis_verification_plan_objectives(
    plan_objective_id,plan_id,revision_id,objective_id
);

CREATE TABLE hypothesis_verification_task_campaigns (
    campaign_id UUID PRIMARY KEY,
    assignment_set_id UUID NOT NULL,
    task_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    plan_objective_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    campaign_contract_version TEXT NOT NULL DEFAULT 'task_campaign_reservation.v1'
        CHECK(campaign_contract_version='task_campaign_reservation.v1'),
    reservation_sha256 TEXT NOT NULL CHECK(reservation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_id,plan_objective_id),
    UNIQUE(campaign_id,assignment_set_id,task_id,plan_objective_id),
    FOREIGN KEY(assignment_set_id,task_id,hypothesis_revision_id,verification_plan_id)
        REFERENCES hypothesis_verification_task_assignment_sets(
            assignment_set_id,task_id,hypothesis_revision_id,verification_plan_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(
        plan_objective_id,verification_plan_id,
        hypothesis_revision_id,verification_objective_id
    )
        REFERENCES attack_hypothesis_verification_plan_objectives(
            plan_objective_id,plan_id,revision_id,objective_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(verification_objective_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_objectives(objective_id,revision_id)
        ON DELETE RESTRICT
);

CREATE UNIQUE INDEX hypothesis_objective_outcome_receipts_task_assignment_fk
ON hypothesis_objective_outcome_receipts(
    objective_outcome_receipt_id,verification_plan_id,verification_objective_id,
    operation_id,project_scope_id,organization_id,outcome_hash
);

CREATE TABLE hypothesis_verification_task_assignment_members (
    assignment_member_id UUID PRIMARY KEY,
    assignment_set_id UUID NOT NULL,
    task_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    plan_objective_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    assignment_kind TEXT NOT NULL CHECK(assignment_kind IN('campaign','already_satisfied','residual')),
    campaign_id UUID,
    already_satisfied_receipt_id UUID,
    already_satisfied_receipt_sha256 TEXT CHECK(already_satisfied_receipt_sha256 IS NULL OR already_satisfied_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    semantic_evidence_set_sha256 TEXT CHECK(semantic_evidence_set_sha256 IS NULL OR semantic_evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_kind TEXT CHECK(residual_kind IS NULL OR residual_kind IN(
        'no_known_capability','needs_enrichment','deferred','out_of_scope','unsafe','blocked'
    )),
    residual_reason_code TEXT,
    residual_owner TEXT,
    residual_next_action TEXT,
    residual_receipt_id UUID,
    residual_receipt_sha256 TEXT CHECK(residual_receipt_sha256 IS NULL OR residual_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(
        (assignment_kind='campaign' AND campaign_id IS NOT NULL
            AND already_satisfied_receipt_id IS NULL AND already_satisfied_receipt_sha256 IS NULL
            AND semantic_evidence_set_sha256 IS NULL AND residual_kind IS NULL
            AND residual_reason_code IS NULL AND residual_owner IS NULL
            AND residual_next_action IS NULL AND residual_receipt_id IS NULL
            AND residual_receipt_sha256 IS NULL)
        OR
        (assignment_kind='already_satisfied' AND campaign_id IS NULL
            AND already_satisfied_receipt_id IS NOT NULL
            AND already_satisfied_receipt_sha256 IS NOT NULL
            AND semantic_evidence_set_sha256 IS NOT NULL AND residual_kind IS NULL
            AND residual_reason_code IS NULL AND residual_owner IS NULL
            AND residual_next_action IS NULL AND residual_receipt_id IS NULL
            AND residual_receipt_sha256 IS NULL)
        OR
        (assignment_kind='residual' AND campaign_id IS NULL
            AND already_satisfied_receipt_id IS NULL AND already_satisfied_receipt_sha256 IS NULL
            AND semantic_evidence_set_sha256 IS NULL AND residual_kind IS NOT NULL
            AND btrim(residual_reason_code)<>'' AND btrim(residual_owner)<>''
            AND btrim(residual_next_action)<>'' AND residual_receipt_id IS NOT NULL
            AND residual_receipt_sha256 IS NOT NULL)
    ),
    UNIQUE(assignment_set_id,plan_objective_id),
    UNIQUE(assignment_set_id,member_sha256),
    FOREIGN KEY(assignment_set_id,task_id,hypothesis_revision_id,verification_plan_id)
        REFERENCES hypothesis_verification_task_assignment_sets(
            assignment_set_id,task_id,hypothesis_revision_id,verification_plan_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(plan_objective_id,verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plan_objectives(
            plan_objective_id,plan_id,revision_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_id,assignment_set_id,task_id,plan_objective_id)
        REFERENCES hypothesis_verification_task_campaigns(
            campaign_id,assignment_set_id,task_id,plan_objective_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_assignment_member_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    PERFORM 1 FROM hypothesis_verification_task_assignment_sets
     WHERE assignment_set_id=NEW.assignment_set_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSIGNMENT_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    SELECT task_row.* INTO STRICT task FROM hypothesis_verification_tasks task_row
     WHERE task_row.task_id=NEW.task_id FOR SHARE;
    IF NEW.assignment_kind='already_satisfied' AND NOT EXISTS(
        SELECT 1
          FROM hypothesis_objective_outcome_heads head
          JOIN hypothesis_objective_outcome_receipts receipt
            ON receipt.objective_outcome_receipt_id=head.current_outcome_id
           AND receipt.verification_plan_id=head.verification_plan_id
           AND receipt.verification_objective_id=head.verification_objective_id
         WHERE head.verification_plan_id=NEW.verification_plan_id
           AND head.verification_objective_id=NEW.verification_objective_id
           AND receipt.objective_outcome_receipt_id=NEW.already_satisfied_receipt_id
           AND receipt.outcome_hash=NEW.already_satisfied_receipt_sha256
           AND receipt.operation_id=task.operation_id
           AND receipt.project_scope_id=task.project_scope_id
           AND receipt.organization_id=task.organization_id
           AND receipt.outcome IN('proof','refutation')
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ALREADY_SATISFIED_NOT_CURRENT' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION investigation_guard_task_campaign_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM 1 FROM hypothesis_verification_task_assignment_sets
     WHERE assignment_set_id=NEW.assignment_set_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSIGNMENT_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_campaigns_open_guard
BEFORE INSERT ON hypothesis_verification_task_campaigns
FOR EACH ROW EXECUTE FUNCTION investigation_guard_task_campaign_insert();

CREATE TRIGGER hypothesis_verification_task_assignment_members_open_guard
BEFORE INSERT ON hypothesis_verification_task_assignment_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_assignment_member_insert();

CREATE FUNCTION investigation_guard_assignment_set_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' OR OLD.status<>'open' OR NEW.status<>'sealed'
       OR NEW.row_version<>OLD.row_version+1
       OR ROW(NEW.assignment_set_id,NEW.stable_request_id,NEW.task_id,
              NEW.hypothesis_revision_id,NEW.verification_plan_id,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.assignment_set_id,OLD.stable_request_id,OLD.task_id,
              OLD.hypothesis_revision_id,OLD.verification_plan_id,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSIGNMENT_SET_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT objective_count INTO STRICT expected_count
      FROM attack_hypothesis_verification_plans
     WHERE plan_id=NEW.verification_plan_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
             'hypothesis_verification_task_assignments.v1',
             COALESCE(array_agg(member_sha256 ORDER BY plan_objective_id),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM hypothesis_verification_task_assignment_members
     WHERE assignment_set_id=NEW.assignment_set_id;
    IF actual_count<>expected_count OR NEW.member_count<>actual_count
       OR NEW.member_set_sha256<>actual_hash
       OR EXISTS(
            SELECT plan_objective_id FROM attack_hypothesis_verification_plan_objectives
             WHERE plan_id=NEW.verification_plan_id
            EXCEPT
            SELECT plan_objective_id FROM hypothesis_verification_task_assignment_members
             WHERE assignment_set_id=NEW.assignment_set_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSIGNMENT_EXACT_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_assignment_sets_guard
BEFORE UPDATE OR DELETE ON hypothesis_verification_task_assignment_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_assignment_set_transition();

CREATE TABLE hypothesis_verification_task_outcome_sets (
    outcome_set_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    assignment_set_id UUID NOT NULL UNIQUE,
    task_id UUID NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'open' CHECK(status IN('open','sealed')),
    member_count BIGINT,
    member_set_sha256 TEXT CHECK(member_set_sha256 IS NULL OR member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CHECK(
        (status='open' AND member_count IS NULL AND member_set_sha256 IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND member_count IS NOT NULL AND member_count>=0
            AND member_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    UNIQUE(outcome_set_id,assignment_set_id,task_id),
    FOREIGN KEY(assignment_set_id,task_id)
        REFERENCES hypothesis_verification_task_assignment_sets(assignment_set_id,task_id)
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_verification_task_outcome_members (
    outcome_member_id UUID PRIMARY KEY,
    outcome_set_id UUID NOT NULL,
    assignment_set_id UUID NOT NULL,
    task_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    outcome_kind TEXT NOT NULL CHECK(outcome_kind IN(
        'completed','blocked','cancelled_before_start','recovery_required'
    )),
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_sha256 TEXT NOT NULL CHECK(terminal_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(outcome_set_id,campaign_id),
    UNIQUE(outcome_set_id,member_sha256),
    FOREIGN KEY(outcome_set_id,assignment_set_id,task_id)
        REFERENCES hypothesis_verification_task_outcome_sets(
            outcome_set_id,assignment_set_id,task_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_outcome_member_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM 1 FROM hypothesis_verification_task_outcome_sets
     WHERE outcome_set_id=NEW.outcome_set_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_OUTCOME_SET_NOT_OPEN' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM hypothesis_verification_task_campaigns campaign
         WHERE campaign.campaign_id=NEW.campaign_id
           AND campaign.assignment_set_id=NEW.assignment_set_id
           AND campaign.task_id=NEW.task_id
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_OUTCOME_CAMPAIGN_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_outcome_members_open_guard
BEFORE INSERT ON hypothesis_verification_task_outcome_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_outcome_member_insert();

CREATE FUNCTION investigation_guard_outcome_set_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' OR OLD.status<>'open' OR NEW.status<>'sealed'
       OR NEW.row_version<>OLD.row_version+1
       OR ROW(NEW.outcome_set_id,NEW.stable_request_id,NEW.assignment_set_id,
              NEW.task_id,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.outcome_set_id,OLD.stable_request_id,OLD.assignment_set_id,
              OLD.task_id,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_OUTCOME_SET_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*) INTO expected_count
      FROM hypothesis_verification_task_assignment_members
     WHERE assignment_set_id=NEW.assignment_set_id AND assignment_kind='campaign';
    SELECT COUNT(*),unified_investigation_exact_set_hash(
             'hypothesis_verification_task_outcomes.v1',
             COALESCE(array_agg(member_sha256 ORDER BY campaign_id),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM hypothesis_verification_task_outcome_members
     WHERE outcome_set_id=NEW.outcome_set_id;
    IF actual_count<>expected_count OR NEW.member_count<>actual_count
       OR NEW.member_set_sha256<>actual_hash
       OR EXISTS(
            SELECT campaign_id FROM hypothesis_verification_task_assignment_members
             WHERE assignment_set_id=NEW.assignment_set_id AND assignment_kind='campaign'
            EXCEPT
            SELECT campaign_id FROM hypothesis_verification_task_outcome_members
             WHERE outcome_set_id=NEW.outcome_set_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_OUTCOME_CAMPAIGN_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_outcome_sets_guard
BEFORE UPDATE OR DELETE ON hypothesis_verification_task_outcome_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_outcome_set_transition();

DO $$
DECLARE
    table_name TEXT;
BEGIN
    FOR table_name IN
        SELECT unnest(ARRAY[
            'hypothesis_verification_task_campaigns',
            'hypothesis_verification_task_assignment_members',
            'hypothesis_verification_task_outcome_members'
        ])
    LOOP
        EXECUTE format(
            'CREATE TRIGGER %I_append_only BEFORE UPDATE OR DELETE ON %I '
            'FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only()',
            table_name,table_name
        );
    END LOOP;
END;
$$;

-- CAS fuel: current projections are checked against immutable reservation
-- events at deferred commit time.  Unknown execution remains held.
CREATE TABLE investigation_fuel_budgets (
    budget_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN('operation','unit','task')),
    owner_id UUID NOT NULL,
    stage_run_unit_id UUID,
    scope_snapshot_id UUID,
    organization_id UUID,
    task_id UUID,
    budget_contract_version TEXT NOT NULL DEFAULT 'investigation_fuel_budget.v1'
        CHECK(budget_contract_version='investigation_fuel_budget.v1'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(
        (scope_kind='operation' AND owner_id=operation_id
            AND stage_run_unit_id IS NULL AND scope_snapshot_id IS NULL
            AND organization_id IS NULL AND task_id IS NULL)
        OR
        (scope_kind='unit' AND owner_id=stage_run_unit_id
            AND stage_run_unit_id IS NOT NULL AND scope_snapshot_id IS NOT NULL
            AND organization_id IS NOT NULL AND task_id IS NULL)
        OR
        (scope_kind='task' AND owner_id=task_id AND task_id IS NOT NULL
            AND stage_run_unit_id IS NOT NULL AND scope_snapshot_id IS NOT NULL
            AND organization_id IS NOT NULL)
    ),
    UNIQUE(budget_id,operation_id,stage_execution_id,scope_kind,owner_id)
);

CREATE FUNCTION investigation_guard_fuel_budget_identity()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    authority investigation_stage_run_authorities%ROWTYPE;
BEGIN
    SELECT * INTO STRICT authority FROM investigation_stage_run_authorities
     WHERE authority_id=NEW.authority_id AND operation_id=NEW.operation_id
       AND stage_execution_id=NEW.stage_execution_id
       AND owning_stage_run_request_id=NEW.owning_stage_run_request_id FOR SHARE;
    IF NEW.scope_kind IN('unit','task') THEN
        IF NEW.scope_snapshot_id<>authority.scope_snapshot_id OR NOT EXISTS(
            SELECT 1 FROM stage_run_units unit
             WHERE unit.id=NEW.stage_run_unit_id
               AND unit.operation_id=NEW.operation_id
               AND unit.stage_execution_id=NEW.stage_execution_id
               AND unit.scope_snapshot_id=NEW.scope_snapshot_id
               AND unit.organization_id=NEW.organization_id
               AND unit.stage_kind='investigation'
        ) THEN
            RAISE EXCEPTION 'INVESTIGATION_FUEL_UNIT_IDENTITY_MISMATCH' USING ERRCODE='23514';
        END IF;
    END IF;
    IF NEW.scope_kind='task' AND NOT EXISTS(
        SELECT 1 FROM hypothesis_verification_tasks task
         WHERE task.task_id=NEW.task_id AND task.operation_id=NEW.operation_id
           AND task.stage_execution_id=NEW.stage_execution_id
           AND task.stage_run_unit_id=NEW.stage_run_unit_id
           AND task.scope_snapshot_id=NEW.scope_snapshot_id
           AND task.organization_id=NEW.organization_id
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_FUEL_TASK_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_fuel_budgets_identity_guard
BEFORE INSERT ON investigation_fuel_budgets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_fuel_budget_identity();
CREATE TRIGGER investigation_fuel_budgets_append_only
BEFORE UPDATE OR DELETE ON investigation_fuel_budgets
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_fuel_budget_heads (
    budget_id UUID NOT NULL REFERENCES investigation_fuel_budgets(budget_id) ON DELETE RESTRICT,
    axis TEXT NOT NULL CHECK(axis IN(
        'analysis_generation','verification_task','campaign','subtask',
        'nested_delegation','consult_or_tool_call','prepared_action',
        'wall_clock_millis','provider_token','risk_micros'
    )),
    limit_amount BIGINT NOT NULL CHECK(limit_amount>0),
    reserved_amount BIGINT NOT NULL DEFAULT 0 CHECK(reserved_amount>=0),
    consumed_amount BIGINT NOT NULL DEFAULT 0 CHECK(consumed_amount>=0),
    unknown_held_amount BIGINT NOT NULL DEFAULT 0 CHECK(unknown_held_amount>=0),
    refunded_before_begin_amount BIGINT NOT NULL DEFAULT 0 CHECK(refunded_before_begin_amount>=0),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(budget_id,axis),
    CHECK(reserved_amount+consumed_amount+unknown_held_amount<=limit_amount),
    UNIQUE(budget_id,axis,head_version)
);

CREATE TABLE investigation_fuel_reservations (
    reservation_id UUID PRIMARY KEY,
    budget_id UUID NOT NULL,
    axis TEXT NOT NULL,
    amount BIGINT NOT NULL CHECK(amount>0),
    work_key_sha256 TEXT NOT NULL CHECK(work_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK(state IN('reserved','consumed','refunded_before_begin','unknown_held')),
    reservation_epoch BIGINT NOT NULL CHECK(reservation_epoch>0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(budget_id,axis,work_key_sha256),
    UNIQUE(reservation_id,budget_id,axis),
    UNIQUE(reservation_id,budget_id,axis,state,row_version),
    FOREIGN KEY(budget_id,axis)
        REFERENCES investigation_fuel_budget_heads(budget_id,axis) ON DELETE RESTRICT
);

CREATE TABLE investigation_fuel_reservation_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    reservation_id UUID NOT NULL,
    budget_id UUID NOT NULL,
    axis TEXT NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK(event_ordinal>=0),
    from_state TEXT CHECK(from_state IS NULL OR from_state IN(
        'reserved','consumed','refunded_before_begin','unknown_held'
    )),
    to_state TEXT NOT NULL CHECK(to_state IN(
        'reserved','consumed','refunded_before_begin','unknown_held'
    )),
    amount BIGINT NOT NULL CHECK(amount>0),
    event_sha256 TEXT NOT NULL CHECK(event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(reservation_id,event_ordinal),
    FOREIGN KEY(reservation_id,budget_id,axis)
        REFERENCES investigation_fuel_reservations(reservation_id,budget_id,axis)
        ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_fuel_reservation_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_FUEL_RESERVATION_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF ROW(NEW.reservation_id,NEW.budget_id,NEW.axis,NEW.amount,
           NEW.work_key_sha256,NEW.reservation_epoch,NEW.created_at)
       IS DISTINCT FROM
       ROW(OLD.reservation_id,OLD.budget_id,OLD.axis,OLD.amount,
           OLD.work_key_sha256,OLD.reservation_epoch,OLD.created_at)
       OR OLD.state<>'reserved' OR NEW.state='reserved'
       OR NEW.row_version<>OLD.row_version+1
    THEN
        RAISE EXCEPTION 'INVESTIGATION_FUEL_RESERVATION_TRANSITION_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_fuel_reservations_guard
BEFORE UPDATE OR DELETE ON investigation_fuel_reservations
FOR EACH ROW EXECUTE FUNCTION investigation_guard_fuel_reservation_update();
CREATE TRIGGER investigation_fuel_reservation_events_append_only
BEFORE UPDATE OR DELETE ON investigation_fuel_reservation_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION investigation_validate_fuel_head_exact()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    head investigation_fuel_budget_heads%ROWTYPE;
    actual_reserved BIGINT;
    actual_consumed BIGINT;
    actual_unknown BIGINT;
    actual_refunded BIGINT;
BEGIN
    SELECT * INTO STRICT head FROM investigation_fuel_budget_heads
     WHERE budget_id=COALESCE(NEW.budget_id,OLD.budget_id)
       AND axis=COALESCE(NEW.axis,OLD.axis);
    SELECT
        COALESCE(SUM(amount) FILTER(WHERE state='reserved'),0),
        COALESCE(SUM(amount) FILTER(WHERE state='consumed'),0),
        COALESCE(SUM(amount) FILTER(WHERE state='unknown_held'),0),
        COALESCE(SUM(amount) FILTER(WHERE state='refunded_before_begin'),0)
      INTO actual_reserved,actual_consumed,actual_unknown,actual_refunded
      FROM investigation_fuel_reservations
     WHERE budget_id=head.budget_id AND axis=head.axis;
    IF ROW(head.reserved_amount,head.consumed_amount,head.unknown_held_amount,
           head.refunded_before_begin_amount)
       IS DISTINCT FROM ROW(actual_reserved,actual_consumed,actual_unknown,actual_refunded)
       OR head.reserved_amount+head.consumed_amount+head.unknown_held_amount>head.limit_amount
    THEN
        RAISE EXCEPTION 'INVESTIGATION_FUEL_HEAD_EXACT_LEDGER_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_fuel_heads_exact_ledger
AFTER INSERT OR UPDATE ON investigation_fuel_budget_heads
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_fuel_head_exact();
CREATE CONSTRAINT TRIGGER investigation_fuel_reservations_exact_ledger
AFTER INSERT OR UPDATE ON investigation_fuel_reservations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_fuel_head_exact();

CREATE FUNCTION investigation_validate_fuel_reservation_events()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    reservation investigation_fuel_reservations%ROWTYPE;
    event_count BIGINT;
    first_state TEXT;
    latest_state TEXT;
    latest_amount BIGINT;
BEGIN
    SELECT * INTO STRICT reservation
      FROM investigation_fuel_reservations
     WHERE reservation_id=COALESCE(NEW.reservation_id,OLD.reservation_id);
    SELECT COUNT(*),
           (array_agg(to_state ORDER BY event_ordinal))[1],
           (array_agg(to_state ORDER BY event_ordinal DESC))[1],
           (array_agg(amount ORDER BY event_ordinal DESC))[1]
      INTO event_count,first_state,latest_state,latest_amount
      FROM investigation_fuel_reservation_events
     WHERE reservation_id=reservation.reservation_id;
    IF event_count<>reservation.row_version+1 OR first_state<>'reserved'
       OR latest_state<>reservation.state OR latest_amount<>reservation.amount
       OR EXISTS(
            SELECT 1
              FROM generate_series(0,reservation.row_version) ordinal
              LEFT JOIN investigation_fuel_reservation_events event
                ON event.reservation_id=reservation.reservation_id
               AND event.event_ordinal=ordinal
             WHERE event.event_id IS NULL
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_FUEL_EVENT_CENSUS_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_fuel_reservation_event_census
AFTER INSERT OR UPDATE ON investigation_fuel_reservations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_fuel_reservation_events();
CREATE CONSTRAINT TRIGGER investigation_fuel_event_reservation_census
AFTER INSERT ON investigation_fuel_reservation_events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_fuel_reservation_events();

CREATE TABLE investigation_semantic_cycle_receipts (
    semantic_cycle_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    cycle_fingerprint_sha256 TEXT NOT NULL CHECK(cycle_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    hypothesis_revision_sha256 TEXT NOT NULL CHECK(hypothesis_revision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_plan_sha256 TEXT NOT NULL CHECK(verification_plan_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    semantic_evidence_set_sha256 TEXT NOT NULL CHECK(semantic_evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    open_obligation_set_sha256 TEXT NOT NULL CHECK(open_obligation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    remaining_work_set_sha256 TEXT NOT NULL CHECK(remaining_work_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK(disposition IN('advanced','fixed_point','residual','stopped')),
    residual_reason_code TEXT,
    stop_receipt_id UUID,
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(
        (disposition IN('advanced','fixed_point') AND residual_reason_code IS NULL AND stop_receipt_id IS NULL)
        OR (disposition='residual' AND btrim(residual_reason_code)<>'' AND stop_receipt_id IS NULL)
        OR (disposition='stopped' AND residual_reason_code IS NULL AND stop_receipt_id IS NOT NULL)
    ),
    UNIQUE(task_id,cycle_fingerprint_sha256),
    UNIQUE(semantic_cycle_receipt_id,task_id,receipt_sha256),
    FOREIGN KEY(task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(
            task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_semantic_cycle_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_semantic_cycle_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
