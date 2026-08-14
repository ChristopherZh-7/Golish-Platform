-- Cut new Investigation asset scheduling over from the historical fixed
-- four-role roster to one durable Primary which may request zero or more
-- asset-bound children. Existing fixed-roster receipts remain immutable and
-- replayable for migration-forward audit/recovery, but never authorize a new
-- runtime entry.

ALTER TABLE investigation_asset_primary_schedules
    ADD COLUMN schedule_round INTEGER NOT NULL DEFAULT 0 CHECK(schedule_round>=0),
    ADD COLUMN schedule_contract TEXT NOT NULL DEFAULT 'fixed_roster_v1'
        CHECK(schedule_contract IN('fixed_roster_v1','primary_dynamic_v2')),
    ALTER COLUMN browser_work_item_id DROP NOT NULL,
    ALTER COLUMN researcher_work_item_id DROP NOT NULL,
    ALTER COLUMN pentester_work_item_id DROP NOT NULL,
    ALTER COLUMN adviser_work_item_id DROP NOT NULL,
    ALTER COLUMN roster_set_sha256 DROP NOT NULL;

DO $$
DECLARE
    legacy_unique_name TEXT;
BEGIN
    SELECT constraint_row.conname INTO STRICT legacy_unique_name
      FROM pg_constraint constraint_row
     WHERE constraint_row.conrelid='investigation_asset_primary_schedules'::REGCLASS
       AND constraint_row.contype='u'
       AND pg_get_constraintdef(constraint_row.oid)=
           'UNIQUE (asset_lane_id, evolution_epoch)';
    EXECUTE format(
        'ALTER TABLE investigation_asset_primary_schedules DROP CONSTRAINT %I',
        legacy_unique_name
    );
END;
$$;
ALTER TABLE investigation_asset_primary_schedules
    ADD CONSTRAINT investigation_asset_primary_schedules_lane_epoch_round_key
    UNIQUE(asset_lane_id,evolution_epoch,schedule_round);
CREATE UNIQUE INDEX investigation_asset_primary_dynamic_lane_epoch_key
    ON investigation_asset_primary_schedules(asset_lane_id,evolution_epoch)
    WHERE schedule_contract='primary_dynamic_v2';
ALTER TABLE investigation_asset_primary_schedules
    ALTER COLUMN schedule_contract SET DEFAULT 'primary_dynamic_v2';

CREATE FUNCTION investigation_asset_primary_dynamic_schedule_receipt_sha256(
    requested_asset_lane_id UUID,
    requested_target_id UUID,
    requested_asset_context_sha256 TEXT,
    requested_evolution_epoch INTEGER,
    requested_schedule_round INTEGER,
    requested_stage_team_plan_id UUID,
    requested_resume_dispatch_epoch BIGINT,
    requested_primary_work_item_id UUID,
    requested_primary_worker_run_id UUID,
    requested_primary_message_chain_id UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_asset_primary_dynamic_schedule_receipt.v2',
        'asset_lane_id',requested_asset_lane_id,
        'target_id',requested_target_id,
        'asset_context_sha256',requested_asset_context_sha256,
        'evolution_epoch',requested_evolution_epoch,
        'schedule_round',requested_schedule_round,
        'stage_team_plan_id',requested_stage_team_plan_id,
        'resume_dispatch_epoch',requested_resume_dispatch_epoch,
        'primary_work_item_id',requested_primary_work_item_id,
        'primary_worker_run_id',requested_primary_worker_run_id,
        'primary_message_chain_id',requested_primary_message_chain_id
    )::TEXT)
$$;

CREATE OR REPLACE FUNCTION enforce_investigation_asset_primary_schedule()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    lane investigation_asset_lanes%ROWTYPE;
    expected_receipt_id UUID;
    expected_primary_work_item_id UUID;
    expected_primary_worker_run_id UUID;
    expected_primary_message_chain_id UUID;
    expected_receipt_sha256 TEXT;
    roster_exact BOOLEAN := FALSE;
    primary_exact BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_APPEND_ONLY';
    END IF;
    SELECT * INTO STRICT plan FROM stage_team_plans
     WHERE id=NEW.stage_team_plan_id FOR SHARE;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;

    expected_primary_message_chain_id := uuid_generate_v5(
        NEW.asset_lane_id,'investigation-asset-primary-chain-v1');
    IF NEW.schedule_contract='primary_dynamic_v2' THEN
        expected_receipt_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-dynamic-schedule-receipt-v2:' ||
            NEW.evolution_epoch::TEXT || ':' || NEW.schedule_round::TEXT);
        expected_primary_work_item_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-work-item-v2:' ||
            NEW.evolution_epoch::TEXT || ':' || NEW.schedule_round::TEXT);
        expected_primary_worker_run_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-worker-v2:' ||
            NEW.evolution_epoch::TEXT || ':' || NEW.schedule_round::TEXT);
        expected_receipt_sha256 := investigation_asset_primary_dynamic_schedule_receipt_sha256(
            NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
            NEW.schedule_round,NEW.stage_team_plan_id,NEW.resume_dispatch_epoch,
            NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id);
    ELSE
        expected_receipt_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-schedule-receipt-v1:' ||
            NEW.evolution_epoch::TEXT);
        expected_primary_work_item_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-work-item-v1:' ||
            NEW.evolution_epoch::TEXT);
        expected_primary_worker_run_id := uuid_generate_v5(
            NEW.asset_lane_id,'investigation-asset-primary-worker-v1:' ||
            NEW.evolution_epoch::TEXT);
        expected_receipt_sha256 := investigation_asset_primary_schedule_receipt_sha256(
            NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
            NEW.stage_team_plan_id,NEW.resume_dispatch_epoch,NEW.primary_work_item_id,
            NEW.primary_worker_run_id,NEW.primary_message_chain_id);
    END IF;

    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
           OR NEW.schedule_receipt_id<>expected_receipt_id
           OR NEW.stable_request_id<>uuid_generate_v5(
                expected_receipt_id,'investigation-asset-primary-schedule-request-v1')
           OR NEW.primary_work_item_id<>expected_primary_work_item_id
           OR NEW.primary_worker_run_id<>expected_primary_worker_run_id
           OR NEW.primary_message_chain_id<>expected_primary_message_chain_id
           OR NEW.receipt_sha256<>expected_receipt_sha256
           OR plan.operation_id<>NEW.operation_id
           OR plan.stage_execution_id<>NEW.stage_execution_id
           OR plan.stage_run_unit_id<>NEW.stage_run_unit_id
           OR plan.scope_snapshot_id<>NEW.scope_snapshot_id
           OR plan.organization_id<>NEW.organization_id
           OR plan.stage_kind<>'investigation'
           OR plan.dynamic_request_policy->>'coordination_mode'<>'investigation_task_orchestrator'
           OR plan.dispatch_epoch<>NEW.source_dispatch_epoch
           OR plan.row_version<>NEW.source_plan_row_version
           OR plan.requests_closed_at IS NULL
           OR plan.final_submitter_worker_run_id IS NOT NULL
           OR lane.operation_id<>NEW.operation_id
           OR lane.stage_execution_id<>NEW.stage_execution_id
           OR lane.scope_snapshot_id<>NEW.scope_snapshot_id
           OR lane.organization_id<>NEW.organization_id
           OR lane.target_id<>NEW.target_id
           OR lane.target_identity_sha256<>NEW.asset_context_sha256
           OR lane.evolution_epoch<>NEW.evolution_epoch
           OR lane.state<>'analyzing'
           OR (NEW.schedule_contract='primary_dynamic_v2' AND (
                NEW.browser_work_item_id IS NOT NULL
                OR NEW.researcher_work_item_id IS NOT NULL
                OR NEW.pentester_work_item_id IS NOT NULL
                OR NEW.adviser_work_item_id IS NOT NULL
                OR NEW.roster_set_sha256 IS NOT NULL))
           OR (NEW.schedule_contract='fixed_roster_v1' AND (
                NEW.schedule_round<>0
                OR NEW.browser_work_item_id<>uuid_generate_v5(
                    NEW.asset_lane_id,'investigation-asset-role-work-item-v1:' ||
                    NEW.evolution_epoch::TEXT || ':browser')
                OR NEW.researcher_work_item_id<>uuid_generate_v5(
                    NEW.asset_lane_id,'investigation-asset-role-work-item-v1:' ||
                    NEW.evolution_epoch::TEXT || ':researcher')
                OR NEW.pentester_work_item_id<>uuid_generate_v5(
                    NEW.asset_lane_id,'investigation-asset-role-work-item-v1:' ||
                    NEW.evolution_epoch::TEXT || ':pentester')
                OR NEW.adviser_work_item_id<>uuid_generate_v5(
                    NEW.asset_lane_id,'investigation-asset-role-work-item-v1:' ||
                    NEW.evolution_epoch::TEXT || ':adviser')
                OR NEW.roster_set_sha256<>investigation_asset_primary_roster_set_sha256()))
        THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.schedule_receipt_id,NEW.stable_request_id,NEW.asset_lane_id,NEW.target_id,
        NEW.asset_context_sha256,NEW.evolution_epoch,NEW.schedule_round,NEW.schedule_contract,
        NEW.stage_team_plan_id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.source_dispatch_epoch,
        NEW.resume_dispatch_epoch,NEW.source_plan_row_version,NEW.primary_work_item_id,
        NEW.primary_worker_run_id,NEW.primary_message_chain_id,NEW.browser_work_item_id,
        NEW.researcher_work_item_id,NEW.pentester_work_item_id,NEW.adviser_work_item_id,
        NEW.roster_set_sha256,NEW.receipt_sha256,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.schedule_receipt_id,OLD.stable_request_id,OLD.asset_lane_id,OLD.target_id,
        OLD.asset_context_sha256,OLD.evolution_epoch,OLD.schedule_round,OLD.schedule_contract,
        OLD.stage_team_plan_id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,OLD.organization_id,OLD.source_dispatch_epoch,
        OLD.resume_dispatch_epoch,OLD.source_plan_row_version,OLD.primary_work_item_id,
        OLD.primary_worker_run_id,OLD.primary_message_chain_id,OLD.browser_work_item_id,
        OLD.researcher_work_item_id,OLD.pentester_work_item_id,OLD.adviser_work_item_id,
        OLD.roster_set_sha256,OLD.receipt_sha256,OLD.created_at
    ) OR OLD.status<>'building' OR NEW.status<>'applied' OR NEW.applied_at IS NULL
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_APPEND_ONLY';
    END IF;

    SELECT EXISTS(
        SELECT 1 FROM stage_work_items item
          JOIN stage_worker_runs worker
            ON worker.id=NEW.primary_worker_run_id AND worker.work_item_id=item.id
          JOIN message_chains chain
            ON chain.id=NEW.primary_message_chain_id
           AND chain.id=worker.message_chain_id AND chain.task_id=NEW.operation_id
         WHERE item.id=NEW.primary_work_item_id
           AND item.team_plan_id=NEW.stage_team_plan_id
           AND item.dispatch_epoch=NEW.resume_dispatch_epoch
           AND item.kind='investigation_asset_primary'
           AND item.stable_key=CASE NEW.schedule_contract
                WHEN 'primary_dynamic_v2' THEN
                    'asset:' || NEW.asset_lane_id::TEXT || ':primary:' ||
                    NEW.evolution_epoch::TEXT || ':round:' || NEW.schedule_round::TEXT
                ELSE 'asset:' || NEW.asset_lane_id::TEXT || ':primary:' ||
                    NEW.evolution_epoch::TEXT END
           AND item.role=plan.leader_role
           AND item.input_manifest_hash=NEW.asset_context_sha256
           AND item.input_refs=CASE NEW.schedule_contract
                WHEN 'primary_dynamic_v2' THEN jsonb_build_array(jsonb_build_object(
                    'kind','investigation_asset_lane','asset_lane_id',NEW.asset_lane_id,
                    'target_id',NEW.target_id,'asset_context_sha256',NEW.asset_context_sha256,
                    'evolution_epoch',NEW.evolution_epoch,'schedule_round',NEW.schedule_round))
                ELSE jsonb_build_array(jsonb_build_object(
                    'kind','investigation_asset_lane','asset_lane_id',NEW.asset_lane_id,
                    'target_id',NEW.target_id,'asset_context_sha256',NEW.asset_context_sha256,
                    'evolution_epoch',NEW.evolution_epoch)) END
           AND item.required_for_barrier=FALSE
           AND item.created_by='server_phase_transition'
           AND item.output_schema='stage_unit_aggregate.v1'
           AND worker.status='queued'
           AND worker.specialist=plan.leader_role
    ) INTO primary_exact;

    IF NEW.schedule_contract='fixed_roster_v1' THEN
        SELECT COUNT(*)=4 AND BOOL_AND(
                   item.kind='investigation_asset_role'
               AND item.required_for_barrier=TRUE
               AND item.created_by='server_phase_transition'
               AND item.role=ANY(ARRAY['browser','researcher','pentester','adviser'])
               AND item.id=CASE item.role
                    WHEN 'browser' THEN NEW.browser_work_item_id
                    WHEN 'researcher' THEN NEW.researcher_work_item_id
                    WHEN 'pentester' THEN NEW.pentester_work_item_id
                    WHEN 'adviser' THEN NEW.adviser_work_item_id END)
          INTO roster_exact FROM stage_work_items item
         WHERE item.team_plan_id=NEW.stage_team_plan_id
           AND item.dispatch_epoch=NEW.resume_dispatch_epoch
           AND item.required_for_barrier=TRUE;
    ELSE
        SELECT COUNT(*)=1 INTO roster_exact FROM stage_work_items item
         WHERE item.team_plan_id=NEW.stage_team_plan_id
           AND item.dispatch_epoch=NEW.resume_dispatch_epoch
           AND item.created_by='server_phase_transition'
           AND item.id=NEW.primary_work_item_id;
    END IF;
    IF plan.dispatch_epoch<>NEW.resume_dispatch_epoch
       OR plan.requests_closed_at IS NOT NULL
       OR NOT primary_exact OR NOT roster_exact
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION enforce_investigation_asset_fixed_roster_work_item()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    schedule investigation_asset_primary_schedules%ROWTYPE;
    expected_role TEXT;
BEGIN
    SELECT * INTO schedule FROM investigation_asset_primary_schedules persisted
     WHERE persisted.stage_team_plan_id=NEW.team_plan_id
       AND persisted.resume_dispatch_epoch=NEW.dispatch_epoch FOR SHARE;
    IF NOT FOUND THEN RETURN NEW; END IF;
    IF schedule.schedule_contract='primary_dynamic_v2' THEN
        IF NEW.id=schedule.primary_work_item_id THEN
            IF schedule.status<>'building'
               OR NEW.kind<>'investigation_asset_primary'
               OR NEW.required_for_barrier
               OR NEW.input_manifest_hash<>schedule.asset_context_sha256
               OR NEW.created_by<>'server_phase_transition'
            THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_WORK_ITEM_MISMATCH'; END IF;
            RETURN NEW;
        END IF;
        IF schedule.status<>'applied' OR NEW.created_by<>'accepted_worker_request' THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_DYNAMIC_CHILD_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;

    IF schedule.status<>'building' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_FIXED_ROSTER_APPEND_ONLY';
    END IF;
    IF NEW.id=schedule.primary_work_item_id THEN RETURN NEW; END IF;
    expected_role := CASE NEW.id
        WHEN schedule.browser_work_item_id THEN 'browser'
        WHEN schedule.researcher_work_item_id THEN 'researcher'
        WHEN schedule.pentester_work_item_id THEN 'pentester'
        WHEN schedule.adviser_work_item_id THEN 'adviser' ELSE NULL END;
    IF expected_role IS NULL OR NEW.kind<>'investigation_asset_role'
       OR NEW.role<>expected_role OR NOT NEW.required_for_barrier
       OR NEW.input_manifest_hash<>schedule.asset_context_sha256
       OR NEW.created_by<>'server_phase_transition'
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_FIXED_ROSTER_MISMATCH'; END IF;
    RETURN NEW;
END;
$$;

-- A Primary is a valid Generator even when it chooses to delegate no child.
-- The exact empty set hash and later seal remain the durable proof; rejecting
-- zero here would recreate a fixed minimum roster at the ledger layer.
DO $$
DECLARE
    legacy_check_name TEXT;
BEGIN
    SELECT constraint_row.conname INTO STRICT legacy_check_name
      FROM pg_constraint constraint_row
     WHERE constraint_row.conrelid='investigation_refiner_plan_ledgers'::REGCLASS
       AND constraint_row.contype='c'
       AND pg_get_constraintdef(constraint_row.oid) LIKE '%generator_subtask_count > 0%';
    EXECUTE format(
        'ALTER TABLE investigation_refiner_plan_ledgers DROP CONSTRAINT %I',
        legacy_check_name
    );
    SELECT constraint_row.conname INTO STRICT legacy_check_name
      FROM pg_constraint constraint_row
     WHERE constraint_row.conrelid='investigation_refiner_plan_ledger_seals'::REGCLASS
       AND constraint_row.contype='c'
       AND pg_get_constraintdef(constraint_row.oid) LIKE '%generator_subtask_count > 0%';
    EXECUTE format(
        'ALTER TABLE investigation_refiner_plan_ledger_seals DROP CONSTRAINT %I',
        legacy_check_name
    );
END;
$$;
ALTER TABLE investigation_refiner_plan_ledgers
    ADD CONSTRAINT refiner_ledger_generator_count_nonnegative
        CHECK(generator_subtask_count>=0);
ALTER TABLE investigation_refiner_plan_ledger_seals
    ADD CONSTRAINT refiner_seal_generator_count_nonnegative
        CHECK(generator_subtask_count>=0);
ALTER TABLE investigation_refiner_plan_ledgers
    ADD COLUMN ledger_contract TEXT NOT NULL DEFAULT 'fixed_denominator_v1'
        CHECK(ledger_contract IN('fixed_denominator_v1','dynamic_ordered_v2'));
ALTER TABLE investigation_refiner_plan_patches
    ADD COLUMN patch_contract TEXT NOT NULL DEFAULT 'fixed_denominator_v1'
        CHECK(patch_contract IN('fixed_denominator_v1','dynamic_ordered_v2'));
ALTER TABLE investigation_refiner_plan_ledger_seals
    ADD COLUMN seal_contract TEXT NOT NULL DEFAULT 'fixed_denominator_v1'
        CHECK(seal_contract IN('fixed_denominator_v1','dynamic_ordered_v2'));

DROP TRIGGER investigation_refiner_plan_ledger_seals_contract
    ON investigation_refiner_plan_ledger_seals;
CREATE TRIGGER investigation_refiner_plan_ledger_seals_v1_contract
BEFORE INSERT ON investigation_refiner_plan_ledger_seals
FOR EACH ROW WHEN (NEW.seal_contract='fixed_denominator_v1')
EXECUTE FUNCTION investigation_guard_refiner_plan_seal_v1();

-- V2 Refiner patches freeze an ordered, mutable active denominator. V1 rows
-- remain readable and their original functions remain unchanged.
CREATE FUNCTION create_investigation_refiner_plan_ledger_v2(
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
    SELECT * INTO STRICT dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_asset_primary_schedules schedule
         WHERE schedule.stage_team_plan_id=plan.stage_team_plan_id
           AND schedule.operation_id=plan.operation_id
           AND schedule.stage_execution_id=plan.stage_execution_id
           AND schedule.stage_run_unit_id=plan.stage_run_unit_id
           AND schedule.organization_id=plan.organization_id
           AND schedule.primary_worker_run_id=dispatch.worker_run_id
           AND schedule.schedule_contract='primary_dynamic_v2' AND schedule.status='applied')
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_ASSET_AUTHORITY_MISMATCH' USING ERRCODE='23514'; END IF;
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

CREATE FUNCTION append_investigation_refiner_plan_patch_v2(
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
    patch_ordinal BIGINT;
    previous_hash TEXT;
    payload_hash TEXT;
    active_count BIGINT;
    active_hash TEXT;
    patch_hash TEXT;
    next_event_ordinal BIGINT;
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
    SELECT * INTO STRICT primary_dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_pentagi_task_plans plan
          JOIN investigation_asset_primary_schedules schedule
            ON schedule.stage_team_plan_id=plan.stage_team_plan_id
           AND schedule.operation_id=plan.operation_id
           AND schedule.stage_execution_id=plan.stage_execution_id
           AND schedule.stage_run_unit_id=plan.stage_run_unit_id
           AND schedule.organization_id=plan.organization_id
           AND schedule.primary_worker_run_id=primary_dispatch.worker_run_id
         WHERE plan.task_plan_id=p_task_plan_id
           AND schedule.schedule_contract='primary_dynamic_v2' AND schedule.status='applied')
       OR EXISTS(SELECT 1 FROM unnest(p_ordered_active_subtask_ids) requested
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

CREATE FUNCTION seal_investigation_refiner_plan_ledger_v2(
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
    SELECT * INTO STRICT dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
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

CREATE FUNCTION investigation_guard_refiner_plan_seal_v2()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    final_patch investigation_refiner_plan_patches%ROWTYPE;
    barrier_event investigation_pentagi_pipeline_events%ROWTYPE;
    actual_patch_count BIGINT; actual_patch_hash TEXT; expected_seal_hash TEXT;
BEGIN
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE ledger_id=NEW.ledger_id AND task_plan_id=NEW.task_plan_id FOR SHARE;
    SELECT * INTO STRICT final_patch FROM investigation_refiner_plan_patches
     WHERE ledger_id=NEW.ledger_id ORDER BY patch_ordinal DESC LIMIT 1;
    SELECT * INTO STRICT barrier_event FROM investigation_pentagi_pipeline_events
     WHERE pipeline_event_id=NEW.result_barrier_pipeline_event_id FOR SHARE;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
        'investigation_refiner_plan_patches.v2',COALESCE(array_agg(
            patch_id::TEXT || ':' || patch_sha256 ORDER BY patch_ordinal),ARRAY[]::TEXT[]))
      INTO actual_patch_count,actual_patch_hash FROM investigation_refiner_plan_patches
     WHERE ledger_id=NEW.ledger_id;
    expected_seal_hash:='sha256:' || encode(digest(convert_to(concat_ws(':',
        'golish.investigation_refiner_plan_seal.v2',NEW.seal_id::TEXT,NEW.ledger_id::TEXT,
        actual_patch_count::TEXT,actual_patch_hash,final_patch.patch_id::TEXT,
        final_patch.patch_sha256,final_patch.active_realized_subtask_count::TEXT,
        final_patch.active_realized_subtask_set_sha256),'UTF8'),'sha256'),'hex');
    IF ledger.ledger_contract<>'dynamic_ordered_v2'
       OR final_patch.patch_contract<>'dynamic_ordered_v2'
       OR NEW.patch_count<>actual_patch_count OR NEW.patch_set_sha256<>actual_patch_hash
       OR NEW.final_patch_id<>final_patch.patch_id
       OR NEW.final_patch_sha256<>final_patch.patch_sha256
       OR NEW.final_active_realized_subtask_count<>final_patch.active_realized_subtask_count
       OR NEW.final_active_realized_subtask_set_sha256<>
          final_patch.active_realized_subtask_set_sha256
       OR NEW.generator_subtask_count<>ledger.generator_subtask_count
       OR NEW.generator_subtask_set_sha256<>ledger.generator_subtask_set_sha256
       OR NEW.seal_sha256<>expected_seal_hash
       OR barrier_event.task_plan_id<>NEW.task_plan_id
       OR barrier_event.event_kind<>'result_barrier'
       OR barrier_event.event_sha256<>expected_seal_hash
       OR EXISTS(SELECT 1 FROM investigation_refiner_plan_patches patch
                   WHERE patch.ledger_id=NEW.ledger_id
                     AND patch.expected_previous_state_sha256<>
                         CASE WHEN patch.patch_ordinal=0 THEN ledger.ledger_sha256 ELSE
                           (SELECT prior.patch_sha256 FROM investigation_refiner_plan_patches prior
                             WHERE prior.ledger_id=NEW.ledger_id
                               AND prior.patch_ordinal=patch.patch_ordinal-1) END)
    THEN RAISE EXCEPTION 'INVESTIGATION_REFINER_V2_SEAL_AUTHORITY_INVALID' USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_refiner_plan_ledger_seals_v2_contract
BEFORE INSERT ON investigation_refiner_plan_ledger_seals
FOR EACH ROW WHEN (NEW.seal_contract='dynamic_ordered_v2')
EXECUTE FUNCTION investigation_guard_refiner_plan_seal_v2();
