-- Asset-bound Investigation verification uses the installed Tool Manager
-- inventory rather than a second, hand-maintained capability catalog.  One
-- round is opened for one current hypothesis revision and reuses the durable
-- Asset Primary plus browser/researcher/pentester/adviser roster.  Invocations
-- are an append-only 0..N audit stream; only the independent Primary+Adviser
-- resolution is business-terminal authority.

-- Session-level authorization is independent from the fixed-action compiler.
-- It freezes effects and credential bindings for one exact asset/hypothesis,
-- but deliberately does not name a Tool Manager member.
CREATE TABLE investigation_asset_verification_authorizations (
    session_authorization_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL UNIQUE,
    allowed_effect_classes JSONB NOT NULL
        CHECK(stage_team_json_string_array_is_valid(allowed_effect_classes)),
    maximum_risk_tier TEXT NOT NULL CHECK(maximum_risk_tier IN('T0','T1','T2','T3')),
    allowed_credential_binding_sha256s JSONB NOT NULL
        CHECK(jsonb_typeof(allowed_credential_binding_sha256s)='array'),
    credential_binding_set_sha256 TEXT NOT NULL
        CHECK(credential_binding_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    decision TEXT NOT NULL CHECK(decision='authorized'),
    authorized_by UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    operator_channel TEXT NOT NULL CHECK(operator_channel IN('local_ui','local_cli','local_admin')),
    authorization_sha256 TEXT NOT NULL CHECK(authorization_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    expires_at TIMESTAMPTZ NOT NULL,
    authorized_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    FOREIGN KEY(target_live_id) REFERENCES targets(id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(task_id,operation_id,stage_execution_id,
                stage_run_unit_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_asset_verification_budget_envelopes (
    session_budget_envelope_id UUID PRIMARY KEY,
    session_authorization_id UUID NOT NULL UNIQUE REFERENCES
        investigation_asset_verification_authorizations(session_authorization_id)
        ON DELETE RESTRICT,
    maximum_invocations BIGINT NOT NULL CHECK(maximum_invocations>0),
    remaining_invocations BIGINT NOT NULL CHECK(remaining_invocations>=0),
    maximum_network_requests BIGINT NOT NULL CHECK(maximum_network_requests>=0),
    remaining_network_requests BIGINT NOT NULL CHECK(remaining_network_requests>=0),
    maximum_wall_time_ms BIGINT NOT NULL CHECK(maximum_wall_time_ms>0),
    remaining_wall_time_ms BIGINT NOT NULL CHECK(remaining_wall_time_ms>=0),
    maximum_output_bytes BIGINT NOT NULL CHECK(maximum_output_bytes>0),
    remaining_output_bytes BIGINT NOT NULL CHECK(remaining_output_bytes>=0),
    maximum_parallel_invocations INTEGER NOT NULL CHECK(maximum_parallel_invocations>0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    envelope_sha256 TEXT NOT NULL CHECK(envelope_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(remaining_invocations<=maximum_invocations),
    CHECK(remaining_network_requests<=maximum_network_requests),
    CHECK(remaining_wall_time_ms<=maximum_wall_time_ms),
    CHECK(remaining_output_bytes<=maximum_output_bytes)
);

CREATE FUNCTION investigation_guard_asset_verification_authorization()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE lane investigation_asset_lanes%ROWTYPE;
DECLARE revision attack_hypothesis_revisions%ROWTYPE;
DECLARE task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_AUTHORIZATION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    SELECT * INTO STRICT revision FROM attack_hypothesis_revisions
     WHERE revision_id=NEW.hypothesis_revision_id FOR SHARE;
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.verification_task_id FOR SHARE;
    IF NEW.expires_at<=statement_timestamp()
       OR NEW.operator_channel<>'local_cli'
       OR NOT EXISTS(SELECT 1 FROM operator_principals principal
                      WHERE principal.id=NEW.authorized_by
                        AND principal.principal_kind='local_operator'
                        AND principal.active)
       OR jsonb_array_length(NEW.allowed_effect_classes)=0
       OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(NEW.allowed_effect_classes) effect
                  WHERE effect NOT IN('read_only','passive_network','active_network',
                                      'credentialed_network','code_execution'))
       OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(
                    NEW.allowed_credential_binding_sha256s) credential
                  WHERE credential !~ '^sha256:[0-9a-f]{64}$')
       OR NEW.credential_binding_set_sha256<>tool_truth_sha256(
            COALESCE((SELECT jsonb_agg(value ORDER BY value)::TEXT
               FROM jsonb_array_elements_text(NEW.allowed_credential_binding_sha256s)),'[]'))
       OR lane.state<>'verifying'
       OR ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,
              lane.organization_id,lane.target_id)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,
              NEW.scope_snapshot_id,NEW.organization_id,NEW.target_live_id)
       OR revision.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR revision.target_live_id IS DISTINCT FROM NEW.target_live_id
       OR revision.lifecycle_state<>'current'
       OR revision.epistemic_state IN('verified','refuted','invalid')
       OR NOT EXISTS(SELECT 1 FROM attack_hypothesis_heads head
                      WHERE head.root_id=revision.root_id
                        AND head.head_revision_id=NEW.hypothesis_revision_id
                        AND head.head_lifecycle_state='current')
       OR task.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR task.hypothesis_revision_id<>NEW.hypothesis_revision_id
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_AUTHORIZATION_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_verification_authorizations_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_verification_authorizations
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_verification_authorization();

CREATE FUNCTION investigation_guard_asset_verification_budget()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='INSERT' THEN
        RETURN NEW;
    END IF;
    IF TG_OP='DELETE'
       OR (TG_OP='UPDATE' AND (
          (to_jsonb(NEW)-ARRAY['remaining_invocations','remaining_network_requests',
             'remaining_wall_time_ms','remaining_output_bytes','row_version']) IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['remaining_invocations','remaining_network_requests',
             'remaining_wall_time_ms','remaining_output_bytes','row_version'])
          OR NEW.row_version<>OLD.row_version+1
          OR NEW.remaining_invocations>=OLD.remaining_invocations
          OR NEW.remaining_network_requests>OLD.remaining_network_requests
          OR NEW.remaining_wall_time_ms>OLD.remaining_wall_time_ms
          OR NEW.remaining_output_bytes>OLD.remaining_output_bytes))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_BUDGET_CAS_CONFLICT'
            USING ERRCODE='40001';
    END IF;
    RETURN NEW;
END;
$$;
CREATE FUNCTION investigation_reject_asset_verification_append_only()
RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
    RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_APPEND_ONLY' USING ERRCODE='23514';
END; $$;

-- Runtime memory currently gives each message chain one immutable WorkerRun
-- owner. Verification-round workers therefore carry an append-only continuity
-- binding to the stable Asset Primary/role chain instead of mutating that
-- historical owner or creating one conversation per hypothesis.
CREATE TABLE investigation_asset_verification_chain_continuities (
    continuity_id UUID PRIMARY KEY,
    asset_primary_schedule_receipt_id UUID NOT NULL REFERENCES
        investigation_asset_primary_schedules(schedule_receipt_id) ON DELETE RESTRICT,
    hypothesis_revision_id UUID NOT NULL,
    role TEXT NOT NULL CHECK(role IN('primary','browser','researcher','pentester','adviser')),
    predecessor_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    predecessor_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    verification_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    verification_worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    durable_message_chain_id UUID NOT NULL REFERENCES message_chains(id) ON DELETE RESTRICT,
    continuity_sha256 TEXT NOT NULL CHECK(continuity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(asset_primary_schedule_receipt_id,hypothesis_revision_id,role),
    CHECK(verification_work_item_id<>predecessor_work_item_id),
    CHECK(predecessor_worker_run_id IS NULL OR verification_worker_run_id<>predecessor_worker_run_id)
);
-- A chain is a durable team identity, not a single-attempt identity. Reusing
-- it is permitted only after the predecessor is terminal; concurrently live
-- owners remain impossible.
DROP INDEX stage_worker_runs_chain_owner;
CREATE UNIQUE INDEX stage_worker_runs_one_live_chain_owner
    ON stage_worker_runs(message_chain_id)
    WHERE message_chain_id IS NOT NULL
      AND status IN('queued','running','waiting_background','recovery_required');
CREATE TRIGGER investigation_asset_verification_chain_continuities_immutable
BEFORE UPDATE OR DELETE ON investigation_asset_verification_chain_continuities
FOR EACH ROW EXECUTE FUNCTION investigation_reject_asset_verification_append_only();

-- A verification round is a new StageTeam request epoch. This server-owned
-- rearm is inserted before the plan CAS and applied only after all five fresh
-- WorkItems/WorkerRuns and the session have been materialized.
CREATE TABLE investigation_asset_verification_round_rearms (
    round_rearm_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL,
    asset_primary_schedule_receipt_id UUID NOT NULL REFERENCES
        investigation_asset_primary_schedules(schedule_receipt_id) ON DELETE RESTRICT,
    stage_team_plan_id UUID NOT NULL,
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL CHECK(resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK(source_plan_row_version>=0),
    rearm_sha256 TEXT NOT NULL CHECK(rearm_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK(status IN('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    UNIQUE(stage_team_plan_id,resume_dispatch_epoch),
    CHECK((status='building' AND applied_at IS NULL)
       OR (status='applied' AND applied_at IS NOT NULL)),
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(target_live_id) REFERENCES targets(id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_asset_verification_round_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE'
       OR (TG_OP='UPDATE' AND (
          (to_jsonb(NEW)-ARRAY['status','applied_at']) IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['status','applied_at'])
          OR OLD.status<>'building' OR NEW.status<>'applied'
          OR NEW.applied_at IS NULL))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_ROUND_REARM_IMMUTABLE'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_verification_round_rearms_guard
BEFORE UPDATE OR DELETE ON investigation_asset_verification_round_rearms
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_verification_round_rearm();

CREATE TABLE investigation_asset_verification_sessions (
    session_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL,
    asset_primary_schedule_receipt_id UUID NOT NULL,
    evolution_epoch INTEGER NOT NULL CHECK(evolution_epoch>=0),
    round_rearm_id UUID NOT NULL UNIQUE REFERENCES
        investigation_asset_verification_round_rearms(round_rearm_id) ON DELETE RESTRICT,
    stage_team_plan_id UUID NOT NULL,
    dispatch_epoch BIGINT NOT NULL CHECK(dispatch_epoch>=0),
    session_authorization_id UUID NOT NULL UNIQUE,
    session_authorization_sha256 TEXT NOT NULL
        CHECK(session_authorization_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    authorization_expires_at TIMESTAMPTZ NOT NULL,
    session_budget_envelope_id UUID NOT NULL UNIQUE,
    budget_envelope_sha256 TEXT NOT NULL
        CHECK(budget_envelope_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    primary_work_item_id UUID NOT NULL,
    primary_worker_run_id UUID NOT NULL,
    primary_message_chain_id UUID NOT NULL,
    browser_work_item_id UUID NOT NULL,
    browser_worker_run_id UUID NOT NULL,
    browser_message_chain_id UUID NOT NULL,
    researcher_work_item_id UUID NOT NULL,
    researcher_worker_run_id UUID NOT NULL,
    researcher_message_chain_id UUID NOT NULL,
    pentester_work_item_id UUID NOT NULL,
    pentester_worker_run_id UUID NOT NULL,
    pentester_message_chain_id UUID NOT NULL,
    adviser_work_item_id UUID NOT NULL,
    adviser_worker_run_id UUID NOT NULL,
    adviser_message_chain_id UUID NOT NULL,
    roster_set_sha256 TEXT NOT NULL CHECK(roster_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN('open','resolved')),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    resolution_authority_id UUID UNIQUE,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    resolved_at TIMESTAMPTZ,
    UNIQUE(asset_lane_id,hypothesis_revision_id),
    CHECK((state='open' AND resolution_authority_id IS NULL AND resolved_at IS NULL)
       OR (state='resolved' AND resolution_authority_id IS NOT NULL AND resolved_at IS NOT NULL)),
    CHECK(primary_worker_run_id<>browser_worker_run_id
      AND primary_worker_run_id<>researcher_worker_run_id
      AND primary_worker_run_id<>pentester_worker_run_id
      AND primary_worker_run_id<>adviser_worker_run_id
      AND browser_worker_run_id<>researcher_worker_run_id
      AND browser_worker_run_id<>pentester_worker_run_id
      AND browser_worker_run_id<>adviser_worker_run_id
      AND researcher_worker_run_id<>pentester_worker_run_id
      AND researcher_worker_run_id<>adviser_worker_run_id
      AND pentester_worker_run_id<>adviser_worker_run_id),
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    FOREIGN KEY(target_live_id) REFERENCES targets(id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(task_id,operation_id,stage_execution_id,
                stage_run_unit_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(asset_primary_schedule_receipt_id)
        REFERENCES investigation_asset_primary_schedules(schedule_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(session_authorization_id)
        REFERENCES investigation_asset_verification_authorizations(session_authorization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(session_budget_envelope_id)
        REFERENCES investigation_asset_verification_budget_envelopes(session_budget_envelope_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(primary_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(browser_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(researcher_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(pentester_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(adviser_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_asset_verification_roster_sha256(
    primary_worker UUID, browser_worker UUID, researcher_worker UUID,
    pentester_worker UUID, adviser_worker UUID
) RETURNS TEXT LANGUAGE SQL STABLE STRICT AS $$
    SELECT tool_truth_sha256(jsonb_build_array(
        jsonb_build_object('role','primary','worker_run_id',primary_worker),
        jsonb_build_object('role','browser','worker_run_id',browser_worker),
        jsonb_build_object('role','researcher','worker_run_id',researcher_worker),
        jsonb_build_object('role','pentester','worker_run_id',pentester_worker),
        jsonb_build_object('role','adviser','worker_run_id',adviser_worker)
    )::TEXT)
$$;

CREATE FUNCTION investigation_guard_asset_verification_session()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    lane investigation_asset_lanes%ROWTYPE;
    schedule investigation_asset_primary_schedules%ROWTYPE;
    revision attack_hypothesis_revisions%ROWTYPE;
    task hypothesis_verification_tasks%ROWTYPE;
    session_auth investigation_asset_verification_authorizations%ROWTYPE;
    session_budget investigation_asset_verification_budget_envelopes%ROWTYPE;
    rearm investigation_asset_verification_round_rearms%ROWTYPE;
    roster_exact BOOLEAN;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_SESSION_APPEND_ONLY' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(NEW)-ARRAY['state','head_version','resolution_authority_id','resolved_at'])
             IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['state','head_version','resolution_authority_id','resolved_at'])
           OR OLD.state<>'open' OR NEW.state<>'resolved'
           OR NEW.head_version<>OLD.head_version+1
           OR NEW.resolution_authority_id IS NULL OR NEW.resolved_at IS NULL
           OR NOT EXISTS(
                SELECT 1 FROM investigation_hypothesis_resolution_authorities resolution
                 WHERE resolution.resolution_authority_id=NEW.resolution_authority_id
                   AND resolution.session_id=NEW.session_id
                   AND resolution.expected_session_head_version=OLD.head_version)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_SESSION_CAS_CONFLICT'
                USING ERRCODE='40001';
        END IF;
        RETURN NEW;
    END IF;

    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    SELECT * INTO STRICT schedule FROM investigation_asset_primary_schedules
     WHERE schedule_receipt_id=NEW.asset_primary_schedule_receipt_id FOR SHARE;
    SELECT * INTO STRICT revision FROM attack_hypothesis_revisions
     WHERE revision_id=NEW.hypothesis_revision_id FOR SHARE;
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.verification_task_id FOR SHARE;
    SELECT * INTO STRICT session_auth FROM investigation_asset_verification_authorizations
     WHERE session_authorization_id=NEW.session_authorization_id FOR SHARE;
    SELECT * INTO STRICT session_budget FROM investigation_asset_verification_budget_envelopes
     WHERE session_budget_envelope_id=NEW.session_budget_envelope_id FOR SHARE;
    SELECT * INTO STRICT rearm FROM investigation_asset_verification_round_rearms
     WHERE round_rearm_id=NEW.round_rearm_id FOR SHARE;

    SELECT COUNT(*)=5 AND BOOL_AND(
        worker.work_item_id=expected.work_item_id
        AND item.team_plan_id=schedule.stage_team_plan_id
        AND item.dispatch_epoch=NEW.dispatch_epoch
        AND chain.id=expected.message_chain_id
        AND chain.task_id=NEW.operation_id
        AND EXISTS(
             SELECT 1 FROM investigation_asset_verification_chain_continuities continuity
              WHERE continuity.asset_primary_schedule_receipt_id=
                    NEW.asset_primary_schedule_receipt_id
                AND continuity.hypothesis_revision_id=NEW.hypothesis_revision_id
                AND continuity.role=expected.role
                AND continuity.verification_work_item_id=expected.work_item_id
                AND continuity.verification_worker_run_id=expected.worker_run_id
                AND continuity.durable_message_chain_id=expected.message_chain_id)
        AND ((expected.role='primary' AND item.role=(
                SELECT leader_role FROM stage_team_plans WHERE id=schedule.stage_team_plan_id))
             OR (expected.role<>'primary' AND item.role=expected.role)))
      INTO roster_exact
      FROM (VALUES
          ('primary',NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id),
          ('browser',NEW.browser_work_item_id,NEW.browser_worker_run_id,NEW.browser_message_chain_id),
          ('researcher',NEW.researcher_work_item_id,NEW.researcher_worker_run_id,NEW.researcher_message_chain_id),
          ('pentester',NEW.pentester_work_item_id,NEW.pentester_worker_run_id,NEW.pentester_message_chain_id),
          ('adviser',NEW.adviser_work_item_id,NEW.adviser_worker_run_id,NEW.adviser_message_chain_id)
      ) expected(role,work_item_id,worker_run_id,message_chain_id)
      JOIN stage_worker_runs worker ON worker.id=expected.worker_run_id
      JOIN stage_work_items item ON item.id=expected.work_item_id
      JOIN message_chains chain ON chain.id=expected.message_chain_id;

    IF NEW.state<>'open' OR NEW.head_version<>0
       OR NEW.resolution_authority_id IS NOT NULL OR NEW.resolved_at IS NOT NULL
       OR lane.state<>'verifying'
       OR ROW(lane.target_id,lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,
              lane.organization_id,lane.evolution_epoch)
          IS DISTINCT FROM ROW(NEW.target_live_id,NEW.operation_id,NEW.stage_execution_id,
                               NEW.scope_snapshot_id,NEW.organization_id,NEW.evolution_epoch)
       OR schedule.status<>'applied' OR schedule.asset_lane_id<>NEW.asset_lane_id
       OR schedule.target_id<>NEW.target_live_id OR schedule.evolution_epoch<>NEW.evolution_epoch
       OR NEW.primary_work_item_id=schedule.primary_work_item_id
       OR NEW.primary_worker_run_id=schedule.primary_worker_run_id
       OR NEW.browser_work_item_id=schedule.browser_work_item_id
       OR NEW.researcher_work_item_id=schedule.researcher_work_item_id
       OR NEW.pentester_work_item_id=schedule.pentester_work_item_id
       OR NEW.adviser_work_item_id=schedule.adviser_work_item_id
       OR NEW.primary_message_chain_id<>schedule.primary_message_chain_id
       OR ROW(rearm.session_id,rearm.operation_id,rearm.stage_execution_id,
              rearm.stage_run_unit_id,rearm.scope_snapshot_id,rearm.organization_id,
              rearm.asset_lane_id,rearm.target_live_id,rearm.hypothesis_revision_id,
              rearm.verification_task_id,rearm.asset_primary_schedule_receipt_id,
              rearm.stage_team_plan_id,rearm.resume_dispatch_epoch,rearm.status)
          IS DISTINCT FROM ROW(NEW.session_id,NEW.operation_id,NEW.stage_execution_id,
              NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
              NEW.asset_lane_id,NEW.target_live_id,NEW.hypothesis_revision_id,
              NEW.verification_task_id,NEW.asset_primary_schedule_receipt_id,
              NEW.stage_team_plan_id,NEW.dispatch_epoch,'building')
       OR NOT EXISTS(
            SELECT 1 FROM stage_worker_runs predecessor
             WHERE predecessor.id=schedule.primary_worker_run_id
               AND predecessor.message_chain_id=NEW.primary_message_chain_id
               AND predecessor.status IN('passed','failed','exhausted','superseded'))
       OR revision.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR revision.target_live_id IS DISTINCT FROM NEW.target_live_id
       OR revision.lifecycle_state<>'current'
       OR revision.epistemic_state IN('verified','refuted','invalid')
       OR NOT EXISTS(SELECT 1 FROM attack_hypothesis_heads head
                      WHERE head.root_id=revision.root_id
                        AND head.head_revision_id=NEW.hypothesis_revision_id
                        AND head.head_lifecycle_state='current')
       OR task.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR task.hypothesis_revision_id<>NEW.hypothesis_revision_id
       OR ROW(session_auth.operation_id,session_auth.project_scope_id,
              session_auth.stage_execution_id,session_auth.stage_run_unit_id,
              session_auth.scope_snapshot_id,session_auth.organization_id,
              session_auth.asset_lane_id,session_auth.target_live_id,
              session_auth.hypothesis_revision_id,session_auth.verification_task_id)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.asset_lane_id,NEW.target_live_id,
              NEW.hypothesis_revision_id,NEW.verification_task_id)
       OR session_auth.authorization_sha256<>NEW.session_authorization_sha256
       OR session_auth.expires_at<>NEW.authorization_expires_at
       OR session_auth.expires_at<=statement_timestamp()
       OR session_budget.session_authorization_id<>NEW.session_authorization_id
       OR session_budget.envelope_sha256<>NEW.budget_envelope_sha256
       OR session_budget.remaining_invocations<=0
       OR NOT roster_exact
       OR EXISTS(
            SELECT 1 FROM (VALUES
                (NEW.primary_work_item_id,NEW.primary_worker_run_id),
                (NEW.browser_work_item_id,NEW.browser_worker_run_id),
                (NEW.researcher_work_item_id,NEW.researcher_worker_run_id),
                (NEW.pentester_work_item_id,NEW.pentester_worker_run_id),
                (NEW.adviser_work_item_id,NEW.adviser_worker_run_id)
            ) actor(work_item_id,worker_run_id)
            JOIN stage_work_items item ON item.id=actor.work_item_id
            JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id
             WHERE item.kind<>'investigation_asset_verification_round'
                OR item.input_refs<>jsonb_build_array(jsonb_build_object(
                    'kind','investigation_asset_verification_round',
                    'asset_lane_id',NEW.asset_lane_id,'target_id',NEW.target_live_id,
                    'hypothesis_revision_id',NEW.hypothesis_revision_id,
                    'evolution_epoch',NEW.evolution_epoch))
                OR item.required_for_barrier<>(CASE WHEN item.id=NEW.primary_work_item_id
                                                    THEN FALSE ELSE TRUE END)
                OR item.created_by<>'server_seed'
                OR item.output_schema<>(CASE
                     WHEN item.id=NEW.adviser_work_item_id
                       THEN 'investigation_asset_verification_adviser_review.v1'
                     WHEN item.id=NEW.primary_work_item_id
                       THEN 'investigation_asset_verification_primary_resolution.v1'
                     ELSE 'investigation_asset_verification_actor_observation.v1' END)
                OR worker.status NOT IN('queued','running','waiting_background'))
       OR NEW.roster_set_sha256<>investigation_asset_verification_roster_sha256(
            NEW.primary_worker_run_id,NEW.browser_worker_run_id,NEW.researcher_worker_run_id,
            NEW.pentester_worker_run_id,NEW.adviser_worker_run_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_SESSION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

-- Resolution table is declared before the session trigger is installed so the
-- trigger function may resolve the relation when a migration runs in one batch.
CREATE TABLE investigation_dynamic_tool_inventory_snapshots (
    inventory_snapshot_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL REFERENCES investigation_asset_verification_sessions(session_id)
        ON DELETE RESTRICT,
    inventory_source_sha256 TEXT NOT NULL CHECK(inventory_source_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK(member_count>=0),
    member_set_sha256 TEXT NOT NULL CHECK(member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(session_id,inventory_source_sha256)
);

CREATE FUNCTION investigation_guard_dynamic_tool_inventory_snapshot()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM investigation_asset_verification_sessions session_row
                   WHERE session_row.session_id=NEW.session_id
                     AND session_row.state='open'
                     AND session_row.authorization_expires_at>statement_timestamp()
                   FOR SHARE)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TOOL_INVENTORY_SESSION_NOT_OPEN'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_tool_inventory_snapshots_guard
BEFORE INSERT ON investigation_dynamic_tool_inventory_snapshots
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_tool_inventory_snapshot();

CREATE TABLE investigation_dynamic_tool_inventory_members (
    inventory_member_id UUID PRIMARY KEY,
    inventory_snapshot_id UUID NOT NULL REFERENCES investigation_dynamic_tool_inventory_snapshots(
        inventory_snapshot_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK(member_ordinal>=0),
    tool_id TEXT NOT NULL CHECK(BTRIM(tool_id)<>''),
    tool_name TEXT NOT NULL CHECK(BTRIM(tool_name)<>''),
    installed BOOLEAN NOT NULL CHECK(installed),
    environment_ready BOOLEAN NOT NULL CHECK(environment_ready),
    config_sha256 TEXT NOT NULL CHECK(config_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    executable_identity_sha256 TEXT NOT NULL
        CHECK(executable_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    runtime TEXT NOT NULL CHECK(BTRIM(runtime)<>''),
    runtime_version TEXT NOT NULL CHECK(BTRIM(runtime_version)<>''),
    launch_mode TEXT NOT NULL CHECK(BTRIM(launch_mode)<>''),
    parameter_schema JSONB NOT NULL CHECK(jsonb_typeof(parameter_schema) IN('array','object')),
    output_schema JSONB NOT NULL CHECK(jsonb_typeof(output_schema) IN('array','object')),
    tags JSONB NOT NULL CHECK(stage_team_json_string_array_is_valid(tags)),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(inventory_snapshot_id,member_ordinal),
    UNIQUE(inventory_snapshot_id,tool_name),
    UNIQUE(inventory_snapshot_id,member_sha256)
);

CREATE FUNCTION investigation_guard_dynamic_tool_inventory_member()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.member_sha256<>tool_truth_sha256(jsonb_build_object(
        'tool_id',NEW.tool_id,'tool_name',NEW.tool_name,
        'config_sha256',NEW.config_sha256,
        'executable_identity_sha256',NEW.executable_identity_sha256,
        'runtime',NEW.runtime,'runtime_version',NEW.runtime_version,
        'launch_mode',NEW.launch_mode,'parameter_schema',NEW.parameter_schema,
        'output_schema',NEW.output_schema,'tags',NEW.tags)::TEXT)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TOOL_INVENTORY_MEMBER_HASH_DRIFT'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_tool_inventory_members_guard
BEFORE INSERT ON investigation_dynamic_tool_inventory_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_tool_inventory_member();

CREATE FUNCTION investigation_validate_dynamic_tool_inventory()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_snapshot_id UUID := COALESCE(NEW.inventory_snapshot_id,OLD.inventory_snapshot_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_dynamic_tool_inventory_snapshots snapshot
         WHERE snapshot.inventory_snapshot_id=requested_snapshot_id
           AND ROW(snapshot.member_count,snapshot.member_set_sha256)
               IS DISTINCT FROM ROW(
                   (SELECT COUNT(*) FROM investigation_dynamic_tool_inventory_members member
                     WHERE member.inventory_snapshot_id=requested_snapshot_id),
                   tool_truth_sha256(COALESCE((
                       SELECT jsonb_agg(member.member_sha256 ORDER BY member.member_ordinal)::TEXT
                         FROM investigation_dynamic_tool_inventory_members member
                        WHERE member.inventory_snapshot_id=requested_snapshot_id),'[]')))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TOOL_INVENTORY_CENSUS_DRIFT'
            USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_dynamic_tool_inventory_census_exact
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_tool_inventory_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_tool_inventory();

CREATE FUNCTION investigation_reject_dynamic_tool_inventory_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
    RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TOOL_INVENTORY_APPEND_ONLY' USING ERRCODE='23514';
END; $$;
CREATE TRIGGER investigation_dynamic_tool_inventory_snapshots_immutable
BEFORE UPDATE OR DELETE ON investigation_dynamic_tool_inventory_snapshots
FOR EACH ROW EXECUTE FUNCTION investigation_reject_dynamic_tool_inventory_mutation();
CREATE TRIGGER investigation_dynamic_tool_inventory_members_immutable
BEFORE UPDATE OR DELETE ON investigation_dynamic_tool_inventory_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_dynamic_tool_inventory_mutation();

CREATE TABLE investigation_asset_verification_invocations (
    invocation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL REFERENCES investigation_asset_verification_sessions(session_id)
        ON DELETE RESTRICT,
    invocation_ordinal BIGINT NOT NULL CHECK(invocation_ordinal>0),
    actor_role TEXT NOT NULL CHECK(actor_role IN('primary','browser','researcher','pentester','adviser')),
    actor_work_item_id UUID NOT NULL,
    actor_worker_run_id UUID NOT NULL,
    actor_message_chain_id UUID NOT NULL,
    inventory_snapshot_id UUID NOT NULL REFERENCES investigation_dynamic_tool_inventory_snapshots(
        inventory_snapshot_id) ON DELETE RESTRICT,
    inventory_member_id UUID REFERENCES investigation_dynamic_tool_inventory_members(
        inventory_member_id) ON DELETE RESTRICT,
    wrapper_name TEXT NOT NULL CHECK(wrapper_name IN(
        'pentest_list_tools','pentest_read_skill','pentest_run','browser_collect_js_api')),
    selected_tool_name TEXT,
    selected_tool_config_sha256 TEXT
        CHECK(selected_tool_config_sha256 IS NULL OR selected_tool_config_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    invocation_authorization_id UUID NOT NULL UNIQUE,
    invocation_authorization_sha256 TEXT NOT NULL
        CHECK(invocation_authorization_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    invocation_authorization_expires_at TIMESTAMPTZ NOT NULL,
    effect_class TEXT NOT NULL CHECK(effect_class IN(
        'read_only','passive_network','active_network','credentialed_network','code_execution')),
    risk_tier TEXT NOT NULL CHECK(risk_tier IN('T0','T1','T2','T3')),
    credential_binding_sha256 TEXT
        CHECK(credential_binding_sha256 IS NULL OR credential_binding_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    network_request_limit BIGINT NOT NULL CHECK(network_request_limit>=0),
    wall_time_limit_ms BIGINT NOT NULL CHECK(wall_time_limit_ms>0),
    output_byte_limit BIGINT NOT NULL CHECK(output_byte_limit>0),
    model_args_redacted JSONB NOT NULL CHECK(jsonb_typeof(model_args_redacted)='object'),
    model_args_sha256 TEXT NOT NULL CHECK(model_args_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    request_manifest_sha256 TEXT NOT NULL CHECK(request_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    started_lease_token UUID NOT NULL,
    started_attempt_epoch BIGINT NOT NULL CHECK(started_attempt_epoch>=0),
    started_checkpoint_version BIGINT NOT NULL CHECK(started_checkpoint_version>=0),
    state TEXT NOT NULL CHECK(state IN('running','succeeded','failed','outcome_unknown')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    capability_execution_receipt_id UUID REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    oracle_receipt_id UUID REFERENCES verification_oracle_assessments(oracle_assessment_id) ON DELETE RESTRICT,
    audit_evidence_ids BIGINT[] NOT NULL DEFAULT '{}'::BIGINT[],
    evidence_set_sha256 TEXT CHECK(evidence_set_sha256 IS NULL OR evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    redacted_result JSONB CHECK(redacted_result IS NULL OR jsonb_typeof(redacted_result)='object'),
    result_sha256 TEXT CHECK(result_sha256 IS NULL OR result_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    started_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    completed_at TIMESTAMPTZ,
    UNIQUE(session_id,invocation_ordinal),
    CHECK((wrapper_name IN('pentest_run','pentest_read_skill'))=
          (inventory_member_id IS NOT NULL AND selected_tool_name IS NOT NULL
           AND selected_tool_config_sha256 IS NOT NULL)),
    CHECK((state='running' AND completed_at IS NULL AND evidence_set_sha256 IS NULL
           AND redacted_result IS NULL AND result_sha256 IS NULL)
       OR (state<>'running' AND completed_at IS NOT NULL AND evidence_set_sha256 IS NOT NULL
           AND redacted_result IS NOT NULL AND result_sha256 IS NOT NULL)),
    CHECK(0<ALL(audit_evidence_ids)),
    FOREIGN KEY(actor_worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX investigation_asset_verification_one_running_per_actor
    ON investigation_asset_verification_invocations(session_id,actor_role)
    WHERE state='running';

CREATE FUNCTION investigation_guard_asset_verification_invocation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE session_row investigation_asset_verification_sessions%ROWTYPE;
DECLARE worker stage_worker_runs%ROWTYPE;
DECLARE member investigation_dynamic_tool_inventory_members%ROWTYPE;
DECLARE session_auth investigation_asset_verification_authorizations%ROWTYPE;
DECLARE session_budget investigation_asset_verification_budget_envelopes%ROWTYPE;
DECLARE expected_item UUID; DECLARE expected_worker UUID; DECLARE expected_chain UUID;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        SELECT * INTO STRICT worker FROM stage_worker_runs
         WHERE id=OLD.actor_worker_run_id FOR SHARE;
        IF (to_jsonb(NEW)-ARRAY['state','row_version','capability_execution_receipt_id',
             'oracle_receipt_id','audit_evidence_ids','evidence_set_sha256','redacted_result',
             'result_sha256','completed_at']) IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['state','row_version','capability_execution_receipt_id',
             'oracle_receipt_id','audit_evidence_ids','evidence_set_sha256','redacted_result',
             'result_sha256','completed_at'])
           OR OLD.state<>'running' OR NEW.state='running' OR NEW.row_version<>OLD.row_version+1
           OR worker.lease_token<>OLD.started_lease_token
           OR worker.attempt_epoch<>OLD.started_attempt_epoch
           OR worker.checkpoint_version<>OLD.started_checkpoint_version
           OR worker.status NOT IN('running','waiting_background')
           OR worker.lease_expires_at IS NULL
           OR worker.lease_expires_at<=statement_timestamp()
        THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_CAS_CONFLICT'
                USING ERRCODE='40001';
        END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT session_row FROM investigation_asset_verification_sessions
     WHERE session_id=NEW.session_id FOR SHARE;
    SELECT * INTO STRICT session_auth FROM investigation_asset_verification_authorizations
     WHERE session_authorization_id=session_row.session_authorization_id FOR SHARE;
    SELECT * INTO STRICT session_budget FROM investigation_asset_verification_budget_envelopes
     WHERE session_budget_envelope_id=session_row.session_budget_envelope_id FOR UPDATE;
    SELECT * INTO STRICT worker FROM stage_worker_runs
     WHERE id=NEW.actor_worker_run_id FOR SHARE;
    expected_item := CASE NEW.actor_role WHEN 'primary' THEN session_row.primary_work_item_id
        WHEN 'browser' THEN session_row.browser_work_item_id
        WHEN 'researcher' THEN session_row.researcher_work_item_id
        WHEN 'pentester' THEN session_row.pentester_work_item_id
        WHEN 'adviser' THEN session_row.adviser_work_item_id END;
    expected_worker := CASE NEW.actor_role WHEN 'primary' THEN session_row.primary_worker_run_id
        WHEN 'browser' THEN session_row.browser_worker_run_id
        WHEN 'researcher' THEN session_row.researcher_worker_run_id
        WHEN 'pentester' THEN session_row.pentester_worker_run_id
        WHEN 'adviser' THEN session_row.adviser_worker_run_id END;
    expected_chain := CASE NEW.actor_role WHEN 'primary' THEN session_row.primary_message_chain_id
        WHEN 'browser' THEN session_row.browser_message_chain_id
        WHEN 'researcher' THEN session_row.researcher_message_chain_id
        WHEN 'pentester' THEN session_row.pentester_message_chain_id
        WHEN 'adviser' THEN session_row.adviser_message_chain_id END;
    IF NEW.inventory_member_id IS NOT NULL THEN
        SELECT * INTO STRICT member FROM investigation_dynamic_tool_inventory_members
         WHERE inventory_member_id=NEW.inventory_member_id FOR SHARE;
    END IF;
    IF session_row.state<>'open' OR session_row.authorization_expires_at<=statement_timestamp()
       OR NEW.invocation_authorization_expires_at<=statement_timestamp()
       OR NEW.invocation_authorization_expires_at>session_row.authorization_expires_at
       OR NEW.invocation_authorization_id<>uuid_generate_v5(
            NEW.session_id,'investigation-asset-verification-invocation-authorization-v1:' ||
                NEW.invocation_id::TEXT)
       OR NEW.invocation_authorization_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_asset_verification_invocation_authorization.v1',
            'invocation_id',NEW.invocation_id,'session_id',NEW.session_id,
            'inventory_snapshot_id',NEW.inventory_snapshot_id,
            'inventory_member_id',NEW.inventory_member_id,'wrapper_name',NEW.wrapper_name,
            'selected_tool_name',NEW.selected_tool_name,
            'selected_tool_config_sha256',NEW.selected_tool_config_sha256,
            'effect_class',NEW.effect_class,'risk_tier',NEW.risk_tier,
            'credential_binding_sha256',NEW.credential_binding_sha256,
            'network_request_limit',NEW.network_request_limit,
            'wall_time_limit_ms',NEW.wall_time_limit_ms,
            'output_byte_limit',NEW.output_byte_limit,
            'model_args_sha256',NEW.model_args_sha256,
            'request_manifest_sha256',NEW.request_manifest_sha256,
            'expires_at',NEW.invocation_authorization_expires_at)::TEXT)
       OR NOT (session_auth.allowed_effect_classes ? NEW.effect_class)
       OR array_position(ARRAY['T0','T1','T2','T3'],NEW.risk_tier)>
          array_position(ARRAY['T0','T1','T2','T3'],session_auth.maximum_risk_tier)
       OR (NEW.credential_binding_sha256 IS NOT NULL
           AND NOT (session_auth.allowed_credential_binding_sha256s ?
                    NEW.credential_binding_sha256))
       OR session_budget.remaining_invocations<=0
       OR session_budget.remaining_network_requests<NEW.network_request_limit
       OR session_budget.remaining_wall_time_ms<NEW.wall_time_limit_ms
       OR session_budget.remaining_output_bytes<NEW.output_byte_limit
       OR (SELECT COUNT(*) FROM investigation_asset_verification_invocations current
            WHERE current.session_id=NEW.session_id AND current.state='running')>=
          session_budget.maximum_parallel_invocations
       OR NEW.state<>'running' OR NEW.row_version<>0 OR NEW.completed_at IS NOT NULL
       OR NEW.actor_work_item_id<>expected_item OR NEW.actor_worker_run_id<>expected_worker
       OR NEW.actor_message_chain_id<>expected_chain
       OR worker.work_item_id<>expected_item
       OR NOT EXISTS(SELECT 1 FROM investigation_asset_verification_chain_continuities continuity
                      WHERE continuity.asset_primary_schedule_receipt_id=
                            session_row.asset_primary_schedule_receipt_id
                        AND continuity.hypothesis_revision_id=session_row.hypothesis_revision_id
                        AND continuity.role=NEW.actor_role
                        AND continuity.verification_work_item_id=expected_item
                        AND continuity.verification_worker_run_id=NEW.actor_worker_run_id
                        AND continuity.durable_message_chain_id=expected_chain)
       OR worker.status NOT IN('running','waiting_background')
       OR worker.lease_token<>NEW.started_lease_token
       OR worker.attempt_epoch<>NEW.started_attempt_epoch
       OR worker.checkpoint_version<>NEW.started_checkpoint_version
       OR worker.lease_expires_at IS NULL OR worker.lease_expires_at<=statement_timestamp()
       OR NOT EXISTS(SELECT 1 FROM investigation_dynamic_tool_inventory_snapshots snapshot
                      WHERE snapshot.inventory_snapshot_id=NEW.inventory_snapshot_id
                        AND snapshot.session_id=NEW.session_id)
       OR (NEW.inventory_member_id IS NOT NULL AND (
            member.inventory_snapshot_id<>NEW.inventory_snapshot_id
            OR member.tool_name<>NEW.selected_tool_name
            OR member.config_sha256<>NEW.selected_tool_config_sha256))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    UPDATE investigation_asset_verification_budget_envelopes
       SET remaining_invocations=remaining_invocations-1,
           remaining_network_requests=remaining_network_requests-NEW.network_request_limit,
           remaining_wall_time_ms=remaining_wall_time_ms-NEW.wall_time_limit_ms,
           remaining_output_bytes=remaining_output_bytes-NEW.output_byte_limit,
           row_version=row_version+1
     WHERE session_budget_envelope_id=session_row.session_budget_envelope_id;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_verification_invocations_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_verification_invocations
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_verification_invocation();

CREATE TABLE investigation_hypothesis_resolution_authorities (
    resolution_authority_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL UNIQUE REFERENCES investigation_asset_verification_sessions(session_id)
        ON DELETE RESTRICT,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    expected_session_head_version BIGINT NOT NULL CHECK(expected_session_head_version>=0),
    primary_work_item_id UUID NOT NULL,
    primary_worker_run_id UUID NOT NULL,
    primary_message_chain_id UUID NOT NULL,
    primary_lease_token UUID NOT NULL,
    primary_attempt_epoch BIGINT NOT NULL CHECK(primary_attempt_epoch>=0),
    primary_checkpoint_version BIGINT NOT NULL CHECK(primary_checkpoint_version>=0),
    adviser_work_item_id UUID NOT NULL,
    adviser_worker_run_id UUID NOT NULL,
    adviser_message_chain_id UUID NOT NULL,
    adviser_review_output_id UUID NOT NULL UNIQUE REFERENCES stage_worker_outputs(id)
        ON DELETE RESTRICT,
    adviser_review_output_sha256 TEXT NOT NULL
        CHECK(adviser_review_output_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK(disposition IN('verified','refuted','invalid')),
    primary_conclusion_sha256 TEXT NOT NULL CHECK(primary_conclusion_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    adviser_concurrence_sha256 TEXT NOT NULL CHECK(adviser_concurrence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    conclusion_redacted JSONB NOT NULL CHECK(jsonb_typeof(conclusion_redacted)='object'),
    citation_count BIGINT NOT NULL CHECK(citation_count>=0),
    citation_set_sha256 TEXT NOT NULL CHECK(citation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    resolution_sha256 TEXT NOT NULL CHECK(resolution_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    resolved_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_hypothesis_resolution_citations (
    citation_id UUID PRIMARY KEY,
    resolution_authority_id UUID NOT NULL,
    citation_ordinal INTEGER NOT NULL CHECK(citation_ordinal>=0),
    citation_kind TEXT NOT NULL CHECK(citation_kind IN(
        'audit_evidence','capability_receipt','oracle_receipt','tool_invocation','other_authority')),
    audit_evidence_id BIGINT REFERENCES audit_log(id) ON DELETE RESTRICT,
    authority_id UUID,
    citation_sha256 TEXT NOT NULL CHECK(citation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(resolution_authority_id,citation_ordinal),
    CHECK((audit_evidence_id IS NOT NULL)::INTEGER+(authority_id IS NOT NULL)::INTEGER=1),
    CHECK(audit_evidence_id IS NULL OR audit_evidence_id>0)
);

-- Every non-empty citation is an audit projection of this exact verification
-- session. Tool count is not a business gate, but a citation may not borrow
-- evidence or receipts from another asset, hypothesis, or session.
CREATE FUNCTION investigation_guard_hypothesis_resolution_citation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE resolution investigation_hypothesis_resolution_authorities%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_RESOLUTION_CITATION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT resolution
      FROM investigation_hypothesis_resolution_authorities
     WHERE resolution_authority_id=NEW.resolution_authority_id FOR SHARE;
    IF (NEW.audit_evidence_id IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_verification_invocations invocation
             WHERE invocation.session_id=resolution.session_id
               AND invocation.state='succeeded'
               AND NEW.audit_evidence_id=ANY(invocation.audit_evidence_ids)))
       OR (NEW.authority_id IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_verification_invocations invocation
             WHERE invocation.session_id=resolution.session_id
               AND invocation.state='succeeded'
               AND NEW.authority_id IN(
                    invocation.invocation_id,
                    invocation.capability_execution_receipt_id,
                    invocation.oracle_receipt_id)))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_RESOLUTION_CITATION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

ALTER TABLE investigation_hypothesis_resolution_citations
    ADD CONSTRAINT investigation_hypothesis_resolution_citation_authority_fk
    FOREIGN KEY(resolution_authority_id)
        REFERENCES investigation_hypothesis_resolution_authorities(resolution_authority_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE investigation_pending_hypothesis_discoveries (
    discovery_authority_id UUID PRIMARY KEY,
    resolution_authority_id UUID NOT NULL,
    session_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    source_hypothesis_revision_id UUID NOT NULL,
    discovery_ordinal INTEGER NOT NULL CHECK(discovery_ordinal>=0),
    subject_kind TEXT NOT NULL CHECK(BTRIM(subject_kind)<>''),
    subject_identity_sha256 TEXT NOT NULL
        CHECK(subject_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    semantic_key_sha256 TEXT NOT NULL CHECK(semantic_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    semantic_key_canonical_json TEXT NOT NULL
        CHECK(jsonb_typeof(semantic_key_canonical_json::JSONB)='object'),
    canonical_proposal JSONB NOT NULL CHECK(jsonb_typeof(canonical_proposal)='object'),
    structured_claim TEXT NOT NULL CHECK(BTRIM(structured_claim)<>''),
    structured_claim_sha256 TEXT NOT NULL
        CHECK(structured_claim_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    rationale_redacted JSONB NOT NULL CHECK(jsonb_typeof(rationale_redacted)='object'),
    discovery_sha256 TEXT NOT NULL CHECK(discovery_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(resolution_authority_id,discovery_ordinal),
    UNIQUE(asset_lane_id,semantic_key_sha256,structured_claim_sha256),
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    FOREIGN KEY(target_live_id) REFERENCES targets(id) ON DELETE RESTRICT,
    FOREIGN KEY(source_hypothesis_revision_id)
        REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT
);

ALTER TABLE investigation_pending_hypothesis_discoveries
    ADD CONSTRAINT investigation_pending_hypothesis_discovery_resolution_fk
    FOREIGN KEY(resolution_authority_id)
        REFERENCES investigation_hypothesis_resolution_authorities(resolution_authority_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION investigation_guard_pending_hypothesis_discovery()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE resolution investigation_hypothesis_resolution_authorities%ROWTYPE;
        lane investigation_asset_lanes%ROWTYPE;
        expected_subject_identity_sha256 TEXT;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT resolution FROM investigation_hypothesis_resolution_authorities
     WHERE resolution_authority_id=NEW.resolution_authority_id FOR SHARE;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    expected_subject_identity_sha256:=tool_truth_sha256(jsonb_build_object(
        'domain','investigation_subject_identity.v1','subject_kind','asset',
        'subject_id',lane.target_id,'display_value',lane.target_value_at_freeze)::TEXT);
    IF ROW(NEW.session_id,NEW.asset_lane_id,NEW.target_live_id,
           NEW.source_hypothesis_revision_id)
       IS DISTINCT FROM ROW(resolution.session_id,resolution.asset_lane_id,
           resolution.target_live_id,resolution.hypothesis_revision_id)
       OR NEW.subject_kind<>'asset'
       OR NEW.subject_identity_sha256<>expected_subject_identity_sha256
       OR NEW.semantic_key_sha256<>
          ('sha256:'||encode(digest(
             convert_to('hypothesis_semantic_key.v1','UTF8')||decode('00','hex')||
             convert_to(NEW.semantic_key_canonical_json,'UTF8'),'sha256'),'hex'))
       OR NEW.semantic_key_canonical_json::JSONB #>> '{subject,kind}'<>'asset'
       OR NEW.semantic_key_canonical_json::JSONB #>> '{subject,identity_hash}'<>
          expected_subject_identity_sha256
       OR NEW.semantic_key_canonical_json::JSONB->>'organization_id'<>
          lane.organization_id::TEXT
       OR NEW.structured_claim_sha256<>tool_truth_sha256(to_jsonb(NEW.structured_claim)::TEXT)
       OR NEW.canonical_proposal->>'proposal_id'<>NEW.discovery_authority_id::TEXT
       OR NEW.canonical_proposal->>'subject_kind'<>'asset'
       OR NEW.canonical_proposal->>'subject_identity_hash'<>expected_subject_identity_sha256
       OR NEW.canonical_proposal->>'structured_claim'<>NEW.structured_claim
       OR NEW.canonical_proposal->'proof_refs'<>'[]'::JSONB
       OR NEW.canonical_proposal->'knowledge_signals'<>'[]'::JSONB
       OR NEW.canonical_proposal->>'readiness'<>'ready_for_strategy'
       OR NEW.discovery_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_pending_hypothesis_discovery.v1',
            'resolution_authority_id',NEW.resolution_authority_id,
            'asset_lane_id',NEW.asset_lane_id,'target_live_id',NEW.target_live_id,
            'source_hypothesis_revision_id',NEW.source_hypothesis_revision_id,
            'discovery_ordinal',NEW.discovery_ordinal,'subject_kind',NEW.subject_kind,
            'subject_identity_sha256',NEW.subject_identity_sha256,
            'semantic_key_sha256',NEW.semantic_key_sha256,
            'structured_claim_sha256',NEW.structured_claim_sha256)::TEXT)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_pending_hypothesis_discoveries_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_pending_hypothesis_discoveries
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pending_hypothesis_discovery();

CREATE TABLE investigation_pending_hypothesis_discovery_consumptions (
    consumption_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    discovery_authority_id UUID NOT NULL UNIQUE REFERENCES
        investigation_pending_hypothesis_discoveries(discovery_authority_id) ON DELETE RESTRICT,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN('admitted','dismissed_duplicate')),
    admitted_root_id UUID REFERENCES attack_hypotheses(root_id) ON DELETE RESTRICT,
    admitted_revision_id UUID REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    compiler_receipt_id UUID REFERENCES investigation_hypothesis_canonical_apply_receipts(
        apply_receipt_id) ON DELETE RESTRICT,
    duplicate_of_revision_id UUID REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    consumption_sha256 TEXT NOT NULL CHECK(consumption_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    consumed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK((disposition='admitted' AND admitted_root_id IS NOT NULL
           AND admitted_revision_id IS NOT NULL AND compiler_receipt_id IS NOT NULL
           AND duplicate_of_revision_id IS NULL)
       OR (disposition='dismissed_duplicate' AND admitted_root_id IS NULL
           AND admitted_revision_id IS NULL AND compiler_receipt_id IS NULL
           AND duplicate_of_revision_id IS NOT NULL))
);

CREATE FUNCTION investigation_guard_pending_hypothesis_discovery_consumption()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE discovery investigation_pending_hypothesis_discoveries%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_CONSUMPTION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT discovery FROM investigation_pending_hypothesis_discoveries
     WHERE discovery_authority_id=NEW.discovery_authority_id FOR SHARE;
    IF ROW(NEW.asset_lane_id,NEW.target_live_id)
       IS DISTINCT FROM ROW(discovery.asset_lane_id,discovery.target_live_id)
       OR (NEW.disposition='admitted' AND NOT EXISTS(
            SELECT 1
              FROM attack_hypotheses root
              JOIN attack_hypothesis_revisions revision
                ON revision.root_id=root.root_id AND revision.revision_id=NEW.admitted_revision_id
              JOIN investigation_hypothesis_canonical_apply_receipts receipt
                ON receipt.apply_receipt_id=NEW.compiler_receipt_id
               AND receipt.generation_id IN(
                    SELECT generation_id FROM hypothesis_generation_members member
                     WHERE member.revision_id=revision.revision_id)
              JOIN investigation_hypothesis_compilation_members compilation_member
                ON compilation_member.decision_id=receipt.decision_id
               AND compilation_member.proposal_id=discovery.discovery_authority_id
               AND compilation_member.successor_revision_id=revision.revision_id
               AND compilation_member.semantic_key_sha256=discovery.semantic_key_sha256
               AND compilation_member.route_kind='create_initial'
               AND compilation_member.created_at>=discovery.created_at
             WHERE root.root_id=NEW.admitted_root_id
               AND root.asset_lane_id=discovery.asset_lane_id
               AND revision.asset_lane_id=discovery.asset_lane_id
               AND revision.target_live_id=discovery.target_live_id
               AND revision.semantic_key_hash=discovery.semantic_key_sha256))
       OR (NEW.disposition='dismissed_duplicate' AND NOT EXISTS(
            SELECT 1 FROM attack_hypothesis_revisions duplicate
             WHERE duplicate.revision_id=NEW.duplicate_of_revision_id
               AND duplicate.asset_lane_id=discovery.asset_lane_id
               AND duplicate.target_live_id=discovery.target_live_id
               AND duplicate.semantic_key_hash=discovery.semantic_key_sha256))
       OR NEW.consumption_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_pending_hypothesis_discovery_consumption.v1',
            'discovery_authority_id',NEW.discovery_authority_id,
            'asset_lane_id',NEW.asset_lane_id,'target_live_id',NEW.target_live_id,
            'disposition',NEW.disposition,'admitted_root_id',NEW.admitted_root_id,
            'admitted_revision_id',NEW.admitted_revision_id,
            'compiler_receipt_id',NEW.compiler_receipt_id,
            'duplicate_of_revision_id',NEW.duplicate_of_revision_id)::TEXT)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_CONSUMPTION_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_pending_hypothesis_discovery_consumptions_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_pending_hypothesis_discovery_consumptions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_pending_hypothesis_discovery_consumption();

CREATE VIEW investigation_pending_hypothesis_discovery_backlog AS
SELECT discovery.*
  FROM investigation_pending_hypothesis_discoveries discovery
  JOIN investigation_asset_verification_sessions session_row
    ON session_row.session_id=discovery.session_id
 WHERE session_row.state='resolved'
   AND NOT EXISTS(SELECT 1 FROM investigation_pending_hypothesis_discovery_consumptions consumed
                   WHERE consumed.discovery_authority_id=discovery.discovery_authority_id);

-- 00006 predates mid-verification discovery authority. Extend its fixed-point
-- admission without rewriting the historical migration: any unconsumed typed
-- discovery is an asset-local backlog member and prevents close.
CREATE FUNCTION investigation_guard_asset_fixed_point_pending_discoveries()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS(SELECT 1 FROM investigation_pending_hypothesis_discovery_backlog backlog
               WHERE backlog.asset_lane_id=NEW.asset_lane_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_PENDING_HYPOTHESIS_DISCOVERY'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_backlog_discovery_guard
BEFORE INSERT ON investigation_asset_backlog_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_fixed_point_pending_discoveries();

ALTER TABLE investigation_asset_verification_sessions
    ADD CONSTRAINT investigation_asset_verification_session_resolution_fk
    FOREIGN KEY(resolution_authority_id)
        REFERENCES investigation_hypothesis_resolution_authorities(resolution_authority_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION investigation_guard_hypothesis_resolution()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE session_row investigation_asset_verification_sessions%ROWTYPE;
DECLARE primary_worker stage_worker_runs%ROWTYPE;
DECLARE adviser_review stage_worker_outputs%ROWTYPE;
DECLARE adviser_worker stage_worker_runs%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_RESOLUTION_APPEND_ONLY' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT session_row FROM investigation_asset_verification_sessions
     WHERE session_id=NEW.session_id FOR UPDATE;
    SELECT * INTO STRICT primary_worker FROM stage_worker_runs
     WHERE id=NEW.primary_worker_run_id FOR SHARE;
    SELECT * INTO STRICT adviser_worker FROM stage_worker_runs
     WHERE id=NEW.adviser_worker_run_id FOR SHARE;
    SELECT * INTO STRICT adviser_review FROM stage_worker_outputs
     WHERE id=NEW.adviser_review_output_id FOR SHARE;
    IF session_row.state<>'open' OR session_row.head_version<>NEW.expected_session_head_version
       OR ROW(session_row.asset_lane_id,session_row.target_live_id,
              session_row.hypothesis_revision_id,session_row.primary_work_item_id,
              session_row.primary_worker_run_id,session_row.primary_message_chain_id,
              session_row.adviser_work_item_id,session_row.adviser_worker_run_id,
              session_row.adviser_message_chain_id)
          IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_live_id,
              NEW.hypothesis_revision_id,NEW.primary_work_item_id,
              NEW.primary_worker_run_id,NEW.primary_message_chain_id,
              NEW.adviser_work_item_id,NEW.adviser_worker_run_id,
              NEW.adviser_message_chain_id)
       OR primary_worker.lease_token<>NEW.primary_lease_token
       OR primary_worker.attempt_epoch<>NEW.primary_attempt_epoch
       OR primary_worker.checkpoint_version<>NEW.primary_checkpoint_version
       OR primary_worker.status NOT IN('running','waiting_background')
       OR primary_worker.lease_expires_at IS NULL
       OR primary_worker.lease_expires_at<=statement_timestamp()
       OR adviser_worker.status<>'passed'
       OR ROW(adviser_review.work_item_id,adviser_review.worker_run_id,
              adviser_review.operation_id,adviser_review.stage_execution_id,
              adviser_review.stage_run_unit_id,adviser_review.scope_snapshot_id,
              adviser_review.organization_id)
          IS DISTINCT FROM ROW(NEW.adviser_work_item_id,NEW.adviser_worker_run_id,
              session_row.operation_id,session_row.stage_execution_id,
              session_row.stage_run_unit_id,session_row.scope_snapshot_id,
              session_row.organization_id)
       OR adviser_review.output_schema<>
          'investigation_asset_verification_adviser_review.v1'
       OR adviser_review.business_disposition<>'artifact_recorded'
       OR adviser_review.output_hash<>(
            'sha256:' || verification_sha256_jsonb(jsonb_build_object(
                'blocker_code',to_jsonb(adviser_review.blocker_codes)->0,
                'canonical_output',adviser_review.canonical_output,
                'checked_empty_units',adviser_review.checked_empty_cells,
                'disposition',adviser_review.business_disposition,
                'evidence_ids',to_jsonb(adviser_review.evidence_ids),
                'fact_refs',adviser_review.canonical_fact_refs,
                'output_schema',adviser_review.output_schema,
                'work_item_id',adviser_review.work_item_id,
                'worker_run_id',adviser_review.worker_run_id)))
       OR adviser_review.output_hash<>NEW.adviser_review_output_sha256
       OR adviser_review.canonical_output->>'session_id'<>NEW.session_id::TEXT
       OR adviser_review.canonical_output->>'hypothesis_revision_id'<>
          NEW.hypothesis_revision_id::TEXT
       OR adviser_review.canonical_output->>'recommendation'<>NEW.disposition
       OR adviser_review.canonical_output->>'adviser_concurrence_sha256'<>
          NEW.adviser_concurrence_sha256
       OR NEW.resolution_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_hypothesis_resolution.v1',
            'session_id',NEW.session_id,
            'hypothesis_revision_id',NEW.hypothesis_revision_id,
            'disposition',NEW.disposition,
            'primary_conclusion_sha256',NEW.primary_conclusion_sha256,
            'adviser_concurrence_sha256',NEW.adviser_concurrence_sha256,
            'adviser_review_output_id',NEW.adviser_review_output_id,
            'adviser_review_output_sha256',NEW.adviser_review_output_sha256,
            'citation_set_sha256',NEW.citation_set_sha256)::TEXT)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_RESOLUTION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_hypothesis_resolution_authorities_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_hypothesis_resolution_authorities
FOR EACH ROW EXECUTE FUNCTION investigation_guard_hypothesis_resolution();

CREATE FUNCTION investigation_validate_hypothesis_resolution_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_resolution_id UUID := COALESCE(NEW.resolution_authority_id,OLD.resolution_authority_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_hypothesis_resolution_authorities resolution
         WHERE resolution.resolution_authority_id=requested_resolution_id
           AND ROW(resolution.citation_count,resolution.citation_set_sha256)
             IS DISTINCT FROM ROW(
                 (SELECT COUNT(*) FROM investigation_hypothesis_resolution_citations citation
                   WHERE citation.resolution_authority_id=requested_resolution_id),
                 tool_truth_sha256(COALESCE((SELECT jsonb_agg(citation.citation_sha256
                     ORDER BY citation.citation_ordinal)::TEXT
                   FROM investigation_hypothesis_resolution_citations citation
                  WHERE citation.resolution_authority_id=requested_resolution_id),'[]')))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_RESOLUTION_CITATION_CENSUS_DRIFT'
            USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_hypothesis_resolution_citation_census_exact
AFTER INSERT OR UPDATE OR DELETE ON investigation_hypothesis_resolution_citations
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_hypothesis_resolution_census();
CREATE TRIGGER investigation_hypothesis_resolution_citations_immutable
BEFORE INSERT OR UPDATE OR DELETE ON investigation_hypothesis_resolution_citations
FOR EACH ROW EXECUTE FUNCTION investigation_guard_hypothesis_resolution_citation();

CREATE TRIGGER investigation_asset_verification_sessions_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_verification_sessions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_verification_session();

-- `artifact_recorded` is normally reserved for Candidate Analysis artifacts.
-- Verification has its own dedicated immutable authority, validated by the
-- Task4 repository and resolution trigger, so the historical candidate-only
-- deferred trigger must not require a fake candidate artifact for these exact
-- schemas.
CREATE OR REPLACE FUNCTION enforce_candidate_artifact_recorded_output()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE artifact_count BIGINT;
BEGIN
    IF NEW.business_disposition<>'artifact_recorded'
       OR NEW.output_schema IN(
            'investigation_asset_verification_actor_observation.v1',
            'investigation_asset_verification_adviser_review.v1',
            'investigation_asset_verification_primary_resolution.v1')
    THEN
        RETURN NULL;
    END IF;
    SELECT count(*) INTO artifact_count
      FROM candidate_analysis_artifacts artifact
      JOIN candidate_analysis_work_items candidate_item
        ON candidate_item.candidate_work_item_id=artifact.candidate_work_item_id
     WHERE artifact.stage_worker_output_id=NEW.id
       AND artifact.worker_run_id=NEW.worker_run_id
       AND candidate_item.stage_work_item_id=NEW.work_item_id;
    IF artifact_count<>1 THEN
        RAISE EXCEPTION 'CANDIDATE_ANALYSIS_ARTIFACT_RECORDED_EXACT_ARTIFACT_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

-- A trigger function cannot invoke another trigger function as a normal SQL
-- function.  Verification round rearm therefore uses a narrowly-scoped,
-- transaction-local admission flag which the existing plan contract consumes.
-- The DB repo sets it only after inserting and locking the exact building
-- round-rearm authority; the plan trigger below rejects every other update.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    resolved_round_rearm_id UUID := NULLIF(current_setting(
        'golish.investigation_asset_verification_round_rearm_id',TRUE),'')::UUID;
    round_advance BOOLEAN := FALSE;
    existing_epoch_advance BOOLEAN := FALSE;
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
       IS DISTINCT FROM ROW(OLD.id,OLD.operation_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,
        OLD.unit_generation,OLD.schema_version,OLD.plan_version,OLD.plan_hash,
        OLD.leader_role,OLD.aggregator_kind,OLD.aggregator_role,OLD.allowed_worker_roles,
        OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,OLD.created_at)
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
    THEN RAISE EXCEPTION 'STAGE_TEAM_PLAN_NOOP_UPDATE_FORBIDDEN'; END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
