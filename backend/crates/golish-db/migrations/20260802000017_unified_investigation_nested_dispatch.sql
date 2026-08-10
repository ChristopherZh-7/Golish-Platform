-- Atomic nested cognition lifecycle for unified Investigation.
--
-- A begin receipt is inserted in the same transaction as the accepted
-- StageWorkerRequest, exact WorkItem/WorkerRun/chain lease and the PentAGI
-- NestedWorker dispatch.  A finish receipt is inserted in the same transaction
-- as the StageWorkerOutput and PentAGI dispatch attempt.  Both tables are
-- append-only replay witnesses; no external action authority is represented.

CREATE TABLE investigation_nested_dispatch_begins (
    begin_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL CHECK (btrim(owning_stage_run_request_id)<>''),
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    task_plan_id UUID NOT NULL,
    subtask_id UUID NOT NULL,
    parent_dispatch_receipt_id UUID NOT NULL,
    parent_worker_run_id UUID NOT NULL,
    parent_work_item_id UUID NOT NULL,
    parent_lease_token UUID NOT NULL,
    parent_attempt_epoch BIGINT NOT NULL CHECK (parent_attempt_epoch>=0),
    parent_checkpoint_version BIGINT NOT NULL CHECK (parent_checkpoint_version>=0),
    stage_team_plan_id UUID NOT NULL,
    dispatch_epoch BIGINT NOT NULL CHECK (dispatch_epoch>=0),
    nested_tool_request_id TEXT NOT NULL CHECK (btrim(nested_tool_request_id)<>''),
    requested_role TEXT NOT NULL CHECK (btrim(requested_role)<>''),
    objective TEXT NOT NULL CHECK (btrim(objective)<>''),
    args_sha256 TEXT NOT NULL CHECK (args_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    snapshot_sha256 TEXT NOT NULL CHECK (snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    request_sha256 TEXT NOT NULL CHECK (request_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dispatch_ordinal INTEGER NOT NULL CHECK (dispatch_ordinal>=0),
    stage_worker_request_id UUID NOT NULL UNIQUE,
    child_work_item_id UUID NOT NULL UNIQUE,
    child_worker_run_id UUID NOT NULL UNIQUE,
    child_message_chain_id UUID NOT NULL UNIQUE,
    child_dispatch_receipt_id UUID NOT NULL UNIQUE,
    begin_receipt_sha256 TEXT NOT NULL CHECK (begin_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(task_plan_id,subtask_id,parent_dispatch_receipt_id,nested_tool_request_id),
    FOREIGN KEY(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES investigation_pentagi_task_plans(
        task_plan_id,authority_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(subtask_id,task_plan_id)
        REFERENCES investigation_pentagi_subtasks(subtask_id,task_plan_id) ON DELETE RESTRICT,
    FOREIGN KEY(parent_dispatch_receipt_id,task_plan_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id,task_plan_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_worker_request_id)
        REFERENCES stage_worker_requests(id) ON DELETE RESTRICT,
    FOREIGN KEY(
        child_work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES stage_work_items(
        id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        child_worker_run_id,child_work_item_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ) REFERENCES stage_worker_runs(
        id,work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(child_dispatch_receipt_id,task_plan_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id,task_plan_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(child_message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT
);

CREATE TABLE investigation_nested_dispatch_finishes (
    finish_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    begin_receipt_id UUID NOT NULL UNIQUE
        REFERENCES investigation_nested_dispatch_begins(begin_receipt_id) ON DELETE RESTRICT,
    task_plan_id UUID NOT NULL,
    subtask_id UUID NOT NULL,
    parent_dispatch_receipt_id UUID NOT NULL,
    child_dispatch_receipt_id UUID NOT NULL,
    child_worker_run_id UUID NOT NULL,
    child_work_item_id UUID NOT NULL,
    child_lease_token UUID NOT NULL,
    child_attempt_epoch BIGINT NOT NULL CHECK (child_attempt_epoch>=0),
    child_checkpoint_version BIGINT NOT NULL CHECK (child_checkpoint_version>=0),
    dispatch_attempt_id UUID NOT NULL UNIQUE,
    output_id UUID NOT NULL UNIQUE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'completed','blocked','residual','recovery_required','unknown_held'
    )),
    result_sha256 TEXT NOT NULL CHECK (result_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fence_sha256 TEXT NOT NULL CHECK (fence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    finish_receipt_sha256 TEXT NOT NULL CHECK (finish_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(child_dispatch_receipt_id,task_plan_id)
        REFERENCES pentagi_logical_dispatch_receipts(dispatch_receipt_id,task_plan_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(dispatch_attempt_id,child_dispatch_receipt_id)
        REFERENCES pentagi_logical_dispatch_attempts(dispatch_attempt_id,dispatch_receipt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(output_id)
        REFERENCES stage_worker_outputs(id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_nested_dispatch_begins_append_only
BEFORE UPDATE OR DELETE ON investigation_nested_dispatch_begins
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER investigation_nested_dispatch_finishes_append_only
BEFORE UPDATE OR DELETE ON investigation_nested_dispatch_finishes
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- One exact receipt authorizes reopening the StageTeam governance epoch for
-- the next server-selected VerificationTask.  The receipt is building only
-- inside the compound transaction and becomes immutable after every new
-- Primary identity is durable.
CREATE TABLE investigation_task_primary_rearms (
    rearm_receipt_id UUID PRIMARY KEY,
    verification_task_id UUID NOT NULL UNIQUE,
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    subject_fingerprint_sha256 TEXT NOT NULL
        CHECK (subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_dispatch_epoch BIGINT NOT NULL CHECK (source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL
        CHECK (resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK (source_plan_row_version>=0),
    previous_primary_work_item_id UUID NOT NULL,
    previous_primary_worker_run_id UUID NOT NULL,
    previous_primary_item_row_version BIGINT NOT NULL
        CHECK (previous_primary_item_row_version>=0),
    previous_primary_attempt_epoch BIGINT NOT NULL
        CHECK (previous_primary_attempt_epoch>=0),
    previous_primary_checkpoint_version BIGINT NOT NULL
        CHECK (previous_primary_checkpoint_version>=0),
    primary_work_item_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL UNIQUE,
    primary_message_chain_id UUID NOT NULL UNIQUE,
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    UNIQUE(stage_team_plan_id,resume_dispatch_epoch),
    CHECK (
        (status='building' AND applied_at IS NULL)
        OR (status='applied' AND applied_at IS NOT NULL)
    ),
    FOREIGN KEY(
        stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) REFERENCES stage_team_plans(
        id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) REFERENCES hypothesis_verification_tasks(
        task_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        previous_primary_work_item_id,stage_team_plan_id,operation_id,
        stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id
    ) REFERENCES stage_work_items(
        id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        previous_primary_worker_run_id,previous_primary_work_item_id,operation_id,
        stage_execution_id,stage_run_unit_id,organization_id
    ) REFERENCES stage_worker_runs(
        id,work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_investigation_task_primary_rearm_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    prior_item stage_work_items%ROWTYPE;
    prior_worker stage_worker_runs%ROWTYPE;
    revision_sha256 TEXT;
    verification_plan_sha256 TEXT;
    assignment_sha256 TEXT;
    semantic_attempt_fingerprint TEXT;
    campaign_denominator_sha256 TEXT;
    expected_subject_fingerprint TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_TASK_PRIMARY_REARM_APPEND_ONLY';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL THEN
            RAISE EXCEPTION 'INVESTIGATION_TASK_PRIMARY_REARM_MUST_BUILD_FIRST';
        END IF;
        SELECT * INTO STRICT plan FROM stage_team_plans
         WHERE id=NEW.stage_team_plan_id FOR UPDATE;
        SELECT * INTO STRICT prior_item FROM stage_work_items
         WHERE id=NEW.previous_primary_work_item_id FOR SHARE;
        SELECT * INTO STRICT prior_worker FROM stage_worker_runs
         WHERE id=NEW.previous_primary_worker_run_id FOR SHARE;
        IF plan.operation_id<>NEW.operation_id
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
           OR prior_item.team_plan_id<>plan.id
           OR prior_item.status<>'completed'
           OR prior_item.row_version<>NEW.previous_primary_item_row_version
           OR prior_item.required_for_barrier
           OR prior_item.role<>plan.leader_role
           OR NOT (
                prior_item.stable_key='leader:primary'
                OR prior_item.stable_key ~ '^task:[0-9a-f-]{36}:primary$'
           )
           OR prior_worker.work_item_id<>prior_item.id
           OR prior_worker.status<>'passed'
           OR prior_worker.attempt_epoch<>NEW.previous_primary_attempt_epoch
           OR prior_worker.checkpoint_version<>NEW.previous_primary_checkpoint_version
           OR EXISTS(
                SELECT 1 FROM stage_work_items item
                 WHERE item.team_plan_id=plan.id AND item.required_for_barrier
                   AND (
                       item.status NOT IN ('completed','exhausted','superseded')
                       OR NOT EXISTS(
                            SELECT 1 FROM stage_worker_outputs output
                             WHERE output.work_item_id=item.id
                       )
                   )
           )
           OR EXISTS(SELECT 1 FROM stage_work_items WHERE id=NEW.primary_work_item_id)
           OR EXISTS(SELECT 1 FROM stage_worker_runs WHERE id=NEW.primary_worker_run_id)
           OR EXISTS(SELECT 1 FROM message_chains WHERE id=NEW.primary_message_chain_id)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_TASK_PRIMARY_REARM_AUTHORITY_MISMATCH';
        END IF;
        SELECT task.hypothesis_revision_sha256,task.verification_plan_sha256,
               assignment.member_set_sha256,task.semantic_attempt_fingerprint,
               unified_investigation_exact_set_hash(
                   'verification_task_campaigns.v1',
                   array_agg(campaign.campaign_id::TEXT ORDER BY campaign.campaign_id)
               )
          INTO STRICT revision_sha256,verification_plan_sha256,assignment_sha256,
                      semantic_attempt_fingerprint,campaign_denominator_sha256
          FROM hypothesis_verification_tasks task
          JOIN hypothesis_verification_task_assignment_sets assignment
            ON assignment.task_id=task.task_id AND assignment.status='sealed'
          JOIN hypothesis_verification_task_campaigns campaign
            ON campaign.task_id=task.task_id
           AND campaign.assignment_set_id=assignment.assignment_set_id
         WHERE task.task_id=NEW.verification_task_id
           AND task.operation_id=NEW.operation_id
           AND task.stage_execution_id=NEW.stage_execution_id
           AND task.stage_run_unit_id=NEW.stage_run_unit_id
           AND task.scope_snapshot_id=NEW.scope_snapshot_id
           AND task.organization_id=NEW.organization_id
         GROUP BY task.hypothesis_revision_sha256,task.verification_plan_sha256,
                  assignment.member_set_sha256,task.semantic_attempt_fingerprint;
        expected_subject_fingerprint := tool_truth_sha256(jsonb_build_object(
            'task_id',NEW.verification_task_id,
            'revision_sha256',revision_sha256,
            'plan_sha256',verification_plan_sha256,
            'assignment_sha256',assignment_sha256,
            'campaign_denominator_sha256',campaign_denominator_sha256,
            'semantic_attempt_fingerprint',semantic_attempt_fingerprint
        )::TEXT);
        IF expected_subject_fingerprint<>NEW.subject_fingerprint_sha256 THEN
            RAISE EXCEPTION 'INVESTIGATION_TASK_PRIMARY_REARM_SUBJECT_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT plan FROM stage_team_plans
     WHERE id=NEW.stage_team_plan_id FOR SHARE;
    IF OLD.status<>'building' OR NEW.status<>'applied'
       OR NEW.applied_at IS NULL
       OR ROW(
            NEW.rearm_receipt_id,NEW.verification_task_id,NEW.stage_team_plan_id,
            NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
            NEW.scope_snapshot_id,NEW.organization_id,NEW.subject_fingerprint_sha256,
            NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
            NEW.source_plan_row_version,NEW.previous_primary_work_item_id,
            NEW.previous_primary_worker_run_id,NEW.previous_primary_item_row_version,
            NEW.previous_primary_attempt_epoch,NEW.previous_primary_checkpoint_version,
            NEW.primary_work_item_id,NEW.primary_worker_run_id,
            NEW.primary_message_chain_id,NEW.receipt_sha256,NEW.created_at
       ) IS DISTINCT FROM ROW(
            OLD.rearm_receipt_id,OLD.verification_task_id,OLD.stage_team_plan_id,
            OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
            OLD.scope_snapshot_id,OLD.organization_id,OLD.subject_fingerprint_sha256,
            OLD.source_dispatch_epoch,OLD.resume_dispatch_epoch,
            OLD.source_plan_row_version,OLD.previous_primary_work_item_id,
            OLD.previous_primary_worker_run_id,OLD.previous_primary_item_row_version,
            OLD.previous_primary_attempt_epoch,OLD.previous_primary_checkpoint_version,
            OLD.primary_work_item_id,OLD.primary_worker_run_id,
            OLD.primary_message_chain_id,OLD.receipt_sha256,OLD.created_at
       )
       OR NOT EXISTS(
            SELECT 1 FROM stage_team_plans current_plan
             WHERE current_plan.id=NEW.stage_team_plan_id
               AND current_plan.dispatch_epoch=NEW.resume_dispatch_epoch
               AND current_plan.row_version=NEW.source_plan_row_version+1
               AND current_plan.requests_closed_at IS NULL
       )
       OR NOT EXISTS(
            SELECT 1 FROM stage_work_items item
             WHERE item.id=NEW.primary_work_item_id
               AND item.team_plan_id=NEW.stage_team_plan_id
               AND item.dispatch_epoch=NEW.resume_dispatch_epoch
               AND item.stable_key='task:'||NEW.verification_task_id::TEXT||':primary'
               AND item.role=plan.leader_role
               AND item.input_manifest_hash=NEW.subject_fingerprint_sha256
               AND item.status='queued' AND NOT item.required_for_barrier
       )
       OR NOT EXISTS(
            SELECT 1 FROM stage_worker_runs worker
             WHERE worker.id=NEW.primary_worker_run_id
               AND worker.work_item_id=NEW.primary_work_item_id
               AND worker.status='queued'
               AND worker.message_chain_id=NEW.primary_message_chain_id
       )
       OR NOT EXISTS(
            SELECT 1 FROM message_chains chain
             WHERE chain.id=NEW.primary_message_chain_id
               AND chain.task_id=NEW.operation_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_TASK_PRIMARY_REARM_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_task_primary_rearms_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_task_primary_rearms
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_task_primary_rearm_receipt();

-- Preserve every existing StageTeam transition while adding exactly one new
-- closed-to-open case backed by the building receipt above.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
    target_intel_goal_resume_advance BOOLEAN := FALSE;
    investigation_task_rearm_advance BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NOT (NEW.allowed_worker_roles ? NEW.leader_role)
            OR (NEW.aggregator_kind='worker' AND NOT (NEW.allowed_worker_roles ? NEW.aggregator_role))
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_ROLE_NOT_ALLOWED';
        END IF;
        IF EXISTS (SELECT 1 FROM stage_worker_runs worker WHERE worker.stage_run_unit_id=NEW.stage_run_unit_id) THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.stage_kind,NEW.unit_generation,
        NEW.schema_version,NEW.plan_version,NEW.plan_hash,NEW.leader_role,
        NEW.aggregator_kind,NEW.aggregator_role,NEW.allowed_worker_roles,
        NEW.max_workers_total,NEW.max_workers_active,NEW.dynamic_requests_allowed,
        NEW.dynamic_request_policy,NEW.final_submitter_kind,
        NEW.created_from_stage_spec_hash,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,OLD.unit_generation,
        OLD.schema_version,OLD.plan_version,OLD.plan_hash,OLD.leader_role,
        OLD.aggregator_kind,OLD.aggregator_role,OLD.allowed_worker_roles,
        OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;
    repair_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_repair_generations generation
            JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
             WHERE generation.team_plan_id=OLD.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
               AND gap.source_dispatch_epoch=OLD.dispatch_epoch
               AND gap.source_aggregator_worker_run_id=OLD.final_submitter_worker_run_id
        );
    controller_turn_resume_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_controller_turn_resumes authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.leader_worker_run_id=OLD.final_submitter_worker_run_id
        );
    target_intel_goal_resume_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM target_intel_goal_resume_authorities authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_goal_epoch=OLD.dispatch_epoch
               AND authority.successor_goal_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.controller_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
        );
    IF NOT target_intel_goal_resume_advance THEN
        target_intel_goal_resume_advance :=
            NEW.dispatch_epoch=OLD.dispatch_epoch+1
            AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
            AND OLD.final_submitter_worker_run_id IS NULL
            AND NEW.final_submitter_worker_run_id IS NULL
            AND EXISTS (
                SELECT 1 FROM target_intel_goal_resume_authorities authority
                 WHERE authority.team_plan_id=OLD.id AND authority.status='building'
                   AND authority.source_goal_epoch=OLD.dispatch_epoch
                   AND authority.successor_goal_epoch=NEW.dispatch_epoch
                   AND authority.source_plan_row_version=OLD.row_version
            );
    END IF;
    investigation_task_rearm_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS (
            SELECT 1 FROM investigation_task_primary_rearms authority
             WHERE authority.stage_team_plan_id=OLD.id
               AND authority.operation_id=OLD.operation_id
               AND authority.stage_execution_id=OLD.stage_execution_id
               AND authority.stage_run_unit_id=OLD.stage_run_unit_id
               AND authority.scope_snapshot_id=OLD.scope_snapshot_id
               AND authority.organization_id=OLD.organization_id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
        );
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT investigation_task_rearm_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR';
    END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED';
    END IF;
    IF OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT investigation_task_rearm_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT investigation_task_rearm_advance
        AND NOT (
            NEW.final_submitter_worker_run_id IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM stage_worker_runs previous_submitter
                 WHERE previous_submitter.id=OLD.final_submitter_worker_run_id
                   AND previous_submitter.status='superseded'
            )
        )
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_FINAL_SUBMITTER_IMMUTABLE';
    END IF;
    IF NEW.requests_closed_at IS NOT DISTINCT FROM OLD.requests_closed_at
        AND NEW.dispatch_epoch IS NOT DISTINCT FROM OLD.dispatch_epoch
        AND NEW.final_submitter_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_NOOP_UPDATE_FORBIDDEN';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
