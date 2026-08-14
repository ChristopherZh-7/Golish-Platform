-- Append-only cutover from the abandoned fixed-roster Analysis authority to
-- one dynamic Asset-Primary Analysis work.  The historical PentAGI plan and
-- its four generated subtasks remain immutable audit material.  An applied
-- receipt makes that plan non-runnable; it never pretends that the old plan
-- sealed or that any old subtask executed.

CREATE TABLE investigation_dynamic_analysis_work_cutovers (
    cutover_authority_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    legacy_work_id UUID NOT NULL UNIQUE,
    legacy_stable_work_key_sha256 TEXT NOT NULL
        CHECK(legacy_stable_work_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_external_identity_sha256 TEXT NOT NULL
        CHECK(legacy_external_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_task_plan_id UUID NOT NULL UNIQUE,
    legacy_task_plan_sha256 TEXT NOT NULL
        CHECK(legacy_task_plan_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_primary_dispatch_receipt_id UUID NOT NULL UNIQUE,
    legacy_primary_dispatch_sha256 TEXT NOT NULL
        CHECK(legacy_primary_dispatch_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_generator_pipeline_event_id UUID NOT NULL UNIQUE,
    legacy_generator_pipeline_sha256 TEXT NOT NULL
        CHECK(legacy_generator_pipeline_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_refiner_ledger_id UUID NOT NULL UNIQUE,
    legacy_refiner_ledger_sha256 TEXT NOT NULL
        CHECK(legacy_refiner_ledger_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    legacy_subtask_count BIGINT NOT NULL CHECK(legacy_subtask_count=4),
    legacy_subtask_set_sha256 TEXT NOT NULL
        CHECK(legacy_subtask_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dynamic_work_id UUID NOT NULL UNIQUE,
    dynamic_stable_work_key_sha256 TEXT NOT NULL
        CHECK(dynamic_stable_work_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dynamic_external_identity_sha256 TEXT NOT NULL
        CHECK(dynamic_external_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    observed_stop_epoch BIGINT NOT NULL CHECK(observed_stop_epoch>=0),
    legacy_work_superseded_event_id UUID NOT NULL UNIQUE,
    receipt_sha256 TEXT NOT NULL UNIQUE
        CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK(status IN('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    CHECK(dynamic_work_id<>legacy_work_id),
    CHECK(dynamic_stable_work_key_sha256<>legacy_stable_work_key_sha256),
    CHECK((status='building' AND applied_at IS NULL)
       OR (status='applied' AND applied_at IS NOT NULL)),
    FOREIGN KEY(legacy_work_id) REFERENCES investigation_run_work_items(work_id) ON DELETE RESTRICT,
    FOREIGN KEY(dynamic_work_id) REFERENCES investigation_run_work_items(work_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(legacy_task_plan_id) REFERENCES investigation_pentagi_task_plans(task_plan_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(legacy_primary_dispatch_receipt_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(legacy_generator_pipeline_event_id)
        REFERENCES investigation_pentagi_pipeline_events(pipeline_event_id) ON DELETE RESTRICT,
    FOREIGN KEY(legacy_refiner_ledger_id)
        REFERENCES investigation_refiner_plan_ledgers(ledger_id) ON DELETE RESTRICT,
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    FOREIGN KEY(authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,
                scope_snapshot_id)
        REFERENCES investigation_run_heads(authority_id,operation_id,stage_execution_id,
                                            owning_stage_run_request_id,scope_snapshot_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION investigation_dynamic_analysis_work_cutover_sha256(
    p_cutover_authority_id UUID,p_stable_request_id UUID,p_authority_id UUID,
    p_operation_id UUID,p_stage_execution_id UUID,p_owning_stage_run_request_id TEXT,
    p_stage_run_unit_id UUID,p_scope_snapshot_id UUID,p_organization_id UUID,
    p_asset_lane_id UUID,p_legacy_work_id UUID,p_legacy_stable_work_key_sha256 TEXT,
    p_legacy_external_identity_sha256 TEXT,p_legacy_task_plan_id UUID,
    p_legacy_task_plan_sha256 TEXT,p_legacy_primary_dispatch_receipt_id UUID,
    p_legacy_primary_dispatch_sha256 TEXT,p_legacy_generator_pipeline_event_id UUID,
    p_legacy_generator_pipeline_sha256 TEXT,p_legacy_refiner_ledger_id UUID,
    p_legacy_refiner_ledger_sha256 TEXT,p_legacy_subtask_count BIGINT,
    p_legacy_subtask_set_sha256 TEXT,p_dynamic_work_id UUID,
    p_dynamic_stable_work_key_sha256 TEXT,p_dynamic_external_identity_sha256 TEXT,
    p_observed_stop_epoch BIGINT,p_legacy_work_superseded_event_id UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_dynamic_analysis_work_cutover.v2',
        'cutover_authority_id',p_cutover_authority_id,'stable_request_id',p_stable_request_id,
        'authority_id',p_authority_id,'operation_id',p_operation_id,
        'stage_execution_id',p_stage_execution_id,
        'owning_stage_run_request_id',p_owning_stage_run_request_id,
        'stage_run_unit_id',p_stage_run_unit_id,'scope_snapshot_id',p_scope_snapshot_id,
        'organization_id',p_organization_id,'asset_lane_id',p_asset_lane_id,
        'legacy_work_id',p_legacy_work_id,
        'legacy_stable_work_key_sha256',p_legacy_stable_work_key_sha256,
        'legacy_external_identity_sha256',p_legacy_external_identity_sha256,
        'legacy_task_plan_id',p_legacy_task_plan_id,
        'legacy_task_plan_sha256',p_legacy_task_plan_sha256,
        'legacy_primary_dispatch_receipt_id',p_legacy_primary_dispatch_receipt_id,
        'legacy_primary_dispatch_sha256',p_legacy_primary_dispatch_sha256,
        'legacy_generator_pipeline_event_id',p_legacy_generator_pipeline_event_id,
        'legacy_generator_pipeline_sha256',p_legacy_generator_pipeline_sha256,
        'legacy_refiner_ledger_id',p_legacy_refiner_ledger_id,
        'legacy_refiner_ledger_sha256',p_legacy_refiner_ledger_sha256,
        'legacy_subtask_count',p_legacy_subtask_count,
        'legacy_subtask_set_sha256',p_legacy_subtask_set_sha256,
        'dynamic_work_id',p_dynamic_work_id,
        'dynamic_stable_work_key_sha256',p_dynamic_stable_work_key_sha256,
        'dynamic_external_identity_sha256',p_dynamic_external_identity_sha256,
        'observed_stop_epoch',p_observed_stop_epoch,
        'legacy_work_superseded_event_id',p_legacy_work_superseded_event_id
    )::TEXT)
$$;

CREATE FUNCTION investigation_dynamic_analysis_cutover_source_is_exact(
    cutover investigation_dynamic_analysis_work_cutovers
) RETURNS BOOLEAN LANGUAGE SQL STABLE STRICT AS $$
    SELECT EXISTS(
        SELECT 1
          FROM investigation_run_work_items work
          JOIN investigation_analysis_attempt_bindings binding
            ON binding.work_id=work.work_id AND binding.authority_id=work.authority_id
          JOIN investigation_pentagi_task_plans plan
            ON plan.authority_id=work.authority_id
           AND plan.operation_id=work.operation_id
           AND plan.stage_execution_id=work.stage_execution_id
           AND plan.stage_run_unit_id=work.stage_run_unit_id
           AND plan.scope_snapshot_id=work.scope_snapshot_id
           AND plan.organization_id=work.organization_id
           AND plan.subject_kind='analysis_attempt'
           AND plan.subject_id=binding.analysis_attempt_id
          JOIN investigation_asset_primary_schedules schedule
            ON schedule.stage_team_plan_id=plan.stage_team_plan_id
           AND schedule.operation_id=plan.operation_id
           AND schedule.stage_execution_id=plan.stage_execution_id
           AND schedule.stage_run_unit_id=plan.stage_run_unit_id
           AND schedule.scope_snapshot_id=plan.scope_snapshot_id
           AND schedule.organization_id=plan.organization_id
           AND schedule.asset_lane_id=work.asset_lane_id
          JOIN pentagi_logical_dispatch_receipts dispatch
            ON dispatch.task_plan_id=plan.task_plan_id
           AND dispatch.actor_kind='primary' AND dispatch.subtask_id IS NULL
          JOIN investigation_pentagi_pipeline_events generator
            ON generator.task_plan_id=plan.task_plan_id
           AND generator.event_kind='generator_sealed' AND generator.event_ordinal=0
           AND generator.subtask_id IS NULL
           AND generator.actor_worker_run_id=dispatch.worker_run_id
           AND generator.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id
          JOIN investigation_refiner_plan_ledgers ledger
            ON ledger.task_plan_id=plan.task_plan_id
           AND ledger.generator_pipeline_event_id=generator.pipeline_event_id
         WHERE work.work_id=cutover.legacy_work_id
           AND ROW(work.authority_id,work.operation_id,work.stage_execution_id,
                   work.owning_stage_run_request_id,work.stage_run_unit_id,
                   work.scope_snapshot_id,work.organization_id,work.asset_lane_id,
                   work.stable_work_key_sha256,work.external_identity_sha256,
                   work.work_kind,work.current_state,work.observed_stop_epoch,
                   work.head_version,work.latest_event_id)
               IS NOT DISTINCT FROM ROW(cutover.authority_id,cutover.operation_id,
                   cutover.stage_execution_id,
                   cutover.owning_stage_run_request_id,cutover.stage_run_unit_id,
                   cutover.scope_snapshot_id,cutover.organization_id,cutover.asset_lane_id,
                   cutover.legacy_stable_work_key_sha256,
                   cutover.legacy_external_identity_sha256,'analysis'::TEXT,'running'::TEXT,
                   cutover.observed_stop_epoch,0::BIGINT,NULL::UUID)
           AND plan.task_plan_id=cutover.legacy_task_plan_id
           AND plan.task_plan_sha256=cutover.legacy_task_plan_sha256
           AND plan.status='open' AND plan.row_version=0
           AND schedule.schedule_contract='fixed_roster_v1' AND schedule.status='applied'
           AND schedule.primary_work_item_id=dispatch.stage_work_item_id
           AND schedule.primary_worker_run_id=dispatch.worker_run_id
           AND dispatch.dispatch_receipt_id=cutover.legacy_primary_dispatch_receipt_id
           AND dispatch.receipt_sha256=cutover.legacy_primary_dispatch_sha256
           AND generator.pipeline_event_id=cutover.legacy_generator_pipeline_event_id
           AND generator.event_sha256=cutover.legacy_generator_pipeline_sha256
           AND ledger.ledger_id=cutover.legacy_refiner_ledger_id
           AND ledger.ledger_sha256=cutover.legacy_refiner_ledger_sha256
           AND ledger.ledger_contract='fixed_denominator_v1'
           AND ledger.generator_subtask_count=4
           AND ledger.generator_subtask_set_sha256=cutover.legacy_subtask_set_sha256
           AND cutover.legacy_subtask_count=4
           AND (SELECT COUNT(*) FROM investigation_pentagi_subtasks subtask
                 WHERE subtask.task_plan_id=plan.task_plan_id AND subtask.runnable)=4
           AND cutover.legacy_subtask_set_sha256=(
                SELECT unified_investigation_exact_set_hash(
                    'investigation_refiner_generator_subtasks.v1',
                    COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                                       ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[]))
                  FROM investigation_pentagi_subtasks subtask
                 WHERE subtask.task_plan_id=plan.task_plan_id)
           AND (SELECT COUNT(*) FROM pentagi_logical_dispatch_receipts all_dispatch
                 WHERE all_dispatch.task_plan_id=plan.task_plan_id)=1
           AND NOT EXISTS(
                SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
                 JOIN pentagi_logical_dispatch_receipts owned_dispatch
                   ON owned_dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
                WHERE owned_dispatch.task_plan_id=plan.task_plan_id)
           AND NOT EXISTS(SELECT 1 FROM investigation_nested_dispatch_begins nested
                           WHERE nested.task_plan_id=plan.task_plan_id)
           AND NOT EXISTS(SELECT 1 FROM investigation_nested_dispatch_finishes nested
                           WHERE nested.task_plan_id=plan.task_plan_id)
           AND NOT EXISTS(SELECT 1 FROM investigation_refiner_plan_patches patch
                           WHERE patch.task_plan_id=plan.task_plan_id)
           AND NOT EXISTS(SELECT 1 FROM investigation_refiner_plan_ledger_seals seal
                           WHERE seal.task_plan_id=plan.task_plan_id)
           AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                 WHERE event.task_plan_id=plan.task_plan_id)=1
           AND NOT EXISTS(
                SELECT 1 FROM tool_calls tool
                 JOIN stage_worker_runs worker ON worker.id=tool.worker_run_id
                 JOIN stage_work_items item ON item.id=worker.work_item_id
                WHERE (worker.id=schedule.primary_worker_run_id
                       OR item.id IN(schedule.browser_work_item_id,
                                     schedule.researcher_work_item_id,
                                     schedule.pentester_work_item_id,
                                     schedule.adviser_work_item_id))
                  AND tool.name NOT IN('submit_result','update_plan'))
    )
$$;

CREATE FUNCTION enforce_investigation_dynamic_analysis_work_cutover()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_receipt_sha256 TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_APPEND_ONLY';
    END IF;
    expected_receipt_sha256:=investigation_dynamic_analysis_work_cutover_sha256(
        NEW.cutover_authority_id,NEW.stable_request_id,NEW.authority_id,NEW.operation_id,
        NEW.stage_execution_id,NEW.owning_stage_run_request_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.asset_lane_id,NEW.legacy_work_id,
        NEW.legacy_stable_work_key_sha256,NEW.legacy_external_identity_sha256,
        NEW.legacy_task_plan_id,NEW.legacy_task_plan_sha256,
        NEW.legacy_primary_dispatch_receipt_id,NEW.legacy_primary_dispatch_sha256,
        NEW.legacy_generator_pipeline_event_id,NEW.legacy_generator_pipeline_sha256,
        NEW.legacy_refiner_ledger_id,NEW.legacy_refiner_ledger_sha256,
        NEW.legacy_subtask_count,NEW.legacy_subtask_set_sha256,NEW.dynamic_work_id,
        NEW.dynamic_stable_work_key_sha256,NEW.dynamic_external_identity_sha256,
        NEW.observed_stop_epoch,NEW.legacy_work_superseded_event_id);
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
           OR NEW.cutover_authority_id<>uuid_generate_v5(
                NEW.legacy_work_id,'investigation-dynamic-asset-analysis-work-cutover-v2')
           OR NEW.legacy_work_superseded_event_id<>uuid_generate_v5(
                NEW.cutover_authority_id,'legacy-work-superseded-event-v2')
           OR NEW.receipt_sha256<>expected_receipt_sha256
           OR NOT investigation_dynamic_analysis_cutover_source_is_exact(NEW)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_SOURCE_MISMATCH'
            USING ERRCODE='23514'; END IF;
        RETURN NEW;
    END IF;
    IF ROW(NEW.cutover_authority_id,NEW.stable_request_id,NEW.authority_id,NEW.operation_id,
           NEW.stage_execution_id,NEW.owning_stage_run_request_id,NEW.stage_run_unit_id,
           NEW.scope_snapshot_id,NEW.organization_id,NEW.asset_lane_id,NEW.legacy_work_id,
           NEW.legacy_stable_work_key_sha256,NEW.legacy_external_identity_sha256,
           NEW.legacy_task_plan_id,NEW.legacy_task_plan_sha256,
           NEW.legacy_primary_dispatch_receipt_id,NEW.legacy_primary_dispatch_sha256,
           NEW.legacy_generator_pipeline_event_id,NEW.legacy_generator_pipeline_sha256,
           NEW.legacy_refiner_ledger_id,NEW.legacy_refiner_ledger_sha256,
           NEW.legacy_subtask_count,NEW.legacy_subtask_set_sha256,NEW.dynamic_work_id,
           NEW.dynamic_stable_work_key_sha256,NEW.dynamic_external_identity_sha256,
           NEW.observed_stop_epoch,NEW.legacy_work_superseded_event_id,NEW.receipt_sha256,
           NEW.created_at)
       IS DISTINCT FROM ROW(OLD.cutover_authority_id,OLD.stable_request_id,OLD.authority_id,
           OLD.operation_id,OLD.stage_execution_id,OLD.owning_stage_run_request_id,
           OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,OLD.asset_lane_id,
           OLD.legacy_work_id,OLD.legacy_stable_work_key_sha256,
           OLD.legacy_external_identity_sha256,OLD.legacy_task_plan_id,
           OLD.legacy_task_plan_sha256,OLD.legacy_primary_dispatch_receipt_id,
           OLD.legacy_primary_dispatch_sha256,OLD.legacy_generator_pipeline_event_id,
           OLD.legacy_generator_pipeline_sha256,OLD.legacy_refiner_ledger_id,
           OLD.legacy_refiner_ledger_sha256,OLD.legacy_subtask_count,
           OLD.legacy_subtask_set_sha256,OLD.dynamic_work_id,
           OLD.dynamic_stable_work_key_sha256,OLD.dynamic_external_identity_sha256,
           OLD.observed_stop_epoch,OLD.legacy_work_superseded_event_id,OLD.receipt_sha256,
           OLD.created_at)
       OR OLD.status<>'building' OR NEW.status<>'applied'
       OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
       OR NOT EXISTS(
            SELECT 1 FROM investigation_run_work_items legacy
             JOIN investigation_run_work_state_events event
               ON event.event_id=NEW.legacy_work_superseded_event_id
              AND event.work_id=legacy.work_id AND event.event_ordinal=legacy.head_version
             JOIN investigation_run_work_items dynamic
               ON dynamic.work_id=NEW.dynamic_work_id
            WHERE legacy.work_id=NEW.legacy_work_id
              AND legacy.current_state='superseded' AND legacy.head_version=1
              AND event.from_state='running' AND event.to_state='superseded'
              AND event.reason_code='dynamic_asset_analysis_work_cutover.v2|' ||
                  NEW.cutover_authority_id::TEXT
              AND event.event_sha256=tool_truth_sha256(jsonb_build_object(
                  'domain','dynamic_asset_analysis_work_cutover_event.v2',
                  'cutover_authority_id',NEW.cutover_authority_id,
                  'event_id',NEW.legacy_work_superseded_event_id,
                  'legacy_work_id',NEW.legacy_work_id,
                  'dynamic_work_id',NEW.dynamic_work_id)::TEXT)
              AND ROW(dynamic.authority_id,dynamic.operation_id,dynamic.stage_execution_id,
                      dynamic.owning_stage_run_request_id,dynamic.stage_run_unit_id,
                      dynamic.scope_snapshot_id,dynamic.organization_id,dynamic.asset_lane_id,
                      dynamic.stable_work_key_sha256,dynamic.external_identity_sha256,
                      dynamic.work_kind,dynamic.current_state,dynamic.observed_stop_epoch,
                      dynamic.head_version,dynamic.latest_event_id)
                  IS NOT DISTINCT FROM ROW(NEW.authority_id,NEW.operation_id,
                      NEW.stage_execution_id,
                      NEW.owning_stage_run_request_id,NEW.stage_run_unit_id,
                      NEW.scope_snapshot_id,NEW.organization_id,NEW.asset_lane_id,
                      NEW.dynamic_stable_work_key_sha256,NEW.dynamic_external_identity_sha256,
                      'analysis'::TEXT,'running'::TEXT,NEW.observed_stop_epoch,0::BIGINT,NULL::UUID))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_APPLY_MISMATCH'
        USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_dynamic_analysis_work_cutovers_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_analysis_work_cutovers
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_dynamic_analysis_work_cutover();

CREATE FUNCTION investigation_dynamic_analysis_cutover_work_transition_allowed(
    work investigation_run_work_items,rearm investigation_run_work_state_events
) RETURNS BOOLEAN LANGUAGE SQL STABLE STRICT AS $$
    SELECT work.work_kind='analysis' AND work.current_state='running'
       AND work.head_version=0 AND work.latest_event_id IS NULL
       AND rearm.from_state='running' AND rearm.to_state='superseded'
       AND rearm.event_ordinal=1 AND rearm.expected_head_version=0
       AND split_part(rearm.reason_code,'|',1)='dynamic_asset_analysis_work_cutover.v2'
       AND EXISTS(
           SELECT 1 FROM investigation_dynamic_analysis_work_cutovers cutover
            WHERE cutover.cutover_authority_id::TEXT=split_part(rearm.reason_code,'|',2)
              AND cutover.status='building' AND cutover.legacy_work_id=work.work_id
              AND cutover.legacy_work_superseded_event_id=rearm.event_id)
$$;

CREATE OR REPLACE FUNCTION unified_investigation_apply_work_event()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE work investigation_run_work_items%ROWTYPE; run investigation_run_heads%ROWTYPE;
    transition_allowed BOOLEAN;
BEGIN
    SELECT * INTO STRICT work FROM investigation_run_work_items
     WHERE work_id=NEW.work_id FOR UPDATE;
    SELECT * INTO STRICT run FROM investigation_run_heads
     WHERE authority_id=work.authority_id FOR SHARE;
    transition_allowed:=unified_investigation_work_transition_allowed(work.current_state,NEW.to_state)
        OR unified_investigation_post_synthesis_analysis_rearm_allowed(work,NEW)
        OR unified_investigation_primary_post_synthesis_analysis_rearm_allowed(work,NEW)
        OR investigation_dynamic_analysis_cutover_work_transition_allowed(work,NEW);
    IF NEW.expected_head_version<>work.head_version
       OR NEW.event_ordinal<>work.head_version+1 OR NEW.from_state<>work.current_state
       OR NOT transition_allowed OR NEW.observed_stop_epoch<>run.stop_epoch
    THEN RAISE EXCEPTION 'INVESTIGATION_WORK_STATE_CAS_INVALID' USING ERRCODE='23514'; END IF;
    IF NOT run.admission_open AND NEW.to_state IN('queued','running','waiting_authorization')
    THEN RAISE EXCEPTION 'INVESTIGATION_WORK_REACTIVATION_AFTER_STOP' USING ERRCODE='23514'; END IF;
    PERFORM set_config('golish.investigation_work_event_apply','on',TRUE);
    UPDATE investigation_run_work_items SET current_state=NEW.to_state,
        observed_stop_epoch=NEW.observed_stop_epoch,head_version=NEW.event_ordinal,
        latest_event_id=NEW.event_id,updated_at=statement_timestamp()
     WHERE work_id=NEW.work_id;
    PERFORM set_config('golish.investigation_work_event_apply','off',TRUE);
    RETURN NEW;
END;
$$;

CREATE FUNCTION investigation_assert_pentagi_plan_not_cut_over(p_task_plan_id UUID)
RETURNS VOID LANGUAGE plpgsql STABLE STRICT AS $$
BEGIN
    IF EXISTS(SELECT 1 FROM investigation_dynamic_analysis_work_cutovers cutover
               WHERE cutover.legacy_task_plan_id=p_task_plan_id AND cutover.status='applied')
    THEN RAISE EXCEPTION 'INVESTIGATION_FIXED_ANALYSIS_PLAN_SUPERSEDED'
        USING ERRCODE='23514'; END IF;
END;
$$;

CREATE FUNCTION investigation_guard_cut_over_pentagi_writer()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_task_plan_id UUID;
BEGIN
    IF TG_TABLE_NAME='pentagi_logical_dispatch_attempts' THEN
        SELECT dispatch.task_plan_id INTO STRICT requested_task_plan_id
          FROM pentagi_logical_dispatch_receipts dispatch
         WHERE dispatch.dispatch_receipt_id=NEW.dispatch_receipt_id;
    ELSIF TG_TABLE_NAME='investigation_refiner_plan_patch_members' THEN
        SELECT patch.task_plan_id INTO STRICT requested_task_plan_id
          FROM investigation_refiner_plan_patches patch
         WHERE patch.patch_id=NEW.patch_id;
    ELSE
        requested_task_plan_id:=NEW.task_plan_id;
    END IF;
    PERFORM investigation_assert_pentagi_plan_not_cut_over(requested_task_plan_id);
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_cutover_task_plan_write_fence
BEFORE UPDATE ON investigation_pentagi_task_plans
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_subtask_write_fence
BEFORE INSERT ON investigation_pentagi_subtasks
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_dispatch_write_fence
BEFORE INSERT ON pentagi_logical_dispatch_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_dispatch_attempt_write_fence
BEFORE INSERT ON pentagi_logical_dispatch_attempts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_task_run_request_write_fence
BEFORE INSERT ON pentagi_task_run_requests
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_pipeline_write_fence
BEFORE INSERT ON investigation_pentagi_pipeline_events
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_census_write_fence
BEFORE INSERT ON investigation_pentagi_delegation_census_seals
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_refiner_ledger_write_fence
BEFORE INSERT ON investigation_refiner_plan_ledgers
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_refiner_patch_write_fence
BEFORE INSERT ON investigation_refiner_plan_patches
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_refiner_patch_member_write_fence
BEFORE INSERT ON investigation_refiner_plan_patch_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_refiner_seal_write_fence
BEFORE INSERT ON investigation_refiner_plan_ledger_seals
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_nested_begin_write_fence
BEFORE INSERT ON investigation_nested_dispatch_begins
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();
CREATE TRIGGER investigation_cutover_nested_finish_write_fence
BEFORE INSERT ON investigation_nested_dispatch_finishes
FOR EACH ROW EXECUTE FUNCTION investigation_guard_cut_over_pentagi_writer();

CREATE FUNCTION ensure_investigation_dynamic_asset_analysis_work_v2(
    p_stable_request_id UUID,p_authority_id UUID,p_operation_id UUID,
    p_stage_execution_id UUID,p_owning_stage_run_request_id TEXT,p_stage_run_unit_id UUID,
    p_scope_snapshot_id UUID,p_organization_id UUID,p_asset_lane_id UUID,
    p_legacy_stable_work_key_sha256 TEXT,p_dynamic_work_id UUID,
    p_dynamic_stable_work_key_sha256 TEXT,p_dynamic_external_identity_sha256 TEXT,
    p_observed_stop_epoch BIGINT
) RETURNS UUID LANGUAGE plpgsql AS $$
DECLARE existing_cutover investigation_dynamic_analysis_work_cutovers%ROWTYPE;
    existing_dynamic investigation_run_work_items%ROWTYPE;
    legacy investigation_run_work_items%ROWTYPE;
    legacy_plan investigation_pentagi_task_plans%ROWTYPE;
    dispatch pentagi_logical_dispatch_receipts%ROWTYPE;
    generator investigation_pentagi_pipeline_events%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    cutover_id UUID; event_id UUID; subtask_count BIGINT; subtask_hash TEXT; receipt_hash TEXT;
BEGIN
    IF p_stable_request_id IS NULL OR p_dynamic_work_id IS NULL OR p_asset_lane_id IS NULL
       OR p_legacy_stable_work_key_sha256 !~ '^sha256:[0-9a-f]{64}$'
       OR p_dynamic_stable_work_key_sha256 !~ '^sha256:[0-9a-f]{64}$'
       OR p_dynamic_external_identity_sha256 !~ '^sha256:[0-9a-f]{64}$'
       OR p_legacy_stable_work_key_sha256=p_dynamic_stable_work_key_sha256
       OR p_observed_stop_epoch<0
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_INPUT_INVALID'
        USING ERRCODE='23514'; END IF;
    SELECT * INTO existing_cutover FROM investigation_dynamic_analysis_work_cutovers
     WHERE stable_request_id=p_stable_request_id FOR SHARE;
    IF FOUND THEN
        IF existing_cutover.status<>'applied'
           OR ROW(existing_cutover.authority_id,existing_cutover.operation_id,
                  existing_cutover.stage_execution_id,existing_cutover.owning_stage_run_request_id,
                  existing_cutover.stage_run_unit_id,existing_cutover.scope_snapshot_id,
                  existing_cutover.organization_id,existing_cutover.asset_lane_id,
                  existing_cutover.legacy_stable_work_key_sha256,
                  existing_cutover.dynamic_work_id,existing_cutover.dynamic_stable_work_key_sha256,
                  existing_cutover.dynamic_external_identity_sha256,
                  existing_cutover.observed_stop_epoch)
              IS DISTINCT FROM ROW(p_authority_id,p_operation_id,p_stage_execution_id,
                  p_owning_stage_run_request_id,p_stage_run_unit_id,p_scope_snapshot_id,
                  p_organization_id,p_asset_lane_id,p_legacy_stable_work_key_sha256,
                  p_dynamic_work_id,p_dynamic_stable_work_key_sha256,
                  p_dynamic_external_identity_sha256,p_observed_stop_epoch)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_REPLAY_MISMATCH'
            USING ERRCODE='23514'; END IF;
        RETURN existing_cutover.dynamic_work_id;
    END IF;
    SELECT * INTO existing_dynamic FROM investigation_run_work_items work
     WHERE work.authority_id=p_authority_id
       AND work.stable_work_key_sha256=p_dynamic_stable_work_key_sha256 FOR UPDATE;
    IF FOUND THEN
        IF ROW(existing_dynamic.work_id,existing_dynamic.asset_lane_id,
               existing_dynamic.operation_id,existing_dynamic.stage_execution_id,
               existing_dynamic.owning_stage_run_request_id,existing_dynamic.stage_run_unit_id,
               existing_dynamic.scope_snapshot_id,existing_dynamic.organization_id,
               existing_dynamic.work_kind,existing_dynamic.external_identity_sha256,
               existing_dynamic.current_state,existing_dynamic.observed_stop_epoch)
           IS DISTINCT FROM ROW(p_dynamic_work_id,p_asset_lane_id,p_operation_id,
               p_stage_execution_id,p_owning_stage_run_request_id,p_stage_run_unit_id,
               p_scope_snapshot_id,p_organization_id,'analysis'::TEXT,
               p_dynamic_external_identity_sha256,'running'::TEXT,p_observed_stop_epoch)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_WORK_REPLAY_MISMATCH'
            USING ERRCODE='23514'; END IF;
        SELECT * INTO existing_cutover
          FROM investigation_dynamic_analysis_work_cutovers cutover
         WHERE cutover.dynamic_work_id=existing_dynamic.work_id FOR SHARE;
        IF FOUND AND (existing_cutover.status<>'applied'
           OR existing_cutover.stable_request_id<>p_stable_request_id
           OR existing_cutover.legacy_stable_work_key_sha256<>
                p_legacy_stable_work_key_sha256)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_ANALYSIS_CUTOVER_REPLAY_MISMATCH'
            USING ERRCODE='23514'; END IF;
        RETURN existing_dynamic.work_id;
    END IF;
    SELECT * INTO legacy FROM investigation_run_work_items work
     WHERE work.authority_id=p_authority_id
       AND work.stable_work_key_sha256=p_legacy_stable_work_key_sha256 FOR UPDATE;
    IF NOT FOUND THEN
        INSERT INTO investigation_run_work_items(work_id,asset_lane_id,stable_work_key_sha256,
            authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,
            stage_run_unit_id,scope_snapshot_id,organization_id,work_kind,
            external_identity_sha256,current_state,observed_stop_epoch)
        VALUES(p_dynamic_work_id,p_asset_lane_id,p_dynamic_stable_work_key_sha256,p_authority_id,
            p_operation_id,p_stage_execution_id,p_owning_stage_run_request_id,p_stage_run_unit_id,
            p_scope_snapshot_id,p_organization_id,'analysis',p_dynamic_external_identity_sha256,
            'running',p_observed_stop_epoch);
        RETURN p_dynamic_work_id;
    END IF;
    SELECT persisted_plan.* INTO STRICT legacy_plan
      FROM investigation_analysis_attempt_bindings binding
      JOIN investigation_pentagi_task_plans persisted_plan
        ON persisted_plan.authority_id=binding.authority_id
       AND persisted_plan.operation_id=binding.operation_id
       AND persisted_plan.stage_execution_id=binding.stage_execution_id
       AND persisted_plan.stage_run_unit_id=binding.stage_run_unit_id
       AND persisted_plan.organization_id=binding.organization_id
       AND persisted_plan.subject_kind='analysis_attempt'
       AND persisted_plan.subject_id=binding.analysis_attempt_id
     WHERE binding.work_id=legacy.work_id FOR UPDATE OF persisted_plan;
    SELECT * INTO STRICT dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=legacy_plan.task_plan_id AND actor_kind='primary' AND subtask_id IS NULL FOR SHARE;
    SELECT * INTO STRICT generator FROM investigation_pentagi_pipeline_events
     WHERE task_plan_id=legacy_plan.task_plan_id AND event_kind='generator_sealed' FOR SHARE;
    SELECT * INTO STRICT ledger FROM investigation_refiner_plan_ledgers
     WHERE task_plan_id=legacy_plan.task_plan_id FOR SHARE;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
        'investigation_refiner_generator_subtasks.v1',
        COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                           ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[]))
      INTO subtask_count,subtask_hash FROM investigation_pentagi_subtasks subtask
     WHERE subtask.task_plan_id=legacy_plan.task_plan_id AND subtask.runnable;
    cutover_id:=uuid_generate_v5(legacy.work_id,
        'investigation-dynamic-asset-analysis-work-cutover-v2');
    event_id:=uuid_generate_v5(cutover_id,'legacy-work-superseded-event-v2');
    receipt_hash:=investigation_dynamic_analysis_work_cutover_sha256(cutover_id,
        p_stable_request_id,p_authority_id,p_operation_id,p_stage_execution_id,
        p_owning_stage_run_request_id,p_stage_run_unit_id,p_scope_snapshot_id,p_organization_id,
        p_asset_lane_id,legacy.work_id,legacy.stable_work_key_sha256,
        legacy.external_identity_sha256,legacy_plan.task_plan_id,legacy_plan.task_plan_sha256,
        dispatch.dispatch_receipt_id,dispatch.receipt_sha256,generator.pipeline_event_id,
        generator.event_sha256,ledger.ledger_id,ledger.ledger_sha256,subtask_count,subtask_hash,
        p_dynamic_work_id,p_dynamic_stable_work_key_sha256,p_dynamic_external_identity_sha256,
        p_observed_stop_epoch,event_id);
    INSERT INTO investigation_dynamic_analysis_work_cutovers(
        cutover_authority_id,stable_request_id,authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
        asset_lane_id,legacy_work_id,legacy_stable_work_key_sha256,
        legacy_external_identity_sha256,legacy_task_plan_id,legacy_task_plan_sha256,
        legacy_primary_dispatch_receipt_id,legacy_primary_dispatch_sha256,
        legacy_generator_pipeline_event_id,legacy_generator_pipeline_sha256,
        legacy_refiner_ledger_id,legacy_refiner_ledger_sha256,legacy_subtask_count,
        legacy_subtask_set_sha256,dynamic_work_id,dynamic_stable_work_key_sha256,
        dynamic_external_identity_sha256,observed_stop_epoch,
        legacy_work_superseded_event_id,receipt_sha256,status)
    VALUES(cutover_id,p_stable_request_id,p_authority_id,p_operation_id,p_stage_execution_id,
        p_owning_stage_run_request_id,p_stage_run_unit_id,p_scope_snapshot_id,p_organization_id,
        p_asset_lane_id,legacy.work_id,legacy.stable_work_key_sha256,
        legacy.external_identity_sha256,legacy_plan.task_plan_id,legacy_plan.task_plan_sha256,
        dispatch.dispatch_receipt_id,dispatch.receipt_sha256,generator.pipeline_event_id,
        generator.event_sha256,ledger.ledger_id,ledger.ledger_sha256,subtask_count,subtask_hash,
        p_dynamic_work_id,p_dynamic_stable_work_key_sha256,p_dynamic_external_identity_sha256,
        p_observed_stop_epoch,event_id,receipt_hash,'building');
    INSERT INTO investigation_run_work_items(work_id,asset_lane_id,stable_work_key_sha256,
        authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,
        stage_run_unit_id,scope_snapshot_id,organization_id,work_kind,
        external_identity_sha256,current_state,observed_stop_epoch)
    VALUES(p_dynamic_work_id,p_asset_lane_id,p_dynamic_stable_work_key_sha256,p_authority_id,
        p_operation_id,p_stage_execution_id,p_owning_stage_run_request_id,p_stage_run_unit_id,
        p_scope_snapshot_id,p_organization_id,'analysis',p_dynamic_external_identity_sha256,
        'running',p_observed_stop_epoch);
    INSERT INTO investigation_run_work_state_events(event_id,stable_request_id,work_id,
        expected_head_version,event_ordinal,from_state,to_state,observed_stop_epoch,
        reason_code,event_sha256)
    VALUES(event_id,uuid_generate_v5(cutover_id,'legacy-work-superseded-request-v2'),
        legacy.work_id,0,1,'running','superseded',p_observed_stop_epoch,
        'dynamic_asset_analysis_work_cutover.v2|' || cutover_id::TEXT,
        tool_truth_sha256(jsonb_build_object('domain','dynamic_asset_analysis_work_cutover_event.v2',
            'cutover_authority_id',cutover_id,'event_id',event_id,
            'legacy_work_id',legacy.work_id,'dynamic_work_id',p_dynamic_work_id)::TEXT));
    UPDATE investigation_dynamic_analysis_work_cutovers
       SET status='applied',applied_at=statement_timestamp()
     WHERE cutover_authority_id=cutover_id AND status='building';
    RETURN p_dynamic_work_id;
END;
$$;
