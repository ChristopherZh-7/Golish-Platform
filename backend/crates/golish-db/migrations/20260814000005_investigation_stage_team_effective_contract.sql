-- Append-only migration-forward authority for retained Investigation plans.
-- Historical TeamPlan material stays immutable. One exact fixed-roster receipt
-- may freeze the source contract and a server-derived effective dynamic
-- contract so later dynamic WorkItems can use the current role catalog without
-- rewriting audit history.

CREATE TABLE investigation_stage_team_effective_contracts (
    contract_authority_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    stage_team_plan_id UUID NOT NULL UNIQUE REFERENCES stage_team_plans(id),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_plan_hash TEXT NOT NULL CHECK(length(source_plan_hash)>0),
    source_spec_hash TEXT NOT NULL CHECK(length(source_spec_hash)>0),
    source_plan_material JSONB NOT NULL CHECK(jsonb_typeof(source_plan_material)='object'),
    source_allowed_roles JSONB NOT NULL CHECK(jsonb_typeof(source_allowed_roles)='array'),
    source_max_workers_total INTEGER NOT NULL CHECK(source_max_workers_total>0),
    source_max_workers_active INTEGER NOT NULL CHECK(source_max_workers_active>0),
    source_dynamic_request_policy JSONB NOT NULL CHECK(jsonb_typeof(source_dynamic_request_policy)='object'),
    source_row_version BIGINT NOT NULL CHECK(source_row_version>=0),
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    effective_plan_hash TEXT NOT NULL CHECK(length(effective_plan_hash)>0),
    effective_spec_hash TEXT NOT NULL CHECK(length(effective_spec_hash)>0),
    effective_plan_material JSONB NOT NULL CHECK(jsonb_typeof(effective_plan_material)='object'),
    effective_allowed_roles JSONB NOT NULL CHECK(jsonb_typeof(effective_allowed_roles)='array'),
    effective_max_workers_total INTEGER NOT NULL CHECK(effective_max_workers_total>0),
    effective_max_workers_active INTEGER NOT NULL CHECK(effective_max_workers_active>0),
    effective_dynamic_request_policy JSONB NOT NULL CHECK(jsonb_typeof(effective_dynamic_request_policy)='object'),
    source_schedule_receipt_id UUID NOT NULL UNIQUE
        REFERENCES investigation_asset_primary_schedules(schedule_receipt_id),
    authority_sha256 TEXT NOT NULL CHECK(length(authority_sha256)>0),
    status TEXT NOT NULL CHECK(status IN ('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    applied_at TIMESTAMPTZ,
    UNIQUE(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
           scope_snapshot_id,organization_id)
);

CREATE FUNCTION investigation_stage_team_effective_contract_sha256(
    requested_contract_authority_id UUID,
    requested_stable_request_id UUID,
    requested_stage_team_plan_id UUID,
    requested_operation_id UUID,
    requested_stage_execution_id UUID,
    requested_stage_run_unit_id UUID,
    requested_scope_snapshot_id UUID,
    requested_organization_id UUID,
    requested_source_plan_hash TEXT,
    requested_source_spec_hash TEXT,
    requested_source_plan_material JSONB,
    requested_source_allowed_roles JSONB,
    requested_source_max_workers_total INTEGER,
    requested_source_max_workers_active INTEGER,
    requested_source_dynamic_request_policy JSONB,
    requested_source_row_version BIGINT,
    requested_source_dispatch_epoch BIGINT,
    requested_effective_plan_hash TEXT,
    requested_effective_spec_hash TEXT,
    requested_effective_plan_material JSONB,
    requested_effective_allowed_roles JSONB,
    requested_effective_max_workers_total INTEGER,
    requested_effective_max_workers_active INTEGER,
    requested_effective_dynamic_request_policy JSONB,
    requested_source_schedule_receipt_id UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_stage_team_effective_contract.v1',
        'contract_authority_id',requested_contract_authority_id,
        'stable_request_id',requested_stable_request_id,
        'stage_team_plan_id',requested_stage_team_plan_id,
        'operation_id',requested_operation_id,
        'stage_execution_id',requested_stage_execution_id,
        'stage_run_unit_id',requested_stage_run_unit_id,
        'scope_snapshot_id',requested_scope_snapshot_id,
        'organization_id',requested_organization_id,
        'source_plan_hash',requested_source_plan_hash,
        'source_spec_hash',requested_source_spec_hash,
        'source_plan_material',requested_source_plan_material,
        'source_allowed_roles',requested_source_allowed_roles,
        'source_max_workers_total',requested_source_max_workers_total,
        'source_max_workers_active',requested_source_max_workers_active,
        'source_dynamic_request_policy',requested_source_dynamic_request_policy,
        'source_row_version',requested_source_row_version,
        'source_dispatch_epoch',requested_source_dispatch_epoch,
        'effective_plan_hash',requested_effective_plan_hash,
        'effective_spec_hash',requested_effective_spec_hash,
        'effective_plan_material',requested_effective_plan_material,
        'effective_allowed_roles',requested_effective_allowed_roles,
        'effective_max_workers_total',requested_effective_max_workers_total,
        'effective_max_workers_active',requested_effective_max_workers_active,
        'effective_dynamic_request_policy',requested_effective_dynamic_request_policy,
        'source_schedule_receipt_id',requested_source_schedule_receipt_id
    )::TEXT)
$$;

CREATE FUNCTION enforce_investigation_stage_team_effective_contract()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    source_schedule investigation_asset_primary_schedules%ROWTYPE;
    expected_source_material JSONB;
    expected_effective_material JSONB;
    expected_authority_sha256 TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_EFFECTIVE_CONTRACT_APPEND_ONLY';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(
            NEW.contract_authority_id,NEW.stable_request_id,NEW.stage_team_plan_id,
            NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
            NEW.scope_snapshot_id,NEW.organization_id,NEW.source_plan_hash,
            NEW.source_spec_hash,NEW.source_plan_material,NEW.source_allowed_roles,
            NEW.source_max_workers_total,NEW.source_max_workers_active,
            NEW.source_dynamic_request_policy,NEW.source_row_version,
            NEW.source_dispatch_epoch,NEW.effective_plan_hash,NEW.effective_spec_hash,
            NEW.effective_plan_material,NEW.effective_allowed_roles,
            NEW.effective_max_workers_total,NEW.effective_max_workers_active,
            NEW.effective_dynamic_request_policy,NEW.source_schedule_receipt_id,
            NEW.authority_sha256,NEW.created_at
        ) IS DISTINCT FROM ROW(
            OLD.contract_authority_id,OLD.stable_request_id,OLD.stage_team_plan_id,
            OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
            OLD.scope_snapshot_id,OLD.organization_id,OLD.source_plan_hash,
            OLD.source_spec_hash,OLD.source_plan_material,OLD.source_allowed_roles,
            OLD.source_max_workers_total,OLD.source_max_workers_active,
            OLD.source_dynamic_request_policy,OLD.source_row_version,
            OLD.source_dispatch_epoch,OLD.effective_plan_hash,OLD.effective_spec_hash,
            OLD.effective_plan_material,OLD.effective_allowed_roles,
            OLD.effective_max_workers_total,OLD.effective_max_workers_active,
            OLD.effective_dynamic_request_policy,OLD.source_schedule_receipt_id,
            OLD.authority_sha256,OLD.created_at
        ) OR OLD.status<>'building' OR NEW.status<>'applied'
          OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
          OR NOT EXISTS(
              SELECT 1 FROM stage_team_plans persisted_plan
               WHERE persisted_plan.id=OLD.stage_team_plan_id
                 AND persisted_plan.operation_id=OLD.operation_id
                 AND persisted_plan.stage_execution_id=OLD.stage_execution_id
                 AND persisted_plan.stage_run_unit_id=OLD.stage_run_unit_id
                 AND persisted_plan.scope_snapshot_id=OLD.scope_snapshot_id
                 AND persisted_plan.organization_id=OLD.organization_id
                 AND persisted_plan.plan_hash=OLD.effective_plan_hash
                 AND persisted_plan.created_from_stage_spec_hash=OLD.effective_spec_hash
                 AND persisted_plan.allowed_worker_roles=OLD.effective_allowed_roles
                 AND persisted_plan.max_workers_total=OLD.effective_max_workers_total
                 AND persisted_plan.max_workers_active=OLD.effective_max_workers_active
                 AND persisted_plan.dynamic_request_policy=OLD.effective_dynamic_request_policy
                 AND persisted_plan.row_version=OLD.source_row_version+1
                 AND persisted_plan.dispatch_epoch=OLD.source_dispatch_epoch)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_EFFECTIVE_CONTRACT_APPEND_ONLY';
        END IF;
        RETURN NEW;
    END IF;

    SELECT * INTO STRICT plan FROM stage_team_plans persisted
     WHERE persisted.id=NEW.stage_team_plan_id FOR SHARE;
    SELECT * INTO STRICT source_schedule
      FROM investigation_asset_primary_schedules persisted
     WHERE persisted.schedule_receipt_id=NEW.source_schedule_receipt_id FOR SHARE;
    expected_source_material := jsonb_build_object(
        'plan_hash',plan.plan_hash,'spec_hash',plan.created_from_stage_spec_hash,
        'allowed_roles',plan.allowed_worker_roles,
        'max_workers_total',plan.max_workers_total,
        'max_workers_active',plan.max_workers_active,
        'dynamic_request_policy',plan.dynamic_request_policy);
    expected_effective_material := jsonb_build_object(
        'plan_hash',NEW.effective_plan_hash,'spec_hash',NEW.effective_spec_hash,
        'allowed_roles',NEW.effective_allowed_roles,
        'max_workers_total',NEW.effective_max_workers_total,
        'max_workers_active',NEW.effective_max_workers_active,
        'dynamic_request_policy',NEW.effective_dynamic_request_policy);
    expected_authority_sha256 := investigation_stage_team_effective_contract_sha256(
        NEW.contract_authority_id,NEW.stable_request_id,NEW.stage_team_plan_id,
        NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.source_plan_hash,
        NEW.source_spec_hash,NEW.source_plan_material,NEW.source_allowed_roles,
        NEW.source_max_workers_total,NEW.source_max_workers_active,
        NEW.source_dynamic_request_policy,NEW.source_row_version,
        NEW.source_dispatch_epoch,NEW.effective_plan_hash,NEW.effective_spec_hash,
        NEW.effective_plan_material,NEW.effective_allowed_roles,
        NEW.effective_max_workers_total,NEW.effective_max_workers_active,
        NEW.effective_dynamic_request_policy,NEW.source_schedule_receipt_id);

    IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
       OR plan.operation_id<>NEW.operation_id
       OR plan.stage_execution_id<>NEW.stage_execution_id
       OR plan.stage_run_unit_id<>NEW.stage_run_unit_id
       OR plan.scope_snapshot_id<>NEW.scope_snapshot_id
       OR plan.organization_id<>NEW.organization_id
       OR plan.stage_kind<>'investigation'
       OR plan.dynamic_request_policy->>'coordination_mode'<>'investigation_task_orchestrator'
       OR plan.plan_hash<>NEW.source_plan_hash
       OR plan.created_from_stage_spec_hash<>NEW.source_spec_hash
       OR plan.allowed_worker_roles<>NEW.source_allowed_roles
       OR plan.max_workers_total<>NEW.source_max_workers_total
       OR plan.max_workers_active<>NEW.source_max_workers_active
       OR plan.dynamic_request_policy<>NEW.source_dynamic_request_policy
       OR plan.row_version<>NEW.source_row_version
       OR plan.dispatch_epoch<>NEW.source_dispatch_epoch
       OR NEW.source_plan_material<>expected_source_material
       OR NEW.effective_plan_material<>expected_effective_material
       OR NEW.effective_dynamic_request_policy<>NEW.source_dynamic_request_policy
       OR NOT (NEW.effective_allowed_roles ? plan.leader_role)
       OR NEW.effective_max_workers_total<NEW.effective_max_workers_active
       OR source_schedule.schedule_contract<>'fixed_roster_v1'
       OR source_schedule.status<>'applied'
       OR source_schedule.stage_team_plan_id<>NEW.stage_team_plan_id
       OR source_schedule.operation_id<>NEW.operation_id
       OR source_schedule.stage_execution_id<>NEW.stage_execution_id
       OR source_schedule.stage_run_unit_id<>NEW.stage_run_unit_id
       OR source_schedule.scope_snapshot_id<>NEW.scope_snapshot_id
       OR source_schedule.organization_id<>NEW.organization_id
       OR source_schedule.resume_dispatch_epoch<>NEW.source_dispatch_epoch
       OR source_schedule.source_plan_row_version+1<>NEW.source_row_version
       OR EXISTS(SELECT 1 FROM stage_team_unit_gaps gap
              WHERE gap.team_plan_id=NEW.stage_team_plan_id)
       OR EXISTS(SELECT 1 FROM stage_team_repair_generations generation
              WHERE generation.team_plan_id=NEW.stage_team_plan_id)
       OR EXISTS(SELECT 1 FROM stage_team_controller_turn_resumes resume
              WHERE resume.team_plan_id=NEW.stage_team_plan_id)
       OR EXISTS(SELECT 1 FROM stage_deliverable_submissions submission
              WHERE submission.operation_id=NEW.operation_id
                AND submission.stage_execution_id=NEW.stage_execution_id
                AND submission.stage_run_unit_id=NEW.stage_run_unit_id)
       OR EXISTS(SELECT 1 FROM stage_worker_requests request
              WHERE request.team_plan_id=NEW.stage_team_plan_id)
       OR (SELECT COUNT(*) FROM stage_work_items item
            WHERE item.team_plan_id=NEW.stage_team_plan_id
              AND item.dispatch_epoch=NEW.source_dispatch_epoch
              AND item.created_by='server_phase_transition'
              AND item.id=ANY(ARRAY[source_schedule.primary_work_item_id,
                  source_schedule.browser_work_item_id,source_schedule.researcher_work_item_id,
                  source_schedule.pentester_work_item_id,source_schedule.adviser_work_item_id]))<>5
       OR EXISTS(SELECT 1 FROM stage_work_items item
            WHERE item.team_plan_id=NEW.stage_team_plan_id
              AND item.dispatch_epoch=NEW.source_dispatch_epoch
              AND item.id<>ALL(ARRAY[source_schedule.primary_work_item_id,
                  source_schedule.browser_work_item_id,source_schedule.researcher_work_item_id,
                  source_schedule.pentester_work_item_id,source_schedule.adviser_work_item_id]))
       OR EXISTS(SELECT 1 FROM stage_worker_runs worker
              JOIN stage_work_items item ON item.id=worker.work_item_id
             WHERE item.team_plan_id=NEW.stage_team_plan_id
               AND item.dispatch_epoch=NEW.source_dispatch_epoch
               AND item.id=ANY(ARRAY[source_schedule.primary_work_item_id,
                   source_schedule.browser_work_item_id,source_schedule.researcher_work_item_id,
                   source_schedule.pentester_work_item_id,source_schedule.adviser_work_item_id])
               AND (worker.lease_token IS NOT NULL OR worker.active_tool_call_id IS NOT NULL))
       OR NEW.authority_sha256<>expected_authority_sha256
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EFFECTIVE_CONTRACT_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_stage_team_effective_contract_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_stage_team_effective_contracts
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_stage_team_effective_contract();

CREATE FUNCTION investigation_require_effective_contract_applied()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_stage_team_effective_contracts authority
         WHERE authority.contract_authority_id=NEW.contract_authority_id
           AND (authority.status<>'applied' OR authority.applied_at IS NULL)
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_EFFECTIVE_CONTRACT_NOT_APPLIED';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_stage_team_effective_contract_complete
AFTER INSERT OR UPDATE ON investigation_stage_team_effective_contracts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_require_effective_contract_applied();

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
        AND EXISTS(
            SELECT 1 FROM investigation_stage_team_effective_contracts authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.status='building'
               AND ROW(authority.operation_id,authority.stage_execution_id,
                   authority.stage_run_unit_id,authority.scope_snapshot_id,
                   authority.organization_id,authority.source_plan_hash,
                   authority.source_spec_hash,authority.source_allowed_roles,
                   authority.source_max_workers_total,authority.source_max_workers_active,
                   authority.source_dynamic_request_policy,authority.source_row_version,
                   authority.source_dispatch_epoch)
                 IS NOT DISTINCT FROM
                   ROW(OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
                   OLD.scope_snapshot_id,OLD.organization_id,OLD.plan_hash,
                   OLD.created_from_stage_spec_hash,OLD.allowed_worker_roles,
                   OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_request_policy,
                   OLD.row_version,OLD.dispatch_epoch)
               AND ROW(authority.effective_plan_hash,authority.effective_spec_hash,
                   authority.effective_allowed_roles,authority.effective_max_workers_total,
                   authority.effective_max_workers_active,
                   authority.effective_dynamic_request_policy)
                 IS NOT DISTINCT FROM
                   ROW(NEW.plan_hash,NEW.created_from_stage_spec_hash,
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
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND resolved_round_rearm_id IS NOT NULL
        AND EXISTS(SELECT 1 FROM investigation_asset_verification_round_rearms authority
             WHERE authority.round_rearm_id=resolved_round_rearm_id
               AND authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    existing_epoch_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND (
          EXISTS(SELECT 1 FROM stage_team_repair_generations generation
            JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
           WHERE generation.team_plan_id=OLD.id
             AND generation.dispatch_epoch=NEW.dispatch_epoch
             AND generation.status='building'
             AND gap.source_dispatch_epoch=OLD.dispatch_epoch
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
             AND authority.controller_worker_run_id IS NOT DISTINCT FROM
                 OLD.final_submitter_worker_run_id)
          OR EXISTS(SELECT 1 FROM investigation_task_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,
               authority.stage_run_unit_id,authority.scope_snapshot_id,
               authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_verification_execution_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,
               authority.stage_run_unit_id,authority.scope_snapshot_id,
               authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_evolution_analysis_primary_rearms authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,
               authority.stage_run_unit_id,authority.scope_snapshot_id,
               authority.organization_id,authority.source_dispatch_epoch,
               authority.resume_dispatch_epoch,authority.source_plan_row_version)
               IS NOT DISTINCT FROM ROW(OLD.operation_id,OLD.stage_execution_id,
               OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
               OLD.dispatch_epoch,NEW.dispatch_epoch,OLD.row_version))
          OR EXISTS(SELECT 1 FROM investigation_asset_primary_schedules authority
           WHERE authority.stage_team_plan_id=OLD.id AND authority.status='building'
             AND ROW(authority.operation_id,authority.stage_execution_id,
               authority.stage_run_unit_id,authority.scope_snapshot_id,
               authority.organization_id,authority.source_dispatch_epoch,
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
