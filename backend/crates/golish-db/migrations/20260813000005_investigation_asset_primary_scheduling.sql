-- One durable Asset Primary and the exact fixed cognitive roster for each
-- active asset/evolution epoch.  The receipt is inserted before reopening the
-- StageTeam plan, then applied only after all five WorkItems/Worker identities
-- exist and the exact-four barrier denominator can be re-derived.
-- Exact REPLAY returns this immutable applied receipt; any caller identity
-- drift is rejected by the repository before a row can be reused.

CREATE FUNCTION investigation_asset_primary_roster_set_sha256()
RETURNS TEXT LANGUAGE SQL STABLE AS $$
    SELECT tool_truth_sha256(
        '["browser","researcher","pentester","adviser"]'
    )
$$;

CREATE FUNCTION investigation_asset_primary_schedule_receipt_sha256(
    requested_asset_lane_id UUID,
    requested_target_id UUID,
    requested_asset_context_sha256 TEXT,
    requested_evolution_epoch INTEGER,
    requested_stage_team_plan_id UUID,
    requested_resume_dispatch_epoch BIGINT,
    requested_primary_work_item_id UUID,
    requested_primary_worker_run_id UUID,
    requested_primary_message_chain_id UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_asset_primary_schedule_receipt.v1',
        'asset_lane_id',requested_asset_lane_id,
        'target_id',requested_target_id,
        'asset_context_sha256',requested_asset_context_sha256,
        'evolution_epoch',requested_evolution_epoch,
        'stage_team_plan_id',requested_stage_team_plan_id,
        'resume_dispatch_epoch',requested_resume_dispatch_epoch,
        'primary_work_item_id',requested_primary_work_item_id,
        'primary_worker_run_id',requested_primary_worker_run_id,
        'primary_message_chain_id',requested_primary_message_chain_id,
        'roster_set_sha256',investigation_asset_primary_roster_set_sha256()
    )::TEXT)
$$;

CREATE TABLE investigation_asset_primary_schedules (
    schedule_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    asset_lane_id UUID NOT NULL,
    target_id UUID NOT NULL,
    asset_context_sha256 TEXT NOT NULL
        CHECK(asset_context_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    evolution_epoch INTEGER NOT NULL CHECK(evolution_epoch>=0),
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL
        CHECK(resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK(source_plan_row_version>=0),
    primary_work_item_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL UNIQUE,
    primary_message_chain_id UUID NOT NULL,
    browser_work_item_id UUID NOT NULL UNIQUE,
    researcher_work_item_id UUID NOT NULL UNIQUE,
    pentester_work_item_id UUID NOT NULL UNIQUE,
    adviser_work_item_id UUID NOT NULL UNIQUE,
    roster_set_sha256 TEXT NOT NULL
        CHECK(roster_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK(status IN('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    UNIQUE(asset_lane_id,evolution_epoch),
    UNIQUE(stage_team_plan_id,resume_dispatch_epoch),
    CHECK((status='building' AND applied_at IS NULL)
       OR (status='applied' AND applied_at IS NOT NULL)),
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(id,operation_id,stage_execution_id,stage_run_unit_id,
                                    scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(primary_work_item_id) REFERENCES stage_work_items(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(primary_worker_run_id) REFERENCES stage_worker_runs(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(primary_message_chain_id) REFERENCES message_chains(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(browser_work_item_id) REFERENCES stage_work_items(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(researcher_work_item_id) REFERENCES stage_work_items(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(pentester_work_item_id) REFERENCES stage_work_items(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(adviser_work_item_id) REFERENCES stage_work_items(id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE FUNCTION enforce_investigation_asset_primary_schedule()
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
    expected_receipt_id := uuid_generate_v5(
        NEW.asset_lane_id,
        'investigation-asset-primary-schedule-receipt-v1:' || NEW.evolution_epoch::TEXT
    );
    expected_primary_work_item_id := uuid_generate_v5(
        NEW.asset_lane_id,
        'investigation-asset-primary-work-item-v1:' || NEW.evolution_epoch::TEXT
    );
    expected_primary_worker_run_id := uuid_generate_v5(
        NEW.asset_lane_id,
        'investigation-asset-primary-worker-v1:' || NEW.evolution_epoch::TEXT
    );
    expected_primary_message_chain_id := uuid_generate_v5(
        NEW.asset_lane_id,'investigation-asset-primary-chain-v1'
    );
    expected_receipt_sha256 := investigation_asset_primary_schedule_receipt_sha256(
        NEW.asset_lane_id,NEW.target_id,NEW.asset_context_sha256,NEW.evolution_epoch,
        NEW.stage_team_plan_id,NEW.resume_dispatch_epoch,NEW.primary_work_item_id,
        NEW.primary_worker_run_id,NEW.primary_message_chain_id
    );

    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
           OR NEW.schedule_receipt_id<>expected_receipt_id
           OR NEW.stable_request_id<>uuid_generate_v5(
                expected_receipt_id,'investigation-asset-primary-schedule-request-v1')
           OR NEW.primary_work_item_id<>expected_primary_work_item_id
           OR NEW.primary_worker_run_id<>expected_primary_worker_run_id
           OR NEW.primary_message_chain_id<>expected_primary_message_chain_id
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
           OR NEW.roster_set_sha256<>investigation_asset_primary_roster_set_sha256()
           OR NEW.receipt_sha256<>expected_receipt_sha256
           OR plan.operation_id<>NEW.operation_id
           OR plan.stage_execution_id<>NEW.stage_execution_id
           OR plan.stage_run_unit_id<>NEW.stage_run_unit_id
           OR plan.scope_snapshot_id<>NEW.scope_snapshot_id
           OR plan.organization_id<>NEW.organization_id
           OR plan.stage_kind<>'investigation'
           OR plan.dynamic_request_policy->>'coordination_mode'<>
              'investigation_task_orchestrator'
           OR plan.dispatch_epoch<>NEW.source_dispatch_epoch
           OR plan.row_version<>NEW.source_plan_row_version
           OR plan.requests_closed_at IS NULL
           OR plan.final_submitter_worker_run_id IS NOT NULL
           OR NOT (plan.allowed_worker_roles ?&
               ARRAY['browser','researcher','pentester','adviser'])
           OR lane.operation_id<>NEW.operation_id
           OR lane.stage_execution_id<>NEW.stage_execution_id
           OR lane.scope_snapshot_id<>NEW.scope_snapshot_id
           OR lane.organization_id<>NEW.organization_id
           OR lane.target_id<>NEW.target_id
           OR lane.target_identity_sha256<>NEW.asset_context_sha256
           OR lane.evolution_epoch<>NEW.evolution_epoch
           OR lane.state<>'analyzing'
        THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.schedule_receipt_id,NEW.stable_request_id,NEW.asset_lane_id,NEW.target_id,
        NEW.asset_context_sha256,NEW.evolution_epoch,NEW.stage_team_plan_id,
        NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.source_dispatch_epoch,
        NEW.resume_dispatch_epoch,NEW.source_plan_row_version,NEW.primary_work_item_id,
        NEW.primary_worker_run_id,NEW.primary_message_chain_id,NEW.browser_work_item_id,
        NEW.researcher_work_item_id,NEW.pentester_work_item_id,NEW.adviser_work_item_id,
        NEW.roster_set_sha256,NEW.receipt_sha256,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.schedule_receipt_id,OLD.stable_request_id,OLD.asset_lane_id,OLD.target_id,
        OLD.asset_context_sha256,OLD.evolution_epoch,OLD.stage_team_plan_id,
        OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
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
        SELECT 1
          FROM stage_work_items item
          JOIN stage_worker_runs worker
            ON worker.id=NEW.primary_worker_run_id AND worker.work_item_id=item.id
          JOIN message_chains chain
            ON chain.id=NEW.primary_message_chain_id
           AND chain.id=worker.message_chain_id AND chain.task_id=NEW.operation_id
         WHERE item.id=NEW.primary_work_item_id
           AND item.team_plan_id=NEW.stage_team_plan_id
           AND item.dispatch_epoch=NEW.resume_dispatch_epoch
           AND item.kind='investigation_asset_primary'
           AND item.stable_key='asset:' || NEW.asset_lane_id::TEXT || ':primary:' ||
               NEW.evolution_epoch::TEXT
           AND item.role=plan.leader_role
           AND item.input_manifest_hash=NEW.asset_context_sha256
           AND item.input_refs=jsonb_build_array(jsonb_build_object(
               'kind','investigation_asset_lane','asset_lane_id',NEW.asset_lane_id,
               'target_id',NEW.target_id,'asset_context_sha256',NEW.asset_context_sha256,
               'evolution_epoch',NEW.evolution_epoch))
           AND item.required_for_barrier=FALSE
           AND item.created_by='server_phase_transition'
           AND item.output_schema='stage_unit_aggregate.v1'
           AND worker.status='queued'
           AND worker.specialist=plan.leader_role
           AND worker.work_item_kind=item.kind
           AND worker.work_item_key=item.stable_key
    ) INTO primary_exact;

    SELECT COUNT(*)=4 AND BOOL_AND(
               item.kind='investigation_asset_role'
           AND item.stable_key='asset:' || NEW.asset_lane_id::TEXT || ':role:' ||
               item.role || ':' || NEW.evolution_epoch::TEXT
           AND item.input_refs = jsonb_build_array(jsonb_build_object(
               'kind','investigation_asset_lane','asset_lane_id',NEW.asset_lane_id,
               'target_id',NEW.target_id,'asset_context_sha256',NEW.asset_context_sha256,
               'evolution_epoch',NEW.evolution_epoch,'role_slot',item.role))
           AND item.required_for_barrier=TRUE
           AND item.created_by='server_phase_transition'
           AND item.output_schema='investigation_cognitive_output.v1'
           AND item.role=ANY(ARRAY['browser','researcher','pentester','adviser'])
           AND item.id=CASE item.role
                WHEN 'browser' THEN NEW.browser_work_item_id
                WHEN 'researcher' THEN NEW.researcher_work_item_id
                WHEN 'pentester' THEN NEW.pentester_work_item_id
                WHEN 'adviser' THEN NEW.adviser_work_item_id END
           )
      INTO roster_exact
      FROM stage_work_items item
     WHERE item.team_plan_id=NEW.stage_team_plan_id
       AND item.dispatch_epoch=NEW.resume_dispatch_epoch
       AND item.required_for_barrier=TRUE;
    IF plan.dispatch_epoch<>NEW.resume_dispatch_epoch
       OR plan.requests_closed_at IS NOT NULL
       OR NOT primary_exact OR NOT roster_exact
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_SCHEDULE_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_primary_schedule_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_primary_schedules
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_asset_primary_schedule();

-- Once an asset epoch is opened, the building receipt is the complete insert
-- authority.  After it is applied no fifth WorkItem can join that epoch.
CREATE FUNCTION enforce_investigation_asset_fixed_roster_work_item()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    schedule investigation_asset_primary_schedules%ROWTYPE;
    expected_role TEXT;
BEGIN
    SELECT * INTO schedule
      FROM investigation_asset_primary_schedules persisted
     WHERE persisted.stage_team_plan_id=NEW.team_plan_id
       AND persisted.resume_dispatch_epoch=NEW.dispatch_epoch
     FOR SHARE;
    IF NOT FOUND THEN RETURN NEW; END IF;
    IF schedule.status<>'building' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_FIXED_ROSTER_APPEND_ONLY';
    END IF;
    IF NEW.id=schedule.primary_work_item_id THEN
        IF NEW.kind<>'investigation_asset_primary'
           OR NEW.stable_key<>'asset:' || schedule.asset_lane_id::TEXT || ':primary:' ||
               schedule.evolution_epoch::TEXT
           OR NEW.required_for_barrier
           OR NEW.input_manifest_hash<>schedule.asset_context_sha256
           OR NEW.created_by<>'server_phase_transition'
        THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_WORK_ITEM_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;
    expected_role := CASE NEW.id
        WHEN schedule.browser_work_item_id THEN 'browser'
        WHEN schedule.researcher_work_item_id THEN 'researcher'
        WHEN schedule.pentester_work_item_id THEN 'pentester'
        WHEN schedule.adviser_work_item_id THEN 'adviser'
        ELSE NULL END;
    IF expected_role IS NULL
       OR NEW.kind<>'investigation_asset_role'
       OR NEW.role<>expected_role
       OR NEW.stable_key<>'asset:' || schedule.asset_lane_id::TEXT || ':role:' ||
           expected_role || ':' || schedule.evolution_epoch::TEXT
       OR NOT NEW.required_for_barrier
       OR NEW.input_manifest_hash<>schedule.asset_context_sha256
       OR NEW.created_by<>'server_phase_transition'
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PRIMARY_FIXED_ROSTER_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_fixed_roster_work_item_contract
BEFORE INSERT ON stage_work_items
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_asset_fixed_roster_work_item();

-- Preserve every existing StageTeam transition and add only one closed->open
-- epoch advance backed by a building Asset Primary schedule receipt.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
    target_intel_goal_resume_advance BOOLEAN := FALSE;
    investigation_task_rearm_advance BOOLEAN := FALSE;
    investigation_execution_primary_rearm_advance BOOLEAN := FALSE;
    investigation_evolution_analysis_rearm_advance BOOLEAN := FALSE;
    investigation_asset_primary_advance BOOLEAN := FALSE;
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
    IF ROW(NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.stage_kind,NEW.unit_generation,
        NEW.schema_version,NEW.plan_version,NEW.plan_hash,NEW.leader_role,
        NEW.aggregator_kind,NEW.aggregator_role,NEW.allowed_worker_roles,
        NEW.max_workers_total,NEW.max_workers_active,NEW.dynamic_requests_allowed,
        NEW.dynamic_request_policy,NEW.final_submitter_kind,
        NEW.created_from_stage_spec_hash,NEW.created_at)
       IS DISTINCT FROM
       ROW(OLD.id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,OLD.unit_generation,
        OLD.schema_version,OLD.plan_version,OLD.plan_hash,OLD.leader_role,
        OLD.aggregator_kind,OLD.aggregator_role,OLD.allowed_worker_roles,
        OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,OLD.created_at)
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE'; END IF;
    repair_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL AND EXISTS(
            SELECT 1 FROM stage_team_repair_generations generation
            JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
             WHERE generation.team_plan_id=OLD.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
               AND gap.source_dispatch_epoch=OLD.dispatch_epoch
               AND gap.source_aggregator_worker_run_id=OLD.final_submitter_worker_run_id);
    controller_turn_resume_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL AND EXISTS(
            SELECT 1 FROM stage_team_controller_turn_resumes authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.leader_worker_run_id=OLD.final_submitter_worker_run_id);
    target_intel_goal_resume_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL AND EXISTS(
            SELECT 1 FROM target_intel_goal_resume_authorities authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_goal_epoch=OLD.dispatch_epoch
               AND authority.successor_goal_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.controller_worker_run_id IS NOT DISTINCT FROM
                   OLD.final_submitter_worker_run_id);
    IF NOT target_intel_goal_resume_advance THEN
        target_intel_goal_resume_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
            AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
            AND OLD.final_submitter_worker_run_id IS NULL
            AND NEW.final_submitter_worker_run_id IS NULL AND EXISTS(
                SELECT 1 FROM target_intel_goal_resume_authorities authority
                 WHERE authority.team_plan_id=OLD.id AND authority.status='building'
                   AND authority.source_goal_epoch=OLD.dispatch_epoch
                   AND authority.successor_goal_epoch=NEW.dispatch_epoch
                   AND authority.source_plan_row_version=OLD.row_version);
    END IF;
    investigation_task_rearm_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS(SELECT 1 FROM investigation_task_primary_rearms authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    investigation_execution_primary_rearm_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS(SELECT 1 FROM investigation_verification_execution_primary_rearms authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    investigation_evolution_analysis_rearm_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS(SELECT 1 FROM investigation_evolution_analysis_primary_rearms authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    investigation_asset_primary_advance := NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS(SELECT 1 FROM investigation_asset_primary_schedules authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version);
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch
       AND NOT repair_advance AND NOT controller_turn_resume_advance
       AND NOT target_intel_goal_resume_advance AND NOT investigation_task_rearm_advance
       AND NOT investigation_execution_primary_rearm_advance
       AND NOT investigation_evolution_analysis_rearm_advance
       AND NOT investigation_asset_primary_advance
    THEN RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR'; END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED'; END IF;
    IF OLD.requests_closed_at IS NOT NULL
       AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
       AND NOT repair_advance AND NOT controller_turn_resume_advance
       AND NOT target_intel_goal_resume_advance AND NOT investigation_task_rearm_advance
       AND NOT investigation_execution_primary_rearm_advance
       AND NOT investigation_evolution_analysis_rearm_advance
       AND NOT investigation_asset_primary_advance
    THEN RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN'; END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
       AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
       AND NOT repair_advance AND NOT controller_turn_resume_advance
       AND NOT target_intel_goal_resume_advance AND NOT investigation_task_rearm_advance
       AND NOT investigation_execution_primary_rearm_advance
       AND NOT investigation_evolution_analysis_rearm_advance
       AND NOT investigation_asset_primary_advance
       AND NOT(NEW.final_submitter_worker_run_id IS NOT NULL AND EXISTS(
           SELECT 1 FROM stage_worker_runs previous_submitter
            WHERE previous_submitter.id=OLD.final_submitter_worker_run_id
              AND previous_submitter.status='superseded'))
    THEN RAISE EXCEPTION 'STAGE_TEAM_FINAL_SUBMITTER_IMMUTABLE'; END IF;
    IF NEW.requests_closed_at IS NOT DISTINCT FROM OLD.requests_closed_at
       AND NEW.dispatch_epoch IS NOT DISTINCT FROM OLD.dispatch_epoch
       AND NEW.final_submitter_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_NOOP_UPDATE_FORBIDDEN'; END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
