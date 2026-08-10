-- Append-only, typed Refiner plan authority for unified Investigation.
--
-- The Generator manifest, every remaining-plan patch and the result barrier
-- are bound to existing PentAGI pipeline events.  Consequently their hashes
-- are already members of the delegation exact set frozen by the task census
-- and recomputed by InvestigationRunClosureV1.

CREATE TABLE investigation_refiner_plan_ledgers (
    ledger_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL CHECK (btrim(owning_stage_run_request_id)<>''),
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generator_pipeline_event_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_pipeline_events(pipeline_event_id) ON DELETE RESTRICT,
    generator_manifest JSONB NOT NULL CHECK (
        jsonb_typeof(generator_manifest)='object' AND generator_manifest<>'{}'::JSONB
    ),
    generator_manifest_sha256 TEXT NOT NULL
        CHECK (generator_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    generator_subtask_count BIGINT NOT NULL CHECK (generator_subtask_count>0),
    generator_subtask_set_sha256 TEXT NOT NULL
        CHECK (generator_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ledger_sha256 TEXT NOT NULL UNIQUE CHECK (ledger_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_pentagi_task_plans(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_refiner_plan_patches (
    patch_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    ledger_id UUID NOT NULL REFERENCES investigation_refiner_plan_ledgers(ledger_id) ON DELETE RESTRICT,
    task_plan_id UUID NOT NULL,
    patch_ordinal BIGINT NOT NULL CHECK (patch_ordinal>=0),
    refiner_pipeline_event_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_pipeline_events(pipeline_event_id) ON DELETE RESTRICT,
    expected_previous_state_sha256 TEXT NOT NULL
        CHECK (expected_previous_state_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    remaining_plan_payload JSONB NOT NULL CHECK (jsonb_typeof(remaining_plan_payload)='object'),
    remaining_plan_payload_sha256 TEXT NOT NULL
        CHECK (remaining_plan_payload_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    active_realized_subtask_count BIGINT NOT NULL CHECK (active_realized_subtask_count>=0),
    active_realized_subtask_set_sha256 TEXT NOT NULL
        CHECK (active_realized_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    patch_sha256 TEXT NOT NULL UNIQUE CHECK (patch_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(ledger_id,patch_ordinal),
    UNIQUE(patch_id,task_plan_id),
    FOREIGN KEY(task_plan_id)
        REFERENCES investigation_pentagi_task_plans(task_plan_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_refiner_plan_patch_members (
    patch_member_id UUID PRIMARY KEY,
    patch_id UUID NOT NULL,
    task_plan_id UUID NOT NULL,
    subtask_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(patch_id,subtask_id),
    UNIQUE(patch_id,member_ordinal),
    UNIQUE(patch_id,member_sha256),
    FOREIGN KEY(patch_id,task_plan_id)
        REFERENCES investigation_refiner_plan_patches(patch_id,task_plan_id) ON DELETE RESTRICT,
    FOREIGN KEY(subtask_id,task_plan_id)
        REFERENCES investigation_pentagi_subtasks(subtask_id,task_plan_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_refiner_plan_ledger_seals (
    seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    ledger_id UUID NOT NULL UNIQUE
        REFERENCES investigation_refiner_plan_ledgers(ledger_id) ON DELETE RESTRICT,
    task_plan_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_task_plans(task_plan_id) ON DELETE RESTRICT,
    result_barrier_pipeline_event_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_pipeline_events(pipeline_event_id) ON DELETE RESTRICT,
    patch_count BIGINT NOT NULL CHECK (patch_count>0),
    patch_set_sha256 TEXT NOT NULL CHECK (patch_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    final_patch_id UUID NOT NULL UNIQUE,
    final_patch_sha256 TEXT NOT NULL CHECK (final_patch_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    final_active_realized_subtask_count BIGINT NOT NULL
        CHECK (final_active_realized_subtask_count>=0),
    final_active_realized_subtask_set_sha256 TEXT NOT NULL
        CHECK (final_active_realized_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    generator_subtask_count BIGINT NOT NULL CHECK (generator_subtask_count>0),
    generator_subtask_set_sha256 TEXT NOT NULL
        CHECK (generator_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    seal_sha256 TEXT NOT NULL UNIQUE CHECK (seal_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(final_patch_id,task_plan_id)
        REFERENCES investigation_refiner_plan_patches(patch_id,task_plan_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_refiner_plan_ledgers_append_only
BEFORE UPDATE OR DELETE ON investigation_refiner_plan_ledgers
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER investigation_refiner_plan_patches_append_only
BEFORE UPDATE OR DELETE ON investigation_refiner_plan_patches
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER investigation_refiner_plan_patch_members_append_only
BEFORE UPDATE OR DELETE ON investigation_refiner_plan_patch_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
CREATE TRIGGER investigation_refiner_plan_ledger_seals_append_only
BEFORE UPDATE OR DELETE ON investigation_refiner_plan_ledger_seals
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION investigation_refiner_payload_hash_v1(p_kind TEXT,p_payload JSONB)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT 'sha256:' || encode(digest(convert_to(
        concat_ws(':','golish.investigation_refiner_payload.v1',p_kind,p_payload::TEXT),
        'UTF8'),'sha256'),'hex')
$$;

CREATE FUNCTION create_investigation_refiner_plan_ledger_v1(
    p_ledger_id UUID,
    p_stable_request_id UUID,
    p_task_plan_id UUID,
    p_generator_pipeline_event_id UUID,
    p_generator_manifest JSONB
)
RETURNS investigation_refiner_plan_ledgers
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_refiner_plan_ledgers%ROWTYPE;
    plan investigation_pentagi_task_plans%ROWTYPE;
    primary_dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    result investigation_refiner_plan_ledgers%ROWTYPE;
    manifest_hash TEXT;
    subtask_count BIGINT;
    subtask_hash TEXT;
    ledger_hash TEXT;
    next_event_ordinal BIGINT;
BEGIN
    IF p_generator_manifest IS NULL OR jsonb_typeof(p_generator_manifest)<>'object'
       OR p_generator_manifest='{}'::JSONB
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_GENERATOR_MANIFEST_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO existing FROM investigation_refiner_plan_ledgers
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.ledger_id,existing.task_plan_id,
               existing.generator_pipeline_event_id,existing.generator_manifest)
           IS DISTINCT FROM
           ROW(p_ledger_id,p_task_plan_id,p_generator_pipeline_event_id,p_generator_manifest)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_REFINER_LEDGER_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=p_task_plan_id AND status='open' FOR UPDATE;
    SELECT * INTO STRICT primary_dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_generator_subtasks.v1',
               COALESCE(array_agg(subtask_id::TEXT || ':' || member_sha256
                                  ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO subtask_count,subtask_hash
      FROM investigation_pentagi_subtasks WHERE task_plan_id=p_task_plan_id;
    IF subtask_count=0 THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_GENERATOR_SUBTASK_SET_EMPTY' USING ERRCODE='23514';
    END IF;
    manifest_hash := investigation_refiner_payload_hash_v1('generator_manifest',p_generator_manifest);
    ledger_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_ledger.v1',p_ledger_id::TEXT,p_task_plan_id::TEXT,
        manifest_hash,subtask_count::TEXT,subtask_hash),'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
        event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256
    ) VALUES(
        p_generator_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,
        next_event_ordinal,'generator_sealed',primary_dispatch.worker_run_id,
        primary_dispatch.dispatch_receipt_id,ledger_hash
    );
    INSERT INTO investigation_refiner_plan_ledgers(
        ledger_id,stable_request_id,task_plan_id,authority_id,operation_id,
        stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
        scope_snapshot_id,organization_id,generator_pipeline_event_id,
        generator_manifest,generator_manifest_sha256,generator_subtask_count,
        generator_subtask_set_sha256,ledger_sha256
    ) VALUES(
        p_ledger_id,p_stable_request_id,p_task_plan_id,plan.authority_id,plan.operation_id,
        plan.stage_execution_id,plan.owning_stage_run_request_id,plan.stage_run_unit_id,
        plan.scope_snapshot_id,plan.organization_id,p_generator_pipeline_event_id,
        p_generator_manifest,manifest_hash,subtask_count,subtask_hash,ledger_hash
    ) RETURNING * INTO result;
    RETURN result;
END;
$$;

CREATE FUNCTION append_investigation_refiner_plan_patch_v1(
    p_patch_id UUID,
    p_stable_request_id UUID,
    p_ledger_id UUID,
    p_task_plan_id UUID,
    p_refiner_pipeline_event_id UUID,
    p_expected_previous_state_sha256 TEXT,
    p_remaining_plan_payload JSONB,
    p_active_realized_subtask_ids UUID[]
)
RETURNS investigation_refiner_plan_patches
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_refiner_plan_patches%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    primary_dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    previous_patch investigation_refiner_plan_patches%ROWTYPE;
    result investigation_refiner_plan_patches%ROWTYPE;
    patch_ordinal BIGINT;
    previous_hash TEXT;
    payload_hash TEXT;
    active_count BIGINT;
    active_hash TEXT;
    patch_hash TEXT;
    next_event_ordinal BIGINT;
BEGIN
    IF p_remaining_plan_payload IS NULL OR jsonb_typeof(p_remaining_plan_payload)<>'object'
       OR p_active_realized_subtask_ids IS NULL
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_PAYLOAD_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO existing FROM investigation_refiner_plan_patches
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        SELECT COUNT(*),unified_investigation_exact_set_hash(
                   'investigation_refiner_active_realized_subtasks.v1',
                   COALESCE(array_agg(member_sha256 ORDER BY member_ordinal),ARRAY[]::TEXT[])
               ) INTO active_count,active_hash
          FROM investigation_refiner_plan_patch_members WHERE patch_id=existing.patch_id;
        IF ROW(existing.patch_id,existing.ledger_id,existing.task_plan_id,
               existing.refiner_pipeline_event_id,existing.expected_previous_state_sha256,
               existing.remaining_plan_payload,existing.active_realized_subtask_count,
               existing.active_realized_subtask_set_sha256)
           IS DISTINCT FROM
           ROW(p_patch_id,p_ledger_id,p_task_plan_id,p_refiner_pipeline_event_id,
               p_expected_previous_state_sha256,p_remaining_plan_payload,active_count,active_hash)
           OR active_count<>(SELECT COUNT(DISTINCT value) FROM unnest(p_active_realized_subtask_ids) value)
           OR EXISTS(
               SELECT 1 FROM unnest(p_active_realized_subtask_ids) value
                WHERE NOT EXISTS(
                    SELECT 1 FROM investigation_refiner_plan_patch_members member
                     WHERE member.patch_id=existing.patch_id AND member.subtask_id=value
                )
           )
        THEN
            RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=p_ledger_id AND task_plan_id=p_task_plan_id FOR UPDATE;
    IF EXISTS(SELECT 1 FROM investigation_refiner_plan_ledger_seals WHERE ledger_id=p_ledger_id) THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_LEDGER_ALREADY_SEALED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT primary_dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    SELECT * INTO previous_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=p_ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    IF FOUND THEN
        patch_ordinal := previous_patch.patch_ordinal+1;
        previous_hash := previous_patch.patch_sha256;
    ELSE
        patch_ordinal := 0;
        previous_hash := ledger.ledger_sha256;
    END IF;
    IF p_expected_previous_state_sha256<>previous_hash THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_PREVIOUS_STATE_CAS_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    IF cardinality(p_active_realized_subtask_ids)
       <>cardinality(ARRAY(SELECT DISTINCT value FROM unnest(p_active_realized_subtask_ids) value))
       OR EXISTS(
           SELECT 1 FROM unnest(p_active_realized_subtask_ids) value
            WHERE NOT EXISTS(
                SELECT 1 FROM investigation_pentagi_subtasks subtask
                 WHERE subtask.task_plan_id=p_task_plan_id
                   AND subtask.subtask_id=value AND subtask.runnable
            )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_ACTIVE_SUBTASK_SET_INVALID' USING ERRCODE='23514';
    END IF;
    IF previous_patch.patch_id IS NOT NULL AND EXISTS(
        SELECT 1 FROM investigation_refiner_plan_patch_members prior_member
         WHERE prior_member.patch_id=previous_patch.patch_id
           AND NOT (prior_member.subtask_id=ANY(p_active_realized_subtask_ids))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_ACTIVE_SUBTASK_SET_REGRESSED' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_active_realized_subtasks.v1',
               COALESCE(array_agg(
                   'sha256:' || encode(digest(convert_to(concat_ws(':',
                       'golish.investigation_refiner_active_realized_subtask.v1',
                       subtask_id::TEXT,member_sha256),'UTF8'),'sha256'),'hex')
                   ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO active_count,active_hash
      FROM investigation_pentagi_subtasks
     WHERE task_plan_id=p_task_plan_id AND subtask_id=ANY(p_active_realized_subtask_ids);
    payload_hash := investigation_refiner_payload_hash_v1('remaining_plan_patch',p_remaining_plan_payload);
    patch_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_patch.v1',p_patch_id::TEXT,p_ledger_id::TEXT,
        patch_ordinal::TEXT,previous_hash,payload_hash,active_count::TEXT,active_hash),
        'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
        event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256
    ) VALUES(
        p_refiner_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,
        next_event_ordinal,'refiner_patch',primary_dispatch.worker_run_id,
        primary_dispatch.dispatch_receipt_id,patch_hash
    );
    INSERT INTO investigation_refiner_plan_patches(
        patch_id,stable_request_id,ledger_id,task_plan_id,patch_ordinal,
        refiner_pipeline_event_id,expected_previous_state_sha256,remaining_plan_payload,
        remaining_plan_payload_sha256,active_realized_subtask_count,
        active_realized_subtask_set_sha256,patch_sha256
    ) VALUES(
        p_patch_id,p_stable_request_id,p_ledger_id,p_task_plan_id,patch_ordinal,
        p_refiner_pipeline_event_id,previous_hash,p_remaining_plan_payload,payload_hash,
        active_count,active_hash,patch_hash
    ) RETURNING * INTO result;
    INSERT INTO investigation_refiner_plan_patch_members(
        patch_member_id,patch_id,task_plan_id,subtask_id,member_ordinal,member_sha256
    )
    SELECT gen_random_uuid(),p_patch_id,p_task_plan_id,subtask_id,
           (row_number() OVER(ORDER BY subtask_ordinal)-1)::INTEGER,
           'sha256:' || encode(digest(convert_to(concat_ws(':',
               'golish.investigation_refiner_active_realized_subtask.v1',
               subtask_id::TEXT,member_sha256),'UTF8'),'sha256'),'hex')
      FROM investigation_pentagi_subtasks
     WHERE task_plan_id=p_task_plan_id AND subtask_id=ANY(p_active_realized_subtask_ids)
     ORDER BY subtask_ordinal;
    RETURN result;
END;
$$;

CREATE FUNCTION seal_investigation_refiner_plan_ledger_v1(
    p_seal_id UUID,
    p_stable_request_id UUID,
    p_ledger_id UUID,
    p_task_plan_id UUID,
    p_result_barrier_pipeline_event_id UUID,
    p_expected_final_patch_sha256 TEXT
)
RETURNS investigation_refiner_plan_ledger_seals
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_refiner_plan_ledger_seals%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    primary_dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    final_patch investigation_refiner_plan_patches%ROWTYPE;
    result investigation_refiner_plan_ledger_seals%ROWTYPE;
    actual_generator_count BIGINT;
    actual_generator_hash TEXT;
    patch_count BIGINT;
    patch_hash TEXT;
    actual_active_count BIGINT;
    actual_active_hash TEXT;
    runnable_count BIGINT;
    runnable_hash TEXT;
    seal_hash TEXT;
    next_event_ordinal BIGINT;
BEGIN
    SELECT * INTO existing FROM investigation_refiner_plan_ledger_seals
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.seal_id,existing.ledger_id,existing.task_plan_id,
               existing.result_barrier_pipeline_event_id,existing.final_patch_sha256)
           IS DISTINCT FROM
           ROW(p_seal_id,p_ledger_id,p_task_plan_id,p_result_barrier_pipeline_event_id,
               p_expected_final_patch_sha256)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_REFINER_SEAL_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=p_ledger_id AND task_plan_id=p_task_plan_id FOR UPDATE;
    SELECT * INTO STRICT primary_dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    SELECT * INTO final_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=p_ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    IF NOT FOUND OR final_patch.patch_sha256<>p_expected_final_patch_sha256 THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_FINAL_PATCH_CAS_MISMATCH' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_generator_subtasks.v1',
               COALESCE(array_agg(subtask_id::TEXT || ':' || member_sha256
                                  ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO actual_generator_count,actual_generator_hash
      FROM investigation_pentagi_subtasks WHERE task_plan_id=p_task_plan_id;
    IF ROW(actual_generator_count,actual_generator_hash)
       IS DISTINCT FROM ROW(ledger.generator_subtask_count,ledger.generator_subtask_set_sha256)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_GENERATOR_CENSUS_CHANGED' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_plan_patches.v1',
               COALESCE(array_agg(patch_id::TEXT || ':' || patch_sha256
                                  ORDER BY patch_ordinal),ARRAY[]::TEXT[])
           ) INTO patch_count,patch_hash
      FROM investigation_refiner_plan_patches WHERE ledger_id=p_ledger_id;
    IF patch_count=0 OR final_patch.patch_ordinal<>patch_count-1
       OR EXISTS(
           SELECT 1 FROM generate_series(0,patch_count-1) ordinal
            WHERE NOT EXISTS(
                SELECT 1 FROM investigation_refiner_plan_patches patch
                 WHERE patch.ledger_id=p_ledger_id AND patch.patch_ordinal=ordinal
            )
       )
       OR EXISTS(
           SELECT 1 FROM investigation_refiner_plan_patches patch
            WHERE patch.ledger_id=p_ledger_id
              AND patch.expected_previous_state_sha256<>(
                  CASE WHEN patch.patch_ordinal=0 THEN ledger.ledger_sha256
                       ELSE (SELECT prior.patch_sha256
                               FROM investigation_refiner_plan_patches prior
                              WHERE prior.ledger_id=p_ledger_id
                                AND prior.patch_ordinal=patch.patch_ordinal-1)
                  END
              )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_CHAIN_INVALID' USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM investigation_refiner_plan_patches patch
         WHERE patch.ledger_id=p_ledger_id
           AND patch.remaining_plan_payload_sha256<>
               investigation_refiner_payload_hash_v1('remaining_plan_patch',patch.remaining_plan_payload)
    ) OR EXISTS(
        SELECT 1 FROM investigation_refiner_plan_patches patch
         LEFT JOIN LATERAL (
             SELECT COUNT(*) AS member_count,
                    unified_investigation_exact_set_hash(
                        'investigation_refiner_active_realized_subtasks.v1',
                        COALESCE(array_agg(member.member_sha256 ORDER BY member.member_ordinal),ARRAY[]::TEXT[])
                    ) AS member_hash
               FROM investigation_refiner_plan_patch_members member
              WHERE member.patch_id=patch.patch_id
         ) actual ON TRUE
        WHERE patch.ledger_id=p_ledger_id
          AND ROW(patch.active_realized_subtask_count,patch.active_realized_subtask_set_sha256)
              IS DISTINCT FROM ROW(actual.member_count,actual.member_hash)
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_CENSUS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_active_realized_subtasks.v1',
               COALESCE(array_agg(member_sha256 ORDER BY member_ordinal),ARRAY[]::TEXT[])
           ) INTO actual_active_count,actual_active_hash
      FROM investigation_refiner_plan_patch_members WHERE patch_id=final_patch.patch_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_active_realized_subtasks.v1',
               COALESCE(array_agg(
                   'sha256:' || encode(digest(convert_to(concat_ws(':',
                       'golish.investigation_refiner_active_realized_subtask.v1',
                       subtask_id::TEXT,member_sha256),'UTF8'),'sha256'),'hex')
                   ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO runnable_count,runnable_hash
      FROM investigation_pentagi_subtasks
     WHERE task_plan_id=p_task_plan_id AND runnable;
    IF ROW(actual_active_count,actual_active_hash)
       IS DISTINCT FROM ROW(runnable_count,runnable_hash)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_FINAL_ACTIVE_SET_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    seal_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_seal.v1',p_seal_id::TEXT,p_ledger_id::TEXT,
        patch_count::TEXT,patch_hash,final_patch.patch_id::TEXT,final_patch.patch_sha256,
        actual_active_count::TEXT,actual_active_hash,actual_generator_count::TEXT,
        actual_generator_hash),'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
        event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256
    ) VALUES(
        p_result_barrier_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,
        next_event_ordinal,'result_barrier',primary_dispatch.worker_run_id,
        primary_dispatch.dispatch_receipt_id,seal_hash
    );
    INSERT INTO investigation_refiner_plan_ledger_seals(
        seal_id,stable_request_id,ledger_id,task_plan_id,result_barrier_pipeline_event_id,
        patch_count,patch_set_sha256,final_patch_id,final_patch_sha256,
        final_active_realized_subtask_count,final_active_realized_subtask_set_sha256,
        generator_subtask_count,generator_subtask_set_sha256,seal_sha256
    ) VALUES(
        p_seal_id,p_stable_request_id,p_ledger_id,p_task_plan_id,
        p_result_barrier_pipeline_event_id,patch_count,patch_hash,final_patch.patch_id,
        final_patch.patch_sha256,actual_active_count,actual_active_hash,
        actual_generator_count,actual_generator_hash,seal_hash
    ) RETURNING * INTO result;
    RETURN result;
END;
$$;

CREATE FUNCTION investigation_guard_refiner_plan_seal_v1()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    plan investigation_pentagi_task_plans%ROWTYPE;
    generator_event investigation_pentagi_pipeline_events%ROWTYPE;
    barrier_event investigation_pentagi_pipeline_events%ROWTYPE;
    final_patch investigation_refiner_plan_patches%ROWTYPE;
    generator_count BIGINT;
    generator_hash TEXT;
    expected_ledger_hash TEXT;
    patch_count BIGINT;
    patch_hash TEXT;
    active_count BIGINT;
    active_hash TEXT;
    runnable_count BIGINT;
    runnable_hash TEXT;
    expected_seal_hash TEXT;
BEGIN
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=NEW.ledger_id AND task_plan_id=NEW.task_plan_id FOR SHARE;
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    SELECT * INTO STRICT generator_event FROM investigation_pentagi_pipeline_events
     WHERE pipeline_event_id=ledger.generator_pipeline_event_id FOR SHARE;
    SELECT * INTO STRICT barrier_event FROM investigation_pentagi_pipeline_events
     WHERE pipeline_event_id=NEW.result_barrier_pipeline_event_id FOR SHARE;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_generator_subtasks.v1',
               COALESCE(array_agg(subtask_id::TEXT || ':' || member_sha256
                                  ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO generator_count,generator_hash
      FROM investigation_pentagi_subtasks WHERE task_plan_id=NEW.task_plan_id;
    expected_ledger_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_ledger.v1',ledger.ledger_id::TEXT,
        ledger.task_plan_id::TEXT,investigation_refiner_payload_hash_v1(
            'generator_manifest',ledger.generator_manifest
        ),generator_count::TEXT,generator_hash),'UTF8'),'sha256'),'hex');
    IF ROW(ledger.generator_manifest_sha256,ledger.generator_subtask_count,
           ledger.generator_subtask_set_sha256,ledger.ledger_sha256)
       IS DISTINCT FROM
       ROW(investigation_refiner_payload_hash_v1('generator_manifest',ledger.generator_manifest),
           generator_count,generator_hash,expected_ledger_hash)
       OR generator_event.task_plan_id<>NEW.task_plan_id
       OR generator_event.event_kind<>'generator_sealed'
       OR generator_event.subtask_id IS NOT NULL
       OR generator_event.event_sha256<>expected_ledger_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_GENERATOR_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO final_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=NEW.ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_plan_patches.v1',
               COALESCE(array_agg(patch_id::TEXT || ':' || patch_sha256
                                  ORDER BY patch_ordinal),ARRAY[]::TEXT[])
           ) INTO patch_count,patch_hash
      FROM investigation_refiner_plan_patches WHERE ledger_id=NEW.ledger_id;
    IF patch_count=0 OR final_patch.patch_ordinal<>patch_count-1
       OR EXISTS(
           SELECT 1 FROM generate_series(0,patch_count-1) ordinal
            WHERE NOT EXISTS(
                SELECT 1 FROM investigation_refiner_plan_patches patch
                 WHERE patch.ledger_id=NEW.ledger_id AND patch.patch_ordinal=ordinal
            )
       )
       OR EXISTS(
           SELECT 1
             FROM investigation_refiner_plan_patches patch
             JOIN investigation_pentagi_pipeline_events event
               ON event.pipeline_event_id=patch.refiner_pipeline_event_id
             LEFT JOIN LATERAL (
                 SELECT COUNT(*) AS member_count,
                        unified_investigation_exact_set_hash(
                            'investigation_refiner_active_realized_subtasks.v1',
                            COALESCE(array_agg(member.member_sha256 ORDER BY member.member_ordinal),ARRAY[]::TEXT[])
                        ) AS member_hash
                   FROM investigation_refiner_plan_patch_members member
                  WHERE member.patch_id=patch.patch_id
             ) actual ON TRUE
            WHERE patch.ledger_id=NEW.ledger_id
              AND (
                  patch.expected_previous_state_sha256<>(
                      CASE WHEN patch.patch_ordinal=0 THEN ledger.ledger_sha256
                           ELSE (SELECT prior.patch_sha256
                                   FROM investigation_refiner_plan_patches prior
                                  WHERE prior.ledger_id=NEW.ledger_id
                                    AND prior.patch_ordinal=patch.patch_ordinal-1)
                      END
                  )
                  OR patch.remaining_plan_payload_sha256<>
                     investigation_refiner_payload_hash_v1(
                         'remaining_plan_patch',patch.remaining_plan_payload
                     )
                  OR ROW(patch.active_realized_subtask_count,
                         patch.active_realized_subtask_set_sha256)
                     IS DISTINCT FROM ROW(actual.member_count,actual.member_hash)
                  OR patch.patch_sha256<>(
                      'sha256:' || encode(digest(convert_to(concat_ws(':',
                          'golish.investigation_refiner_plan_patch.v1',patch.patch_id::TEXT,
                          patch.ledger_id::TEXT,patch.patch_ordinal::TEXT,
                          patch.expected_previous_state_sha256,
                          patch.remaining_plan_payload_sha256,
                          patch.active_realized_subtask_count::TEXT,
                          patch.active_realized_subtask_set_sha256
                      ),'UTF8'),'sha256'),'hex')
                  )
                  OR event.task_plan_id<>NEW.task_plan_id
                  OR event.event_kind<>'refiner_patch'
                  OR event.subtask_id IS NOT NULL
                  OR event.event_sha256<>patch.patch_sha256
              )
       )
       OR EXISTS(
           SELECT 1
             FROM investigation_refiner_plan_patch_members member
             JOIN investigation_pentagi_subtasks subtask
               ON subtask.subtask_id=member.subtask_id AND subtask.task_plan_id=member.task_plan_id
            WHERE member.patch_id IN(
                SELECT patch_id FROM investigation_refiner_plan_patches
                 WHERE ledger_id=NEW.ledger_id
            ) AND (
                NOT subtask.runnable OR member.member_sha256<>(
                    'sha256:' || encode(digest(convert_to(concat_ws(':',
                        'golish.investigation_refiner_active_realized_subtask.v1',
                        subtask.subtask_id::TEXT,subtask.member_sha256
                    ),'UTF8'),'sha256'),'hex')
                )
            )
       )
       OR EXISTS(
           SELECT 1 FROM investigation_refiner_plan_patches patch
            WHERE patch.ledger_id=NEW.ledger_id AND patch.patch_ordinal>0
              AND EXISTS(
                  SELECT 1 FROM investigation_refiner_plan_patch_members prior_member
                   WHERE prior_member.patch_id=(
                       SELECT prior.patch_id FROM investigation_refiner_plan_patches prior
                        WHERE prior.ledger_id=NEW.ledger_id
                          AND prior.patch_ordinal=patch.patch_ordinal-1
                   ) AND NOT EXISTS(
                       SELECT 1 FROM investigation_refiner_plan_patch_members current_member
                        WHERE current_member.patch_id=patch.patch_id
                          AND current_member.subtask_id=prior_member.subtask_id
                   )
              )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_active_realized_subtasks.v1',
               COALESCE(array_agg(member_sha256 ORDER BY member_ordinal),ARRAY[]::TEXT[])
           ) INTO active_count,active_hash
      FROM investigation_refiner_plan_patch_members WHERE patch_id=final_patch.patch_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_refiner_active_realized_subtasks.v1',
               COALESCE(array_agg(
                   'sha256:' || encode(digest(convert_to(concat_ws(':',
                       'golish.investigation_refiner_active_realized_subtask.v1',
                       subtask_id::TEXT,member_sha256),'UTF8'),'sha256'),'hex')
                   ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           ) INTO runnable_count,runnable_hash
      FROM investigation_pentagi_subtasks
     WHERE task_plan_id=NEW.task_plan_id AND runnable;
    expected_seal_hash := 'sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_seal.v1',NEW.seal_id::TEXT,NEW.ledger_id::TEXT,
        patch_count::TEXT,patch_hash,final_patch.patch_id::TEXT,final_patch.patch_sha256,
        active_count::TEXT,active_hash,generator_count::TEXT,generator_hash),
        'UTF8'),'sha256'),'hex');
    IF ROW(active_count,active_hash) IS DISTINCT FROM ROW(runnable_count,runnable_hash)
       OR ROW(NEW.patch_count,NEW.patch_set_sha256,NEW.final_patch_id,
              NEW.final_patch_sha256,NEW.final_active_realized_subtask_count,
              NEW.final_active_realized_subtask_set_sha256,NEW.generator_subtask_count,
              NEW.generator_subtask_set_sha256,NEW.seal_sha256)
          IS DISTINCT FROM
          ROW(patch_count,patch_hash,final_patch.patch_id,final_patch.patch_sha256,
              active_count,active_hash,generator_count,generator_hash,expected_seal_hash)
       OR barrier_event.task_plan_id<>NEW.task_plan_id
       OR barrier_event.event_kind<>'result_barrier'
       OR barrier_event.subtask_id IS NOT NULL
       OR barrier_event.event_sha256<>expected_seal_hash
       OR barrier_event.event_ordinal<=(
           SELECT final_event.event_ordinal FROM investigation_pentagi_pipeline_events final_event
            WHERE final_event.pipeline_event_id=final_patch.refiner_pipeline_event_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_REFINER_SEAL_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_refiner_plan_ledger_seals_contract
BEFORE INSERT ON investigation_refiner_plan_ledger_seals
FOR EACH ROW EXECUTE FUNCTION investigation_guard_refiner_plan_seal_v1();

ALTER TABLE investigation_run_closure_v1_authorities
    ADD COLUMN refiner_ledger_count BIGINT NOT NULL DEFAULT 0 CHECK(refiner_ledger_count>=0),
    ADD COLUMN refiner_patch_count BIGINT NOT NULL DEFAULT 0 CHECK(refiner_patch_count>=0),
    ADD COLUMN refiner_active_realized_subtask_count BIGINT NOT NULL DEFAULT 0
        CHECK(refiner_active_realized_subtask_count>=0),
    ADD COLUMN refiner_member_set_sha256 TEXT NOT NULL
        DEFAULT 'sha256:0000000000000000000000000000000000000000000000000000000000000000'
        CHECK(refiner_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$');

CREATE FUNCTION investigation_guard_refiner_closure_census_v1()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task_count BIGINT;
    ledger_count BIGINT;
    seal_count BIGINT;
    patch_count BIGINT;
    active_count BIGINT;
    refiner_hash TEXT;
    delegation_hash TEXT;
BEGIN
    SELECT COUNT(*) INTO task_count
      FROM investigation_pentagi_task_plans WHERE authority_id=NEW.authority_id;
    SELECT COUNT(*) INTO ledger_count
      FROM investigation_refiner_plan_ledgers ledger
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=ledger.task_plan_id
     WHERE plan.authority_id=NEW.authority_id;
    SELECT COUNT(*),COALESCE(SUM(seal.patch_count),0),
           COALESCE(SUM(seal.final_active_realized_subtask_count),0)
      INTO seal_count,patch_count,active_count
      FROM investigation_refiner_plan_ledger_seals seal
      JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=seal.task_plan_id
     WHERE plan.authority_id=NEW.authority_id;
    IF task_count<>ledger_count OR task_count<>seal_count
       OR patch_count<>(
           SELECT COUNT(*) FROM investigation_refiner_plan_patches patch
           JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=patch.task_plan_id
           WHERE plan.authority_id=NEW.authority_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_REFINER_LEDGER_NOT_CLOSED' USING ERRCODE='23514';
    END IF;
    WITH refiner_members AS (
        SELECT 'ledger:' || ledger.ledger_id::TEXT AS member_key,ledger.ledger_sha256 AS member_hash
          FROM investigation_refiner_plan_ledgers ledger
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=ledger.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'patch:' || patch.patch_id::TEXT,patch.patch_sha256
          FROM investigation_refiner_plan_patches patch
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=patch.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'seal:' || seal.seal_id::TEXT,seal.seal_sha256
          FROM investigation_refiner_plan_ledger_seals seal
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=seal.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
    )
    SELECT unified_investigation_exact_set_hash(
               'investigation_refiner_closure.v1',
               COALESCE(array_agg(member_key || ':' || member_hash ORDER BY member_key),ARRAY[]::TEXT[])
           ) INTO refiner_hash FROM refiner_members;
    WITH delegation_members AS (
        SELECT 'plan:' || plan.task_plan_id::TEXT AS member_key,plan.task_plan_sha256 AS member_hash
          FROM investigation_pentagi_task_plans plan WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'subtask:' || subtask.subtask_id::TEXT,subtask.member_sha256
          FROM investigation_pentagi_subtasks subtask
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=subtask.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'dispatch:' || dispatch.dispatch_receipt_id::TEXT,dispatch.receipt_sha256
          FROM pentagi_logical_dispatch_receipts dispatch
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'attempt:' || attempt.dispatch_attempt_id::TEXT,
               unified_investigation_exact_set_hash(
                   'pentagi_dispatch_attempt.v1',ARRAY[attempt.fence_sha256,attempt.outcome,attempt.result_sha256]
               )
          FROM pentagi_logical_dispatch_attempts attempt
          JOIN pentagi_logical_dispatch_receipts dispatch
            ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'pipeline:' || pipeline.pipeline_event_id::TEXT,pipeline.event_sha256
          FROM investigation_pentagi_pipeline_events pipeline
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=pipeline.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
        UNION ALL
        SELECT 'census:' || census.census_seal_id::TEXT,census.seal_sha256
          FROM investigation_pentagi_delegation_census_seals census
          JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=census.task_plan_id
         WHERE plan.authority_id=NEW.authority_id
    )
    SELECT unified_investigation_exact_set_hash(
               'investigation_delegation_closure.v1',
               COALESCE(array_agg(member_key || ':' || member_hash ORDER BY member_key),ARRAY[]::TEXT[])
           ) INTO delegation_hash FROM delegation_members;
    IF NEW.delegation_member_set_sha256<>delegation_hash THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_REFINER_DELEGATION_HASH_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    NEW.refiner_ledger_count := ledger_count;
    NEW.refiner_patch_count := patch_count;
    NEW.refiner_active_realized_subtask_count := active_count;
    NEW.refiner_member_set_sha256 := refiner_hash;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_run_closure_v1_refiner_census
BEFORE INSERT ON investigation_run_closure_v1_authorities
FOR EACH ROW EXECUTE FUNCTION investigation_guard_refiner_closure_census_v1();
