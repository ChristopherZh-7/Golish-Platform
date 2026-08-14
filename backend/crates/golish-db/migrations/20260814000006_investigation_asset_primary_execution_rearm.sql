-- One bounded, append-only execution rearm for an exhausted dynamic Asset
-- Primary.  The failed WorkItem/Worker/output remain immutable audit evidence;
-- the rearm opens a fresh request epoch and installs a successor Primary on
-- the same durable message chain.

CREATE TABLE investigation_asset_primary_rearms (
    rearm_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    source_schedule_receipt_id UUID NOT NULL UNIQUE
        REFERENCES investigation_asset_primary_schedules(schedule_receipt_id) ON DELETE RESTRICT,
    asset_lane_id UUID NOT NULL REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    target_id UUID NOT NULL,
    asset_context_sha256 TEXT NOT NULL CHECK(asset_context_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    evolution_epoch INTEGER NOT NULL CHECK(evolution_epoch>=0),
    successor_schedule_round INTEGER NOT NULL CHECK(successor_schedule_round>0),
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL CHECK(resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK(source_plan_row_version>=0),
    previous_primary_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    previous_primary_worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    previous_primary_item_row_version BIGINT NOT NULL CHECK(previous_primary_item_row_version>=0),
    previous_primary_attempt_epoch BIGINT NOT NULL CHECK(previous_primary_attempt_epoch>=0),
    previous_primary_checkpoint_version BIGINT NOT NULL CHECK(previous_primary_checkpoint_version>=0),
    source_exhaustion_output_id UUID NOT NULL UNIQUE REFERENCES stage_worker_outputs(id) ON DELETE RESTRICT,
    source_exhaustion_output_sha256 TEXT NOT NULL CHECK(source_exhaustion_output_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    primary_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    primary_worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    primary_message_chain_id UUID NOT NULL REFERENCES message_chains(id) ON DELETE RESTRICT,
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK(status IN('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    UNIQUE(stage_team_plan_id,resume_dispatch_epoch),
    CHECK((status='building' AND applied_at IS NULL)
       OR (status='applied' AND applied_at IS NOT NULL)),
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(id,operation_id,stage_execution_id,stage_run_unit_id,
                                    scope_snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_asset_primary_rearm_receipt_sha256(
    p_rearm_receipt_id UUID,p_stable_request_id UUID,p_source_schedule_receipt_id UUID,
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
        'domain','investigation_asset_primary_execution_rearm.v1',
        'rearm_receipt_id',p_rearm_receipt_id,'stable_request_id',p_stable_request_id,
        'source_schedule_receipt_id',p_source_schedule_receipt_id,
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

CREATE FUNCTION enforce_investigation_asset_primary_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    lane investigation_asset_lanes%ROWTYPE;
    schedule investigation_asset_primary_schedules%ROWTYPE;
    source_item stage_work_items%ROWTYPE;
    source_worker stage_worker_runs%ROWTYPE;
    source_output stage_worker_outputs%ROWTYPE;
    expected_receipt_sha256 TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_APPEND_ONLY';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(NEW.rearm_receipt_id,NEW.stable_request_id,NEW.source_schedule_receipt_id,
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
                 AND worker.message_chain_id=OLD.primary_message_chain_id)
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
    IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
       OR NEW.rearm_receipt_id<>uuid_generate_v5(NEW.source_schedule_receipt_id,
            'investigation-asset-primary-execution-rearm-v1')
       OR NEW.stable_request_id<>uuid_generate_v5(NEW.rearm_receipt_id,
            'investigation-asset-primary-execution-rearm-request-v1')
       OR NEW.successor_schedule_round<>schedule.schedule_round+1
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
              schedule.organization_id,schedule.primary_work_item_id,
              schedule.primary_worker_run_id,schedule.primary_message_chain_id)
          IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,
              NEW.evolution_epoch,NEW.stage_team_plan_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.previous_primary_work_item_id,
              NEW.previous_primary_worker_run_id,NEW.primary_message_chain_id)
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

CREATE TRIGGER investigation_asset_primary_rearms_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_primary_rearms
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_asset_primary_rearm();

CREATE FUNCTION investigation_require_asset_primary_rearm_applied()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS(SELECT 1 FROM investigation_asset_primary_rearms rearm
               WHERE rearm.rearm_receipt_id=NEW.rearm_receipt_id
                 AND (rearm.status<>'applied' OR rearm.applied_at IS NULL))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_REARM_NOT_APPLIED'; END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_asset_primary_rearms_complete
AFTER INSERT OR UPDATE ON investigation_asset_primary_rearms
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_require_asset_primary_rearm_applied();

CREATE VIEW investigation_asset_primary_current_authorities AS
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
       rearm.primary_message_chain_id,rearm.rearm_receipt_id,1,
       rearm.previous_primary_work_item_id,rearm.previous_primary_worker_run_id
  FROM investigation_asset_primary_rearms rearm
 WHERE rearm.status='applied';

-- Preserve the exact latest (00005) TeamPlan contract and add only the
-- successor Asset Primary rearm to the existing epoch-advance authority set.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    resolved_round_rearm_id UUID := NULLIF(current_setting(
        'golish.investigation_asset_verification_round_rearm_id',TRUE),'')::UUID;
    round_advance BOOLEAN := FALSE;
    existing_epoch_advance BOOLEAN := FALSE;
    investigation_contract_upgrade BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE'; END IF;
    IF TG_OP='INSERT' THEN
        IF NOT (NEW.allowed_worker_roles ? NEW.leader_role)
           OR (NEW.aggregator_kind='worker' AND NOT (NEW.allowed_worker_roles ? NEW.aggregator_role))
        THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_ROLE_NOT_ALLOWED'; END IF;
        IF EXISTS(SELECT 1 FROM stage_worker_runs worker
                   WHERE worker.stage_run_unit_id=NEW.stage_run_unit_id)
        THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS'; END IF;
        RETURN NEW;
    END IF;
    investigation_contract_upgrade :=
        ROW(NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
            NEW.scope_snapshot_id,NEW.organization_id,NEW.stage_kind,NEW.unit_generation,
            NEW.schema_version,NEW.plan_version,NEW.leader_role,NEW.aggregator_kind,
            NEW.aggregator_role,NEW.dynamic_requests_allowed,NEW.final_submitter_kind,
            NEW.created_at,NEW.dispatch_epoch,NEW.requests_closed_at,
            NEW.final_submitter_worker_run_id)
        IS NOT DISTINCT FROM
        ROW(OLD.id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
            OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,OLD.unit_generation,
            OLD.schema_version,OLD.plan_version,OLD.leader_role,OLD.aggregator_kind,
            OLD.aggregator_role,OLD.dynamic_requests_allowed,OLD.final_submitter_kind,
            OLD.created_at,OLD.dispatch_epoch,OLD.requests_closed_at,
            OLD.final_submitter_worker_run_id)
        AND EXISTS(SELECT 1 FROM investigation_stage_team_effective_contracts authority
             WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
               AND ROW(authority.operation_id,authority.stage_execution_id,
                   authority.stage_run_unit_id,authority.scope_snapshot_id,
                   authority.organization_id,authority.source_plan_hash,
                   authority.source_spec_hash,authority.source_allowed_roles,
                   authority.source_max_workers_total,authority.source_max_workers_active,
                   authority.source_dynamic_request_policy,authority.source_row_version,
                   authority.source_dispatch_epoch)
                 IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
                   OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,OLD.plan_hash,
                   OLD.created_from_stage_spec_hash,OLD.allowed_worker_roles,
                   OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_request_policy,
                   OLD.row_version,OLD.dispatch_epoch)
               AND ROW(authority.effective_plan_hash,authority.effective_spec_hash,
                   authority.effective_allowed_roles,authority.effective_max_workers_total,
                   authority.effective_max_workers_active,authority.effective_dynamic_request_policy)
                 IS NOT DISTINCT FROM ROW(NEW.plan_hash,NEW.created_from_stage_spec_hash,
                   NEW.allowed_worker_roles,NEW.max_workers_total,NEW.max_workers_active,
                   NEW.dynamic_request_policy));
    IF ROW(NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.stage_kind,NEW.unit_generation,
        NEW.schema_version,NEW.plan_version,NEW.plan_hash,NEW.leader_role,
        NEW.aggregator_kind,NEW.aggregator_role,NEW.allowed_worker_roles,
        NEW.max_workers_total,NEW.max_workers_active,NEW.dynamic_requests_allowed,
        NEW.dynamic_request_policy,NEW.final_submitter_kind,
        NEW.created_from_stage_spec_hash,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.operation_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,
        OLD.unit_generation,OLD.schema_version,OLD.plan_version,OLD.plan_hash,
        OLD.leader_role,OLD.aggregator_kind,OLD.aggregator_role,OLD.allowed_worker_roles,
        OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,OLD.created_at)
    AND NOT investigation_contract_upgrade
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE'; END IF;
    round_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL AND NEW.final_submitter_worker_run_id IS NULL
        AND resolved_round_rearm_id IS NOT NULL
        AND EXISTS(SELECT 1 FROM investigation_asset_verification_round_rearms authority
             WHERE authority.round_rearm_id=resolved_round_rearm_id
               AND authority.stage_team_plan_id=OLD.id AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    existing_epoch_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND (
          EXISTS(SELECT 1 FROM stage_team_repair_generations generation
            JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
           WHERE generation.team_plan_id=OLD.id AND generation.dispatch_epoch=NEW.dispatch_epoch
             AND generation.status='building' AND gap.source_dispatch_epoch=OLD.dispatch_epoch
             AND gap.source_aggregator_worker_run_id=OLD.final_submitter_worker_run_id)
          OR EXISTS(SELECT 1 FROM stage_team_controller_turn_resumes authority
           WHERE authority.team_plan_id=OLD.id AND authority.status='building'
             AND authority.source_dispatch_epoch=OLD.dispatch_epoch
             AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
             AND authority.source_plan_row_version=OLD.row_version
             AND authority.leader_worker_run_id=OLD.final_submitter_worker_run_id)
          OR EXISTS(SELECT 1 FROM target_intel_goal_resume_authorities authority
           WHERE authority.team_plan_id=OLD.id AND authority.status='building'
             AND authority.source_goal_epoch=OLD.dispatch_epoch
             AND authority.successor_goal_epoch=NEW.dispatch_epoch
             AND authority.source_plan_row_version=OLD.row_version
             AND authority.controller_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id)
          OR EXISTS(SELECT 1 FROM investigation_task_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,authority.stage_run_unit_id,
               authority.scope_snapshot_id,authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_verification_execution_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,authority.stage_run_unit_id,
               authority.scope_snapshot_id,authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_evolution_analysis_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,authority.stage_run_unit_id,
               authority.scope_snapshot_id,authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_asset_primary_schedules authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,authority.stage_run_unit_id,
               authority.scope_snapshot_id,authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_asset_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,authority.stage_run_unit_id,
               authority.scope_snapshot_id,authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
        );
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED'; END IF;
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch
       AND NOT round_advance AND NOT existing_epoch_advance
    THEN RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR'; END IF;
    IF OLD.requests_closed_at IS NOT NULL
       AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
       AND NOT round_advance AND NOT existing_epoch_advance
    THEN RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN'; END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
       AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
       AND NOT round_advance AND NOT existing_epoch_advance
       AND NOT(NEW.final_submitter_worker_run_id IS NOT NULL AND EXISTS(
           SELECT 1 FROM stage_worker_runs previous_submitter
            WHERE previous_submitter.id=OLD.final_submitter_worker_run_id
              AND previous_submitter.status='superseded'))
    THEN RAISE EXCEPTION 'STAGE_TEAM_FINAL_SUBMITTER_IMMUTABLE'; END IF;
    IF NEW.requests_closed_at IS NOT DISTINCT FROM OLD.requests_closed_at
       AND NEW.dispatch_epoch IS NOT DISTINCT FROM OLD.dispatch_epoch
       AND NEW.final_submitter_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
       AND NOT investigation_contract_upgrade
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_NOOP_UPDATE_FORBIDDEN'; END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
