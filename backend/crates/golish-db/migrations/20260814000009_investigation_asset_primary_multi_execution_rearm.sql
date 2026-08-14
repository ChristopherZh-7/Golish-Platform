-- Extend Asset Primary execution recovery from one successor to a bounded,
-- append-only chain.  Every continuation consumes the exact terminal
-- exhaustion output of the immediately preceding shell, advances the shared
-- TeamPlan by one dispatch epoch, and reuses the asset's durable message chain.

ALTER TABLE investigation_asset_primary_rearms
    ADD COLUMN predecessor_rearm_receipt_id UUID,
    -- A constant default backfills retained generation-one rows as a catalog
    -- migration and does not invoke their append-only UPDATE trigger.
    ADD COLUMN execution_ordinal INTEGER DEFAULT 1;

-- PostgreSQL truncates long auto-generated constraint identifiers, so locate
-- the retained single-column UNIQUE by its exact column census rather than by
-- a guessed name.
DO $migration$
DECLARE
    source_schedule_unique_names NAME[];
BEGIN
    SELECT array_agg(constraint_row.conname ORDER BY constraint_row.conname)
      INTO source_schedule_unique_names
      FROM pg_constraint constraint_row
      JOIN pg_attribute source_schedule_column
        ON source_schedule_column.attrelid=constraint_row.conrelid
       AND source_schedule_column.attname='source_schedule_receipt_id'
     WHERE constraint_row.conrelid='investigation_asset_primary_rearms'::REGCLASS
       AND constraint_row.contype='u'
       AND constraint_row.conkey=ARRAY[source_schedule_column.attnum]::SMALLINT[];
    IF cardinality(source_schedule_unique_names) IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_SOURCE_UNIQUE_CENSUS_MISMATCH';
    END IF;
    EXECUTE format('ALTER TABLE investigation_asset_primary_rearms DROP CONSTRAINT %I',
        source_schedule_unique_names[1]);
END;
$migration$;

ALTER TABLE investigation_asset_primary_rearms
    ALTER COLUMN execution_ordinal SET NOT NULL,
    ALTER COLUMN execution_ordinal DROP DEFAULT,
    ADD CONSTRAINT investigation_asset_primary_rearms_execution_ordinal_check
        CHECK(execution_ordinal BETWEEN 1 AND 32),
    ADD CONSTRAINT investigation_asset_primary_rearms_predecessor_shape_check
        CHECK((execution_ordinal=1 AND predecessor_rearm_receipt_id IS NULL)
           OR (execution_ordinal>1 AND predecessor_rearm_receipt_id IS NOT NULL)),
    ADD CONSTRAINT investigation_asset_primary_rearms_predecessor_fk
        FOREIGN KEY(predecessor_rearm_receipt_id)
        REFERENCES investigation_asset_primary_rearms(rearm_receipt_id) ON DELETE RESTRICT,
    ADD CONSTRAINT investigation_asset_primary_rearms_predecessor_unique
        UNIQUE(predecessor_rearm_receipt_id),
    ADD CONSTRAINT investigation_asset_primary_rearms_schedule_ordinal_unique
        UNIQUE(source_schedule_receipt_id,execution_ordinal);

CREATE FUNCTION investigation_asset_primary_continuation_receipt_sha256(
    p_rearm_receipt_id UUID,p_stable_request_id UUID,p_source_schedule_receipt_id UUID,
    p_predecessor_rearm_receipt_id UUID,p_execution_ordinal INTEGER,
    p_asset_lane_id UUID,p_target_id UUID,p_asset_context_sha256 TEXT,p_evolution_epoch INTEGER,
    p_successor_schedule_round INTEGER,p_stage_team_plan_id UUID,p_operation_id UUID,
    p_stage_execution_id UUID,p_stage_run_unit_id UUID,p_scope_snapshot_id UUID,
    p_organization_id UUID,p_source_dispatch_epoch BIGINT,p_resume_dispatch_epoch BIGINT,
    p_source_plan_row_version BIGINT,p_previous_primary_work_item_id UUID,
    p_previous_primary_worker_run_id UUID,p_previous_primary_item_row_version BIGINT,
    p_previous_primary_attempt_epoch BIGINT,p_previous_primary_checkpoint_version BIGINT,
    p_source_exhaustion_output_id UUID,p_source_exhaustion_output_sha256 TEXT,
    p_primary_work_item_id UUID,p_primary_worker_run_id UUID,p_primary_message_chain_id UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_asset_primary_execution_continuation.v2',
        'rearm_receipt_id',p_rearm_receipt_id,'stable_request_id',p_stable_request_id,
        'source_schedule_receipt_id',p_source_schedule_receipt_id,
        'predecessor_rearm_receipt_id',p_predecessor_rearm_receipt_id,
        'execution_ordinal',p_execution_ordinal,
        'asset_lane_id',p_asset_lane_id,'target_id',p_target_id,
        'asset_context_sha256',p_asset_context_sha256,'evolution_epoch',p_evolution_epoch,
        'successor_schedule_round',p_successor_schedule_round,
        'stage_team_plan_id',p_stage_team_plan_id,'operation_id',p_operation_id,
        'stage_execution_id',p_stage_execution_id,'stage_run_unit_id',p_stage_run_unit_id,
        'scope_snapshot_id',p_scope_snapshot_id,'organization_id',p_organization_id,
        'source_dispatch_epoch',p_source_dispatch_epoch,'resume_dispatch_epoch',p_resume_dispatch_epoch,
        'source_plan_row_version',p_source_plan_row_version,
        'previous_primary_work_item_id',p_previous_primary_work_item_id,
        'previous_primary_worker_run_id',p_previous_primary_worker_run_id,
        'previous_primary_item_row_version',p_previous_primary_item_row_version,
        'previous_primary_attempt_epoch',p_previous_primary_attempt_epoch,
        'previous_primary_checkpoint_version',p_previous_primary_checkpoint_version,
        'source_exhaustion_output_id',p_source_exhaustion_output_id,
        'source_exhaustion_output_sha256',p_source_exhaustion_output_sha256,
        'primary_work_item_id',p_primary_work_item_id,'primary_worker_run_id',p_primary_worker_run_id,
        'primary_message_chain_id',p_primary_message_chain_id
    )::TEXT)
$$;

-- A PentAGI task plan keeps its original logical Primary dispatch immutable.
-- A later execution shell may therefore consume work for a dispatch owned by
-- either the root schedule worker or any applied predecessor in the current
-- source-schedule lineage.  The current-authority join prevents a historical
-- or foreign asset lineage from authorizing new Refiner writes.
CREATE FUNCTION investigation_asset_primary_dispatch_in_current_lineage(
    p_stage_team_plan_id UUID,p_operation_id UUID,p_stage_execution_id UUID,
    p_stage_run_unit_id UUID,p_scope_snapshot_id UUID,p_organization_id UUID,
    p_dispatch_worker_run_id UUID
) RETURNS BOOLEAN LANGUAGE SQL STABLE STRICT AS $$
    SELECT EXISTS(
        SELECT 1
          FROM investigation_asset_primary_current_authorities current_primary
          JOIN investigation_asset_primary_schedules root_schedule
            ON root_schedule.schedule_receipt_id=current_primary.source_schedule_receipt_id
         WHERE current_primary.stage_team_plan_id=p_stage_team_plan_id
           AND current_primary.operation_id=p_operation_id
           AND current_primary.stage_execution_id=p_stage_execution_id
           AND current_primary.stage_run_unit_id=p_stage_run_unit_id
           AND current_primary.scope_snapshot_id=p_scope_snapshot_id
           AND current_primary.organization_id=p_organization_id
           AND root_schedule.schedule_contract='primary_dynamic_v2'
           AND root_schedule.status='applied'
           AND (root_schedule.primary_worker_run_id=p_dispatch_worker_run_id
             OR EXISTS(
                SELECT 1
                  FROM investigation_asset_primary_rearms lineage
                 WHERE lineage.source_schedule_receipt_id=root_schedule.schedule_receipt_id
                   AND lineage.status='applied'
                   AND lineage.primary_worker_run_id=p_dispatch_worker_run_id)))
$$;

CREATE OR REPLACE FUNCTION enforce_investigation_asset_primary_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    lane investigation_asset_lanes%ROWTYPE;
    schedule investigation_asset_primary_schedules%ROWTYPE;
    predecessor investigation_asset_primary_rearms%ROWTYPE;
    source_item stage_work_items%ROWTYPE;
    source_worker stage_worker_runs%ROWTYPE;
    source_output stage_worker_outputs%ROWTYPE;
    expected_receipt_sha256 TEXT;
    expected_rearm_receipt_id UUID;
    expected_stable_request_id UUID;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_APPEND_ONLY';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(NEW.rearm_receipt_id,NEW.stable_request_id,NEW.source_schedule_receipt_id,
            NEW.predecessor_rearm_receipt_id,NEW.execution_ordinal,
            NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
            NEW.successor_schedule_round,NEW.stage_team_plan_id,NEW.operation_id,
            NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
            NEW.organization_id,NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
            NEW.source_plan_row_version,NEW.previous_primary_work_item_id,
            NEW.previous_primary_worker_run_id,NEW.previous_primary_item_row_version,
            NEW.previous_primary_attempt_epoch,NEW.previous_primary_checkpoint_version,
            NEW.source_exhaustion_output_id,NEW.source_exhaustion_output_sha256,
            NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id,
            NEW.receipt_sha256,NEW.created_at)
        IS DISTINCT FROM ROW(OLD.rearm_receipt_id,OLD.stable_request_id,OLD.source_schedule_receipt_id,
            OLD.predecessor_rearm_receipt_id,OLD.execution_ordinal,
            OLD.asset_lane_id,OLD.target_id,OLD.asset_context_sha256,OLD.evolution_epoch,
            OLD.successor_schedule_round,OLD.stage_team_plan_id,OLD.operation_id,
            OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
            OLD.organization_id,OLD.source_dispatch_epoch,OLD.resume_dispatch_epoch,
            OLD.source_plan_row_version,OLD.previous_primary_work_item_id,
            OLD.previous_primary_worker_run_id,OLD.previous_primary_item_row_version,
            OLD.previous_primary_attempt_epoch,OLD.previous_primary_checkpoint_version,
            OLD.source_exhaustion_output_id,OLD.source_exhaustion_output_sha256,
            OLD.primary_work_item_id,OLD.primary_worker_run_id,OLD.primary_message_chain_id,
            OLD.receipt_sha256,OLD.created_at)
           OR OLD.status<>'building' OR NEW.status<>'applied'
           OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
           OR NOT EXISTS(
              SELECT 1 FROM stage_team_plans persisted_plan
               JOIN stage_work_items item ON item.id=OLD.primary_work_item_id
               JOIN stage_worker_runs worker ON worker.id=OLD.primary_worker_run_id
                    AND worker.work_item_id=item.id
               WHERE persisted_plan.id=OLD.stage_team_plan_id
                 AND persisted_plan.dispatch_epoch=OLD.resume_dispatch_epoch
                 AND persisted_plan.row_version=OLD.source_plan_row_version+1
                 AND persisted_plan.requests_closed_at IS NULL
                 AND item.team_plan_id=persisted_plan.id
                 AND item.dispatch_epoch=OLD.resume_dispatch_epoch
                 AND item.kind='investigation_asset_primary'
                 AND item.stable_key='asset:' || OLD.asset_lane_id::TEXT || ':primary:' ||
                     OLD.evolution_epoch::TEXT || ':round:' || OLD.successor_schedule_round::TEXT
                 AND item.status='queued' AND item.terminal_at IS NULL
                 AND worker.status='queued' AND worker.terminal_at IS NULL
                 AND worker.message_chain_id=OLD.primary_message_chain_id
                 AND (
                   (OLD.execution_ordinal=1 AND worker.worker_generation=0
                    AND worker.parent_request_id IS NULL)
                   OR
                   (OLD.execution_ordinal>1
                    AND worker.worker_generation=(SELECT predecessor_worker.worker_generation+1
                         FROM stage_worker_runs predecessor_worker
                        WHERE predecessor_worker.id=OLD.previous_primary_worker_run_id)
                    AND (SELECT COUNT(DISTINCT task_plan.task_plan_id)
                           FROM investigation_pentagi_task_plans task_plan
                           JOIN pentagi_logical_dispatch_receipts dispatch
                             ON dispatch.task_plan_id=task_plan.task_plan_id
                            AND dispatch.actor_kind='primary' AND dispatch.subtask_id IS NULL
                          WHERE task_plan.operation_id=OLD.operation_id
                            AND task_plan.stage_execution_id=OLD.stage_execution_id
                            AND task_plan.stage_run_unit_id=OLD.stage_run_unit_id
                            AND task_plan.organization_id=OLD.organization_id
                            AND task_plan.status='open'
                            AND investigation_asset_primary_dispatch_in_current_lineage(
                                OLD.stage_team_plan_id,OLD.operation_id,
                                OLD.stage_execution_id,OLD.stage_run_unit_id,
                                OLD.scope_snapshot_id,OLD.organization_id,
                                dispatch.worker_run_id))<=1
                    AND worker.parent_request_id IS NOT DISTINCT FROM
                        CASE WHEN (SELECT COUNT(DISTINCT task_plan.task_plan_id)
                           FROM investigation_pentagi_task_plans task_plan
                           JOIN pentagi_logical_dispatch_receipts dispatch
                             ON dispatch.task_plan_id=task_plan.task_plan_id
                            AND dispatch.actor_kind='primary' AND dispatch.subtask_id IS NULL
                          WHERE task_plan.operation_id=OLD.operation_id
                            AND task_plan.stage_execution_id=OLD.stage_execution_id
                            AND task_plan.stage_run_unit_id=OLD.stage_run_unit_id
                            AND task_plan.organization_id=OLD.organization_id
                            AND task_plan.status='open'
                            AND investigation_asset_primary_dispatch_in_current_lineage(
                                OLD.stage_team_plan_id,OLD.operation_id,
                                OLD.stage_execution_id,OLD.stage_run_unit_id,
                                OLD.scope_snapshot_id,OLD.organization_id,
                                dispatch.worker_run_id))=1
                        THEN 'investigation-task-primary-infrastructure-recovery:' ||
                             OLD.previous_primary_worker_run_id::TEXT
                        ELSE NULL END)
                 ))
        THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_APPEND_ONLY'; END IF;
        RETURN NEW;
    END IF;

    SELECT * INTO STRICT plan FROM stage_team_plans WHERE id=NEW.stage_team_plan_id FOR SHARE;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    SELECT * INTO STRICT schedule FROM investigation_asset_primary_schedules
     WHERE schedule_receipt_id=NEW.source_schedule_receipt_id FOR SHARE;
    SELECT * INTO STRICT source_item FROM stage_work_items
     WHERE id=NEW.previous_primary_work_item_id FOR SHARE;
    SELECT * INTO STRICT source_worker FROM stage_worker_runs
     WHERE id=NEW.previous_primary_worker_run_id FOR SHARE;
    SELECT * INTO STRICT source_output FROM stage_worker_outputs
     WHERE id=NEW.source_exhaustion_output_id FOR SHARE;

    IF NEW.execution_ordinal=1 THEN
        expected_rearm_receipt_id := uuid_generate_v5(NEW.source_schedule_receipt_id,
            'investigation-asset-primary-execution-rearm-v1');
        expected_stable_request_id := uuid_generate_v5(expected_rearm_receipt_id,
            'investigation-asset-primary-execution-rearm-request-v1');
        expected_receipt_sha256 := investigation_asset_primary_rearm_receipt_sha256(
            NEW.rearm_receipt_id,NEW.stable_request_id,NEW.source_schedule_receipt_id,
            NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
            NEW.successor_schedule_round,NEW.stage_team_plan_id,NEW.operation_id,
            NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
            NEW.organization_id,NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
            NEW.source_plan_row_version,NEW.previous_primary_work_item_id,
            NEW.previous_primary_worker_run_id,NEW.previous_primary_item_row_version,
            NEW.previous_primary_attempt_epoch,NEW.previous_primary_checkpoint_version,
            NEW.source_exhaustion_output_id,NEW.source_exhaustion_output_sha256,
            NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id);
        IF NEW.predecessor_rearm_receipt_id IS NOT NULL
           OR ROW(schedule.primary_work_item_id,schedule.primary_worker_run_id,
                  schedule.primary_message_chain_id)
              IS DISTINCT FROM ROW(NEW.previous_primary_work_item_id,
                  NEW.previous_primary_worker_run_id,NEW.primary_message_chain_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_AUTHORITY_MISMATCH'; END IF;
    ELSE
        SELECT * INTO STRICT predecessor FROM investigation_asset_primary_rearms
         WHERE rearm_receipt_id=NEW.predecessor_rearm_receipt_id FOR SHARE;
        expected_rearm_receipt_id := uuid_generate_v5(predecessor.rearm_receipt_id,
            'investigation-asset-primary-execution-continuation-v2:' || NEW.execution_ordinal::TEXT);
        expected_stable_request_id := uuid_generate_v5(expected_rearm_receipt_id,
            'investigation-asset-primary-execution-continuation-request-v2');
        expected_receipt_sha256 := investigation_asset_primary_continuation_receipt_sha256(
            NEW.rearm_receipt_id,NEW.stable_request_id,NEW.source_schedule_receipt_id,
            NEW.predecessor_rearm_receipt_id,NEW.execution_ordinal,
            NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
            NEW.successor_schedule_round,NEW.stage_team_plan_id,NEW.operation_id,
            NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
            NEW.organization_id,NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
            NEW.source_plan_row_version,NEW.previous_primary_work_item_id,
            NEW.previous_primary_worker_run_id,NEW.previous_primary_item_row_version,
            NEW.previous_primary_attempt_epoch,NEW.previous_primary_checkpoint_version,
            NEW.source_exhaustion_output_id,NEW.source_exhaustion_output_sha256,
            NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id);
        IF predecessor.status<>'applied' OR predecessor.applied_at IS NULL
           OR predecessor.source_schedule_receipt_id<>NEW.source_schedule_receipt_id
           OR predecessor.execution_ordinal<>NEW.execution_ordinal-1
           OR ROW(predecessor.asset_lane_id,predecessor.target_id,
                  predecessor.asset_context_sha256,predecessor.evolution_epoch,
                  predecessor.successor_schedule_round,predecessor.stage_team_plan_id,
                  predecessor.operation_id,predecessor.stage_execution_id,
                  predecessor.stage_run_unit_id,predecessor.scope_snapshot_id,
                  predecessor.organization_id,predecessor.primary_work_item_id,
                  predecessor.primary_worker_run_id,predecessor.primary_message_chain_id)
              IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_id,
                  NEW.asset_context_sha256,NEW.evolution_epoch,
                  NEW.successor_schedule_round-1,NEW.stage_team_plan_id,
                  NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
                  NEW.scope_snapshot_id,NEW.organization_id,
                  NEW.previous_primary_work_item_id,NEW.previous_primary_worker_run_id,
                  NEW.primary_message_chain_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_AUTHORITY_MISMATCH'; END IF;
    END IF;

    IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
       OR NEW.execution_ordinal NOT BETWEEN 1 AND 32
       OR NEW.successor_schedule_round<>schedule.schedule_round+NEW.execution_ordinal
       OR NEW.rearm_receipt_id<>expected_rearm_receipt_id
       OR NEW.stable_request_id<>expected_stable_request_id
       OR NEW.primary_work_item_id<>uuid_generate_v5(NEW.asset_lane_id,
            'investigation-asset-primary-work-item-v2:' || NEW.evolution_epoch::TEXT || ':' ||
            NEW.successor_schedule_round::TEXT)
       OR NEW.primary_worker_run_id<>uuid_generate_v5(NEW.asset_lane_id,
            'investigation-asset-primary-worker-v2:' || NEW.evolution_epoch::TEXT || ':' ||
            NEW.successor_schedule_round::TEXT)
       OR NEW.receipt_sha256<>expected_receipt_sha256
       OR schedule.schedule_contract<>'primary_dynamic_v2' OR schedule.status<>'applied'
       OR ROW(schedule.asset_lane_id,schedule.target_id,schedule.asset_context_sha256,
              schedule.evolution_epoch,schedule.stage_team_plan_id,schedule.operation_id,
              schedule.stage_execution_id,schedule.stage_run_unit_id,schedule.scope_snapshot_id,
              schedule.organization_id,schedule.primary_message_chain_id)
          IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,
              NEW.evolution_epoch,NEW.stage_team_plan_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.primary_message_chain_id)
       OR ROW(plan.operation_id,plan.stage_execution_id,plan.stage_run_unit_id,
              plan.scope_snapshot_id,plan.organization_id,plan.dispatch_epoch,plan.row_version)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
              NEW.scope_snapshot_id,NEW.organization_id,NEW.source_dispatch_epoch,
              NEW.source_plan_row_version)
       OR plan.stage_kind<>'investigation' OR plan.requests_closed_at IS NULL
       OR plan.final_submitter_worker_run_id IS NOT NULL
       OR plan.dynamic_request_policy->>'coordination_mode'<>'investigation_task_orchestrator'
       OR NOT EXISTS(
            SELECT 1 FROM investigation_stage_team_effective_contracts effective
             WHERE effective.stage_team_plan_id=NEW.stage_team_plan_id
               AND effective.operation_id=NEW.operation_id
               AND effective.stage_execution_id=NEW.stage_execution_id
               AND effective.stage_run_unit_id=NEW.stage_run_unit_id
               AND effective.scope_snapshot_id=NEW.scope_snapshot_id
               AND effective.organization_id=NEW.organization_id
               AND effective.status='applied' AND effective.applied_at IS NOT NULL
               AND effective.effective_plan_hash=plan.plan_hash
               AND effective.effective_spec_hash=plan.created_from_stage_spec_hash
               AND effective.effective_allowed_roles=plan.allowed_worker_roles
               AND effective.effective_max_workers_total=plan.max_workers_total
               AND effective.effective_max_workers_active=plan.max_workers_active
               AND effective.effective_dynamic_request_policy=plan.dynamic_request_policy)
       OR ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,
              lane.organization_id,lane.target_id,lane.target_identity_sha256,
              lane.evolution_epoch)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.target_id,NEW.asset_context_sha256,
              NEW.evolution_epoch)
       OR lane.state NOT IN('analyzing','verifying','consolidating','evolving')
       OR ROW(source_item.team_plan_id,source_item.dispatch_epoch,source_item.status,
              source_item.row_version,source_item.terminal_at IS NOT NULL)
          IS DISTINCT FROM ROW(NEW.stage_team_plan_id,NEW.source_dispatch_epoch,'exhausted'::TEXT,
              NEW.previous_primary_item_row_version,TRUE)
       OR source_worker.work_item_id<>source_item.id OR source_worker.status<>'failed'
       OR source_worker.attempt_epoch<>NEW.previous_primary_attempt_epoch
       OR source_worker.checkpoint_version<>NEW.previous_primary_checkpoint_version
       OR source_worker.message_chain_id<>NEW.primary_message_chain_id
       OR source_worker.terminal_at IS NULL OR source_worker.lease_token IS NOT NULL
       OR source_worker.active_tool_call_id IS NOT NULL
       OR source_output.work_item_id<>source_item.id OR source_output.worker_run_id<>source_worker.id
       OR source_output.output_hash<>NEW.source_exhaustion_output_sha256
       OR source_output.business_disposition<>'blocked'
       OR source_output.canonical_output->>'kind'<>'stage_team_attempts_exhausted'
       OR source_output.canonical_output->>'failure_code'<>'stage_team_worker_lease_expired'
       OR NOT ('STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=ANY(source_output.blocker_codes))
       OR (SELECT COUNT(*) FROM stage_worker_runs worker WHERE worker.work_item_id=source_item.id)<>1
       OR (SELECT COUNT(*) FROM stage_worker_outputs output WHERE output.work_item_id=source_item.id)<>1
       OR EXISTS(SELECT 1 FROM stage_worker_runs worker
                  WHERE worker.stage_run_unit_id=NEW.stage_run_unit_id
                    AND worker.status IN('queued','running','waiting_background','recovery_required'))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_AUTHORITY_MISMATCH'; END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE VIEW investigation_asset_primary_current_authorities AS
SELECT schedule.schedule_receipt_id AS source_schedule_receipt_id,
       schedule.asset_lane_id,schedule.target_id,schedule.asset_context_sha256,
       schedule.evolution_epoch,schedule.schedule_round,schedule.stage_team_plan_id,
       schedule.operation_id,schedule.stage_execution_id,schedule.stage_run_unit_id,
       schedule.scope_snapshot_id,schedule.organization_id,schedule.resume_dispatch_epoch,
       schedule.primary_work_item_id,schedule.primary_worker_run_id,
       schedule.primary_message_chain_id,NULL::UUID AS execution_rearm_receipt_id,
       0::INTEGER AS execution_ordinal,
       schedule.primary_work_item_id AS authority_primary_work_item_id,
       schedule.primary_worker_run_id AS authority_primary_worker_run_id
  FROM investigation_asset_primary_schedules schedule
 WHERE schedule.schedule_contract='primary_dynamic_v2' AND schedule.status='applied'
   AND NOT EXISTS(SELECT 1 FROM investigation_asset_primary_rearms rearm
                   WHERE rearm.source_schedule_receipt_id=schedule.schedule_receipt_id
                     AND rearm.status='applied')
UNION ALL
SELECT rearm.source_schedule_receipt_id,rearm.asset_lane_id,rearm.target_id,
       rearm.asset_context_sha256,rearm.evolution_epoch,rearm.successor_schedule_round,
       rearm.stage_team_plan_id,rearm.operation_id,rearm.stage_execution_id,
       rearm.stage_run_unit_id,rearm.scope_snapshot_id,rearm.organization_id,
       rearm.resume_dispatch_epoch,rearm.primary_work_item_id,rearm.primary_worker_run_id,
       rearm.primary_message_chain_id,rearm.rearm_receipt_id,rearm.execution_ordinal,
       rearm.previous_primary_work_item_id,rearm.previous_primary_worker_run_id
  FROM investigation_asset_primary_rearms rearm
 WHERE rearm.status='applied'
   AND NOT EXISTS(SELECT 1 FROM investigation_asset_primary_rearms successor
                   WHERE successor.predecessor_rearm_receipt_id=rearm.rearm_receipt_id
                     AND successor.status='applied');

-- Refiner writes continue against the immutable logical task-plan dispatch.
-- Replacing these two entrypoints makes that dispatch legal when it belongs to
-- any worker in the current Asset Primary execution lineage, without rewriting
-- the original dispatch receipt.
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
    SELECT * INTO STRICT dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    IF NOT investigation_asset_primary_dispatch_in_current_lineage(
        plan.stage_team_plan_id,plan.operation_id,plan.stage_execution_id,
        plan.stage_run_unit_id,plan.scope_snapshot_id,plan.organization_id,
        dispatch.worker_run_id)
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

CREATE OR REPLACE FUNCTION append_investigation_refiner_plan_patch_v2(
    p_patch_id UUID,p_stable_request_id UUID,p_ledger_id UUID,p_task_plan_id UUID,
    p_refiner_pipeline_event_id UUID,p_expected_previous_state_sha256 TEXT,
    p_remaining_plan_payload JSONB,p_ordered_active_subtask_ids UUID[]
) RETURNS investigation_refiner_plan_patches LANGUAGE plpgsql AS $$
DECLARE
    existing investigation_refiner_plan_patches%ROWTYPE;
    ledger investigation_refiner_plan_ledgers%ROWTYPE;
    plan investigation_pentagi_task_plans%ROWTYPE;
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
    SELECT * INTO STRICT plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=p_task_plan_id AND status='open' FOR SHARE;
    SELECT * INTO STRICT primary_dispatch FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=p_task_plan_id AND actor_kind='primary' FOR SHARE;
    IF NOT investigation_asset_primary_dispatch_in_current_lineage(
        plan.stage_team_plan_id,plan.operation_id,plan.stage_execution_id,
        plan.stage_run_unit_id,plan.scope_snapshot_id,plan.organization_id,
        primary_dispatch.worker_run_id)
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
