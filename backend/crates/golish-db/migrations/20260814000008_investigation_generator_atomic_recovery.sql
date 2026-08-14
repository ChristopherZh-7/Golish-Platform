-- Generator/Refiner authority must follow the current Asset Primary chain.
-- Historical schedule and rearm rows remain immutable audit evidence, but a
-- task plan may legitimately have frozen its Primary dispatch at any ancestor
-- of the current execution shell on the same source schedule and message
-- chain.

CREATE FUNCTION investigation_refiner_primary_source_is_current_v3(
    p_task_plan_id UUID,
    p_stage_work_item_id UUID,
    p_worker_run_id UUID
) RETURNS BOOLEAN LANGUAGE SQL STABLE STRICT AS $$
    SELECT EXISTS(
        SELECT 1
          FROM investigation_pentagi_task_plans plan
          JOIN investigation_asset_primary_current_authorities current_authority
            ON current_authority.stage_team_plan_id=plan.stage_team_plan_id
           AND current_authority.operation_id=plan.operation_id
           AND current_authority.stage_execution_id=plan.stage_execution_id
           AND current_authority.stage_run_unit_id=plan.stage_run_unit_id
           AND current_authority.scope_snapshot_id=plan.scope_snapshot_id
           AND current_authority.organization_id=plan.organization_id
         WHERE plan.task_plan_id=p_task_plan_id
           AND (
                EXISTS(
                    SELECT 1 FROM investigation_asset_primary_schedules source_schedule
                     WHERE source_schedule.schedule_receipt_id=
                               current_authority.source_schedule_receipt_id
                       AND source_schedule.status='applied'
                       AND source_schedule.schedule_contract='primary_dynamic_v2'
                       AND source_schedule.primary_work_item_id=p_stage_work_item_id
                       AND source_schedule.primary_worker_run_id=p_worker_run_id
                )
                OR EXISTS(
                    SELECT 1 FROM investigation_asset_primary_rearms ancestor_rearm
                     WHERE ancestor_rearm.source_schedule_receipt_id=
                               current_authority.source_schedule_receipt_id
                       AND ancestor_rearm.status='applied'
                       AND ancestor_rearm.primary_message_chain_id=
                           current_authority.primary_message_chain_id
                       AND ancestor_rearm.primary_work_item_id=p_stage_work_item_id
                       AND ancestor_rearm.primary_worker_run_id=p_worker_run_id
                )
           )
    )
$$;

CREATE TABLE investigation_generator_source_receipts (
    source_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_task_plans(task_plan_id) ON DELETE RESTRICT,
    ledger_id UUID NOT NULL UNIQUE
        REFERENCES investigation_refiner_plan_ledgers(ledger_id) ON DELETE RESTRICT,
    generator_pipeline_event_id UUID NOT NULL UNIQUE
        REFERENCES investigation_pentagi_pipeline_events(pipeline_event_id) ON DELETE RESTRICT,
    source_tool_call_id UUID NOT NULL UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT,
    source_provider_call_id TEXT NOT NULL CHECK(BTRIM(source_provider_call_id)<>''),
    source_attempt_epoch BIGINT NOT NULL CHECK(source_attempt_epoch>=0),
    source_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    source_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    current_consumer_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    current_consumer_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    current_consumer_lease_token UUID NOT NULL,
    current_consumer_attempt_epoch BIGINT NOT NULL CHECK(current_consumer_attempt_epoch>=0),
    current_consumer_checkpoint_version BIGINT NOT NULL
        CHECK(current_consumer_checkpoint_version>=0),
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    canonical_result_sha256 TEXT NOT NULL
        CHECK(canonical_result_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    adopted_subtask_count BIGINT NOT NULL CHECK(adopted_subtask_count>=0),
    adopted_subtask_set_sha256 TEXT NOT NULL
        CHECK(adopted_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_kind TEXT NOT NULL CHECK(receipt_kind IN('materialized','orphan_adoption')),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'applied' CHECK(status='applied'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                organization_id)
        REFERENCES investigation_pentagi_task_plans(
            task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
            organization_id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_generator_source_receipt_sha256_v1(
    p_source_receipt_id UUID,p_stable_request_id UUID,p_task_plan_id UUID,
    p_ledger_id UUID,p_generator_pipeline_event_id UUID,p_source_tool_call_id UUID,
    p_source_provider_call_id TEXT,p_source_attempt_epoch BIGINT,
    p_source_work_item_id UUID,p_source_worker_run_id UUID,
    p_current_consumer_work_item_id UUID,p_current_consumer_worker_run_id UUID,
    p_current_consumer_lease_token UUID,p_current_consumer_attempt_epoch BIGINT,
    p_current_consumer_checkpoint_version BIGINT,
    p_operation_id UUID,p_stage_execution_id UUID,p_stage_run_unit_id UUID,
    p_scope_snapshot_id UUID,p_organization_id UUID,p_canonical_result_sha256 TEXT,
    p_adopted_subtask_count BIGINT,p_adopted_subtask_set_sha256 TEXT,
    p_receipt_kind TEXT
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_generator_source_receipt.v1',
        'source_receipt_id',p_source_receipt_id,
        'stable_request_id',p_stable_request_id,
        'task_plan_id',p_task_plan_id,'ledger_id',p_ledger_id,
        'generator_pipeline_event_id',p_generator_pipeline_event_id,
        'source_tool_call_id',p_source_tool_call_id,
        'source_provider_call_id',p_source_provider_call_id,
        'source_attempt_epoch',p_source_attempt_epoch,
        'source_work_item_id',p_source_work_item_id,
        'source_worker_run_id',p_source_worker_run_id,
        'current_consumer_work_item_id',p_current_consumer_work_item_id,
        'current_consumer_worker_run_id',p_current_consumer_worker_run_id,
        'current_consumer_lease_token',p_current_consumer_lease_token,
        'current_consumer_attempt_epoch',p_current_consumer_attempt_epoch,
        'current_consumer_checkpoint_version',p_current_consumer_checkpoint_version,
        'operation_id',p_operation_id,'stage_execution_id',p_stage_execution_id,
        'stage_run_unit_id',p_stage_run_unit_id,'scope_snapshot_id',p_scope_snapshot_id,
        'organization_id',p_organization_id,
        'canonical_result_sha256',p_canonical_result_sha256,
        'adopted_subtask_count',p_adopted_subtask_count,
        'adopted_subtask_set_sha256',p_adopted_subtask_set_sha256,
        'receipt_kind',p_receipt_kind
    )::TEXT)
$$;

CREATE FUNCTION enforce_investigation_generator_source_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    source_call tool_calls%ROWTYPE;
    source_worker stage_worker_runs%ROWTYPE;
    consumer_worker stage_worker_runs%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    actual_count BIGINT;
    actual_adopted_set_sha256 TEXT;
    actual_generator_set_sha256 TEXT;
    expected_receipt_sha256 TEXT;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_GENERATOR_ADOPTION_RECEIPT_APPEND_ONLY';
    END IF;
    SELECT * INTO STRICT source_call FROM tool_calls
     WHERE id=NEW.source_tool_call_id FOR SHARE;
    SELECT * INTO STRICT source_worker FROM stage_worker_runs
     WHERE id=NEW.source_worker_run_id FOR SHARE;
    SELECT * INTO STRICT consumer_worker FROM stage_worker_runs
     WHERE id=NEW.current_consumer_worker_run_id FOR SHARE;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=NEW.ledger_id AND task_plan_id=NEW.task_plan_id FOR SHARE;
    SELECT COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_generator_adopted_subtasks.v1',
               COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                                  ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[])),
           unified_investigation_exact_set_hash(
               'investigation_refiner_generator_subtasks.v2',
               COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                                  ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[]))
      INTO actual_count,actual_adopted_set_sha256,actual_generator_set_sha256
      FROM investigation_pentagi_subtasks subtask
     WHERE subtask.task_plan_id=NEW.task_plan_id;
    expected_receipt_sha256:=investigation_generator_source_receipt_sha256_v1(
        NEW.source_receipt_id,NEW.stable_request_id,NEW.task_plan_id,NEW.ledger_id,
        NEW.generator_pipeline_event_id,NEW.source_tool_call_id,
        NEW.source_provider_call_id,NEW.source_attempt_epoch,NEW.source_work_item_id,
        NEW.source_worker_run_id,NEW.current_consumer_work_item_id,
        NEW.current_consumer_worker_run_id,NEW.current_consumer_lease_token,
        NEW.current_consumer_attempt_epoch,NEW.current_consumer_checkpoint_version,
        NEW.operation_id,NEW.stage_execution_id,
        NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
        NEW.canonical_result_sha256,NEW.adopted_subtask_count,
        NEW.adopted_subtask_set_sha256,NEW.receipt_kind);
    IF source_call.call_id<>NEW.source_provider_call_id
       OR NEW.source_receipt_id<>uuid_generate_v5(
          NEW.task_plan_id,
          E'unified-investigation.v1\n' ||
          CASE WHEN NEW.receipt_kind='materialized'
               THEN E'generator-source-receipt\n'
               ELSE E'generator-orphan-adoption-receipt\n' END ||
          NEW.source_tool_call_id::TEXT)
       OR (NEW.receipt_kind='orphan_adoption' AND
           NEW.stable_request_id<>uuid_generate_v5(
               NEW.task_plan_id,
               E'unified-investigation.v1\nadopt-generator-orphan\n' ||
               NEW.source_tool_call_id::TEXT))
       OR source_call.worker_run_id<>NEW.source_worker_run_id
       OR source_call.operation_id<>NEW.operation_id
       OR source_call.stage_execution_id<>NEW.stage_execution_id
       OR source_call.stage_run_unit_id<>NEW.stage_run_unit_id
       OR source_call.organization_id<>NEW.organization_id
       OR source_call.attempt_epoch<>NEW.source_attempt_epoch
       OR source_call.name<>'submit_result' OR source_call.status<>'finished'
       OR source_call.result IS NULL
       OR source_call.result::JSONB->>'status'<>'result submitted'
       OR NOT(source_call.args ? 'result')
       OR tool_truth_sha256((source_call.args->'result')::TEXT)<>
          NEW.canonical_result_sha256
       OR source_worker.work_item_id<>NEW.source_work_item_id
       OR NOT investigation_refiner_primary_source_is_current_v3(
           NEW.task_plan_id,NEW.source_work_item_id,NEW.source_worker_run_id)
       OR consumer_worker.work_item_id<>NEW.current_consumer_work_item_id
       OR consumer_worker.status<>'running'
       OR consumer_worker.lease_token<>NEW.current_consumer_lease_token
       OR consumer_worker.attempt_epoch<>NEW.current_consumer_attempt_epoch
       OR consumer_worker.checkpoint_version<>NEW.current_consumer_checkpoint_version
       OR consumer_worker.active_tool_call_id IS NOT NULL
       OR NOT EXISTS(
           SELECT 1 FROM investigation_pentagi_task_plans receipt_plan
           JOIN investigation_asset_primary_current_authorities current_authority
             ON current_authority.stage_team_plan_id=receipt_plan.stage_team_plan_id
            AND current_authority.operation_id=receipt_plan.operation_id
            AND current_authority.stage_execution_id=receipt_plan.stage_execution_id
            AND current_authority.stage_run_unit_id=receipt_plan.stage_run_unit_id
            AND current_authority.scope_snapshot_id=receipt_plan.scope_snapshot_id
            AND current_authority.organization_id=receipt_plan.organization_id
          WHERE receipt_plan.task_plan_id=NEW.task_plan_id
            AND current_authority.primary_work_item_id=NEW.current_consumer_work_item_id
            AND current_authority.primary_worker_run_id=NEW.current_consumer_worker_run_id)
       OR ledger.generator_pipeline_event_id<>NEW.generator_pipeline_event_id
       OR ledger.generator_manifest<>source_call.args->'result'
       OR ledger.generator_manifest_sha256<>
          investigation_refiner_payload_hash_v1(
              'generator_manifest',source_call.args->'result')
       OR ledger.generator_subtask_count<>actual_count
       OR ledger.generator_subtask_set_sha256<>actual_generator_set_sha256
       OR NEW.adopted_subtask_count<>actual_count
       OR NEW.adopted_subtask_set_sha256<>actual_adopted_set_sha256
       OR NEW.receipt_sha256<>expected_receipt_sha256
    THEN RAISE EXCEPTION 'INVESTIGATION_GENERATOR_ADOPTION_AUTHORITY_MISMATCH'; END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_generator_source_receipts_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_generator_source_receipts
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_generator_source_receipt();

-- Replace only the V2 functions.  V1 is retained for historical read/replay.
CREATE OR REPLACE FUNCTION create_investigation_refiner_plan_ledger_v2(
    p_ledger_id UUID,p_stable_request_id UUID,p_task_plan_id UUID,
    p_generator_pipeline_event_id UUID,p_generator_manifest JSONB
) RETURNS investigation_refiner_plan_ledgers LANGUAGE plpgsql AS $$
DECLARE
    existing investigation_refiner_plan_ledgers%ROWTYPE;
    plan investigation_pentagi_task_plans%ROWTYPE;
    dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    result investigation_refiner_plan_ledgers%ROWTYPE;
    manifest_hash TEXT; subtask_count BIGINT; subtask_hash TEXT;
    ledger_hash TEXT; next_event_ordinal BIGINT;
BEGIN
    IF p_generator_manifest IS NULL OR jsonb_typeof(p_generator_manifest)<>'object'
       OR p_generator_manifest='{}'::JSONB
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_GENERATOR_MANIFEST_INVALID' USING ERRCODE='23514'; END IF;
    SELECT * INTO existing FROM investigation_refiner_plan_ledgers
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.ledger_id,existing.task_plan_id,existing.generator_pipeline_event_id,
               existing.generator_manifest)
           IS DISTINCT FROM ROW(p_ledger_id,p_task_plan_id,p_generator_pipeline_event_id,
                                p_generator_manifest)
        THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_LEDGER_REPLAY_MISMATCH' USING ERRCODE='23514'; END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=p_task_plan_id AND status='open' FOR UPDATE;
    SELECT candidate.* INTO STRICT dispatch
      FROM pentagi_logical_dispatch_receipts candidate
     WHERE candidate.task_plan_id=p_task_plan_id AND candidate.actor_kind='primary'
       AND investigation_refiner_primary_source_is_current_v3(
           p_task_plan_id,candidate.stage_work_item_id,candidate.worker_run_id)
     ORDER BY candidate.dispatch_ordinal DESC LIMIT 1 FOR SHARE;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
        'investigation_refiner_generator_subtasks.v2',
        COALESCE(array_agg(subtask_id::TEXT || ':' || member_sha256 ORDER BY subtask_ordinal),
                 ARRAY[]::TEXT[]))
      INTO subtask_count,subtask_hash FROM investigation_pentagi_subtasks
     WHERE task_plan_id=p_task_plan_id;
    manifest_hash:=investigation_refiner_payload_hash_v1('generator_manifest',p_generator_manifest);
    ledger_hash:='sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_ledger.v2',p_ledger_id::TEXT,p_task_plan_id::TEXT,
        manifest_hash,subtask_count::TEXT,subtask_hash),'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,event_kind,
        actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
    VALUES(p_generator_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,
        next_event_ordinal,'generator_sealed',dispatch.worker_run_id,
        dispatch.dispatch_receipt_id,ledger_hash);
    INSERT INTO investigation_refiner_plan_ledgers(
        ledger_id,stable_request_id,task_plan_id,authority_id,operation_id,
        stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
        scope_snapshot_id,organization_id,generator_pipeline_event_id,generator_manifest,
        generator_manifest_sha256,generator_subtask_count,generator_subtask_set_sha256,
        ledger_sha256,ledger_contract)
    VALUES(p_ledger_id,p_stable_request_id,p_task_plan_id,plan.authority_id,plan.operation_id,
        plan.stage_execution_id,plan.owning_stage_run_request_id,plan.stage_run_unit_id,
        plan.scope_snapshot_id,plan.organization_id,p_generator_pipeline_event_id,
        p_generator_manifest,manifest_hash,subtask_count,subtask_hash,ledger_hash,
        'dynamic_ordered_v2')
    RETURNING * INTO result;
    RETURN result;
END;
$$;

CREATE OR REPLACE FUNCTION append_investigation_refiner_plan_patch_v2(
    p_patch_id UUID,p_stable_request_id UUID,p_ledger_id UUID,p_task_plan_id UUID,
    p_refiner_pipeline_event_id UUID,p_expected_previous_state_sha256 TEXT,
    p_remaining_plan_payload JSONB,p_ordered_active_subtask_ids UUID[]
) RETURNS investigation_refiner_plan_patches LANGUAGE plpgsql AS $$
DECLARE
    existing investigation_refiner_plan_patches%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    primary_dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    previous_patch investigation_refiner_plan_patches%ROWTYPE;
    result investigation_refiner_plan_patches%ROWTYPE;
    patch_ordinal BIGINT; previous_hash TEXT; payload_hash TEXT;
    active_count BIGINT; active_hash TEXT; patch_hash TEXT; next_event_ordinal BIGINT;
BEGIN
    IF p_remaining_plan_payload IS NULL OR jsonb_typeof(p_remaining_plan_payload)<>'object'
       OR p_ordered_active_subtask_ids IS NULL
       OR cardinality(p_ordered_active_subtask_ids)<>
          cardinality(ARRAY(SELECT DISTINCT value FROM unnest(p_ordered_active_subtask_ids) value))
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_PATCH_PAYLOAD_INVALID' USING ERRCODE='23514'; END IF;
    SELECT * INTO existing FROM investigation_refiner_plan_patches
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.patch_id,existing.ledger_id,existing.task_plan_id,
               existing.refiner_pipeline_event_id,existing.expected_previous_state_sha256,
               existing.remaining_plan_payload)
           IS DISTINCT FROM ROW(p_patch_id,p_ledger_id,p_task_plan_id,
               p_refiner_pipeline_event_id,p_expected_previous_state_sha256,p_remaining_plan_payload)
           OR (SELECT COALESCE(array_agg(member.subtask_id ORDER BY member.member_ordinal),ARRAY[]::UUID[])
                 FROM investigation_refiner_plan_patch_members member
                WHERE member.patch_id=existing.patch_id)<>p_ordered_active_subtask_ids
        THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_PATCH_REPLAY_MISMATCH' USING ERRCODE='23514'; END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=p_ledger_id AND task_plan_id=p_task_plan_id FOR UPDATE;
    IF EXISTS(SELECT 1 FROM investigation_refiner_plan_ledger_seals WHERE ledger_id=p_ledger_id)
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_LEDGER_ALREADY_SEALED' USING ERRCODE='23514'; END IF;
    SELECT candidate.* INTO STRICT primary_dispatch
      FROM pentagi_logical_dispatch_receipts candidate
     WHERE candidate.task_plan_id=p_task_plan_id AND candidate.actor_kind='primary'
       AND investigation_refiner_primary_source_is_current_v3(
           p_task_plan_id,candidate.stage_work_item_id,candidate.worker_run_id)
     ORDER BY candidate.dispatch_ordinal DESC LIMIT 1 FOR SHARE;
    IF EXISTS(SELECT 1 FROM unnest(p_ordered_active_subtask_ids) requested
               WHERE NOT EXISTS(SELECT 1 FROM investigation_pentagi_subtasks subtask
                                 WHERE subtask.task_plan_id=p_task_plan_id
                                   AND subtask.subtask_id=requested AND subtask.runnable))
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_ASSET_AUTHORITY_MISMATCH' USING ERRCODE='23514'; END IF;
    SELECT * INTO previous_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=p_ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    patch_ordinal:=COALESCE(previous_patch.patch_ordinal+1,0);
    previous_hash:=COALESCE(previous_patch.patch_sha256,ledger.ledger_sha256);
    IF p_expected_previous_state_sha256<>previous_hash
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_PATCH_PREVIOUS_STATE_CAS_MISMATCH' USING ERRCODE='23514'; END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
        'investigation_refiner_active_realized_subtasks.v2',
        COALESCE(array_agg('sha256:' || encode(digest(convert_to(concat_ws(':',
            'golish.investigation_refiner_active_realized_subtask.v2',subtask.subtask_id::TEXT,
            subtask.member_sha256,requested.ordinality::TEXT),'UTF8'),'sha256'),'hex')
            ORDER BY requested.ordinality),ARRAY[]::TEXT[]))
      INTO active_count,active_hash
      FROM unnest(p_ordered_active_subtask_ids) WITH ORDINALITY requested(subtask_id,ordinality)
      JOIN investigation_pentagi_subtasks subtask
        ON subtask.task_plan_id=p_task_plan_id AND subtask.subtask_id=requested.subtask_id;
    payload_hash:=investigation_refiner_payload_hash_v1('remaining_plan_patch',p_remaining_plan_payload);
    patch_hash:='sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_patch.v2',p_patch_id::TEXT,p_ledger_id::TEXT,
        patch_ordinal::TEXT,previous_hash,payload_hash,active_count::TEXT,active_hash),
        'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,event_kind,
        actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
    VALUES(p_refiner_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,next_event_ordinal,
        'refiner_patch',primary_dispatch.worker_run_id,primary_dispatch.dispatch_receipt_id,patch_hash);
    INSERT INTO investigation_refiner_plan_patches(
        patch_id,stable_request_id,ledger_id,task_plan_id,patch_ordinal,
        refiner_pipeline_event_id,expected_previous_state_sha256,remaining_plan_payload,
        remaining_plan_payload_sha256,active_realized_subtask_count,
        active_realized_subtask_set_sha256,patch_sha256,patch_contract)
    VALUES(p_patch_id,p_stable_request_id,p_ledger_id,p_task_plan_id,patch_ordinal,
        p_refiner_pipeline_event_id,previous_hash,p_remaining_plan_payload,payload_hash,
        active_count,active_hash,patch_hash,'dynamic_ordered_v2') RETURNING * INTO result;
    INSERT INTO investigation_refiner_plan_patch_members(
        patch_member_id,patch_id,task_plan_id,subtask_id,member_ordinal,member_sha256)
    SELECT gen_random_uuid(),p_patch_id,p_task_plan_id,subtask.subtask_id,
           (requested.ordinality-1)::INTEGER,
           'sha256:' || encode(digest(convert_to(concat_ws(':',
               'golish.investigation_refiner_active_realized_subtask.v2',subtask.subtask_id::TEXT,
               subtask.member_sha256,requested.ordinality::TEXT),'UTF8'),'sha256'),'hex')
      FROM unnest(p_ordered_active_subtask_ids) WITH ORDINALITY requested(subtask_id,ordinality)
      JOIN investigation_pentagi_subtasks subtask
        ON subtask.task_plan_id=p_task_plan_id AND subtask.subtask_id=requested.subtask_id
     ORDER BY requested.ordinality;
    RETURN result;
END;
$$;

CREATE OR REPLACE FUNCTION seal_investigation_refiner_plan_ledger_v2(
    p_seal_id UUID,p_stable_request_id UUID,p_ledger_id UUID,p_task_plan_id UUID,
    p_result_barrier_pipeline_event_id UUID,p_expected_final_patch_sha256 TEXT
) RETURNS investigation_refiner_plan_ledger_seals LANGUAGE plpgsql AS $$
DECLARE
    existing investigation_refiner_plan_ledger_seals%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    final_patch investigation_refiner_plan_patches%ROWTYPE;
    result investigation_refiner_plan_ledger_seals%ROWTYPE;
    patch_count BIGINT; patch_hash TEXT; seal_hash TEXT; next_event_ordinal BIGINT;
BEGIN
    SELECT * INTO existing FROM investigation_refiner_plan_ledger_seals
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF ROW(existing.seal_id,existing.ledger_id,existing.task_plan_id,
               existing.result_barrier_pipeline_event_id,existing.final_patch_sha256)
           IS DISTINCT FROM ROW(p_seal_id,p_ledger_id,p_task_plan_id,
               p_result_barrier_pipeline_event_id,p_expected_final_patch_sha256)
        THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_SEAL_REPLAY_MISMATCH' USING ERRCODE='23514'; END IF;
        RETURN existing;
    END IF;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=p_ledger_id AND task_plan_id=p_task_plan_id FOR UPDATE;
    SELECT candidate.* INTO STRICT dispatch
      FROM pentagi_logical_dispatch_receipts candidate
     WHERE candidate.task_plan_id=p_task_plan_id AND candidate.actor_kind='primary'
       AND investigation_refiner_primary_source_is_current_v3(
           p_task_plan_id,candidate.stage_work_item_id,candidate.worker_run_id)
     ORDER BY candidate.dispatch_ordinal DESC LIMIT 1 FOR SHARE;
    SELECT * INTO final_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=p_ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    IF NOT FOUND OR final_patch.patch_sha256<>p_expected_final_patch_sha256
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_FINAL_PATCH_CAS_MISMATCH' USING ERRCODE='23514'; END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash('investigation_refiner_plan_patches.v2',
        COALESCE(array_agg(patch_id::TEXT || ':' || patch_sha256 ORDER BY patch_ordinal),ARRAY[]::TEXT[]))
      INTO patch_count,patch_hash FROM investigation_refiner_plan_patches WHERE ledger_id=p_ledger_id;
    IF final_patch.patch_ordinal<>patch_count-1 OR EXISTS(
        SELECT 1 FROM investigation_refiner_plan_patches patch
         WHERE patch.ledger_id=p_ledger_id AND patch.expected_previous_state_sha256<>
            CASE WHEN patch.patch_ordinal=0 THEN ledger.ledger_sha256 ELSE
              (SELECT prior.patch_sha256 FROM investigation_refiner_plan_patches prior
                WHERE prior.ledger_id=p_ledger_id AND prior.patch_ordinal=patch.patch_ordinal-1) END)
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_PATCH_CHAIN_INVALID' USING ERRCODE='23514'; END IF;
    seal_hash:='sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_seal.v2',p_seal_id::TEXT,p_ledger_id::TEXT,
        patch_count::TEXT,patch_hash,final_patch.patch_id::TEXT,final_patch.patch_sha256,
        final_patch.active_realized_subtask_count::TEXT,
        final_patch.active_realized_subtask_set_sha256),'UTF8'),'sha256'),'hex');
    SELECT COALESCE(MAX(event_ordinal)+1,0) INTO next_event_ordinal
      FROM investigation_pentagi_pipeline_events WHERE task_plan_id=p_task_plan_id;
    INSERT INTO investigation_pentagi_pipeline_events(
        pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,event_kind,
        actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
    VALUES(p_result_barrier_pipeline_event_id,p_stable_request_id,p_task_plan_id,NULL,
        next_event_ordinal,'result_barrier',dispatch.worker_run_id,dispatch.dispatch_receipt_id,seal_hash);
    INSERT INTO investigation_refiner_plan_ledger_seals(
        seal_id,stable_request_id,ledger_id,task_plan_id,result_barrier_pipeline_event_id,
        patch_count,patch_set_sha256,final_patch_id,final_patch_sha256,
        final_active_realized_subtask_count,final_active_realized_subtask_set_sha256,
        generator_subtask_count,generator_subtask_set_sha256,seal_sha256,seal_contract)
    VALUES(p_seal_id,p_stable_request_id,p_ledger_id,p_task_plan_id,
        p_result_barrier_pipeline_event_id,patch_count,patch_hash,final_patch.patch_id,
        final_patch.patch_sha256,final_patch.active_realized_subtask_count,
        final_patch.active_realized_subtask_set_sha256,ledger.generator_subtask_count,
        ledger.generator_subtask_set_sha256,seal_hash,'dynamic_ordered_v2') RETURNING * INTO result;
    RETURN result;
END;
$$;
