-- Primary-led Investigation verification execution authority.
--
-- The immutable assignment freezes every authority axis before external I/O.
-- Its mutable head is an exact worker/lease fence; events are append-only.
-- Tool execution remains application-owned and never runs in a DB transaction.

CREATE TABLE investigation_verification_execution_assignments (
    assignment_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL,
    task_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    assignment_set_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    campaign_dispatch_generation BIGINT NOT NULL CHECK(campaign_dispatch_generation>=0),
    plan_objective_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    prepared_action_id UUID NOT NULL,
    action_execution_id UUID NOT NULL UNIQUE,
    authorization_receipt_id UUID NOT NULL,
    authorization_hash TEXT NOT NULL CHECK(authorization_hash ~ '^sha256:[0-9a-f]{64}$'),
    authorization_expires_at TIMESTAMPTZ NOT NULL,
    budget_reservation_id UUID NOT NULL,
    budget_contract_set_sha256 TEXT NOT NULL CHECK(budget_contract_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    conflict_set_id UUID NOT NULL,
    conflict_member_set_sha256 TEXT NOT NULL CHECK(conflict_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    stage_worker_request_id UUID NOT NULL UNIQUE,
    execution_work_item_id UUID NOT NULL UNIQUE,
    execution_worker_role TEXT NOT NULL CHECK(execution_worker_role IN(
        'pentester','browser','coder','researcher','installer','memorist','adviser'
    )),
    allowed_tool_names JSONB NOT NULL CHECK(stage_team_json_string_array_is_valid(allowed_tool_names)),
    allowed_tool_types JSONB NOT NULL CHECK(stage_team_json_string_array_is_valid(allowed_tool_types)),
    canonical_args JSONB NOT NULL CHECK(jsonb_typeof(canonical_args)='object'),
    canonical_args_sha256 TEXT NOT NULL CHECK(canonical_args_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    private_manifest_sha256 TEXT NOT NULL CHECK(private_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    evidence_contract_sha256 TEXT NOT NULL CHECK(evidence_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    oracle_contract_sha256 TEXT NOT NULL CHECK(oracle_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    assignment_authority_sha256 TEXT NOT NULL CHECK(assignment_authority_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    contract_version TEXT NOT NULL DEFAULT 'investigation_verification_execution_assignment.v1'
        CHECK(contract_version='investigation_verification_execution_assignment.v1'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(assignment_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id),
    UNIQUE(assignment_id,action_execution_id),
    UNIQUE(assignment_id,verification_task_id,task_plan_id,action_execution_id),
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(task_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        REFERENCES investigation_pentagi_task_plans(task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(campaign_id,assignment_set_id,verification_task_id,plan_objective_id)
        REFERENCES hypothesis_verification_task_campaigns(campaign_id,assignment_set_id,task_id,plan_objective_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_executions(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(authorization_receipt_id,prepared_action_id)
        REFERENCES verification_prepared_action_authorizations(authorization_receipt_id,prepared_action_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(budget_reservation_id,prepared_action_id,authorization_receipt_id)
        REFERENCES verification_budget_reservations(budget_reservation_id,prepared_action_id,authorization_receipt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(conflict_set_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_conflict_sets(conflict_set_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_worker_request_id)
        REFERENCES stage_worker_requests(id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_work_item_id)
        REFERENCES stage_work_items(id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_verification_execution_assignment_insert()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    action verification_prepared_actions%ROWTYPE;
    execution verification_action_executions%ROWTYPE;
    action_authorization verification_prepared_action_authorizations%ROWTYPE;
    task_plan investigation_pentagi_task_plans%ROWTYPE;
    worker_request stage_worker_requests%ROWTYPE;
    work_item stage_work_items%ROWTYPE;
BEGIN
    SELECT * INTO STRICT execution FROM verification_action_executions
     WHERE action_execution_id=NEW.action_execution_id FOR SHARE;
    SELECT * INTO STRICT action FROM verification_prepared_actions
     WHERE prepared_action_id=NEW.prepared_action_id FOR SHARE;
    SELECT * INTO STRICT action_authorization FROM verification_prepared_action_authorizations
     WHERE authorization_receipt_id=NEW.authorization_receipt_id FOR SHARE;
    SELECT * INTO STRICT task_plan FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id FOR SHARE;
    SELECT * INTO STRICT worker_request FROM stage_worker_requests
     WHERE id=NEW.stage_worker_request_id FOR SHARE;
    SELECT * INTO STRICT work_item FROM stage_work_items
     WHERE id=NEW.execution_work_item_id FOR SHARE;

    IF execution.state<>'started' OR action.state<>'started'
       OR action_authorization.decision<>'authorized'
       OR action_authorization.expires_at IS NULL
       OR action_authorization.expires_at<=statement_timestamp()
       OR action_authorization.campaign_dispatch_generation<>execution.campaign_dispatch_generation
       OR action_authorization.authorization_hash<>NEW.authorization_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_JIT_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    IF task_plan.subject_kind<>'verification_task'
       OR task_plan.subject_id<>NEW.verification_task_id
       OR task_plan.status<>'sealed'
       OR task_plan.sealed_at IS NULL
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_REQUIRES_SEALED_TASK_PLAN' USING ERRCODE='23514';
    END IF;
    IF worker_request.status<>'accepted'
       OR worker_request.accepted_work_item_id<>NEW.execution_work_item_id
       OR worker_request.request_kind<>'investigation_verification_execution'
       OR worker_request.expected_output_schema<>'investigation_verification_execution_output.v1'
       OR work_item.created_by<>'accepted_worker_request'
       OR work_item.output_schema<>'investigation_verification_execution_output.v1'
       OR work_item.status NOT IN ('queued','claimed','running')
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_WORK_ITEM_INVALID' USING ERRCODE='23514';
    END IF;
    IF NEW.execution_worker_role IS DISTINCT FROM work_item.role OR NOT EXISTS(
        SELECT 1
          FROM investigation_verification_task_advisory_receipts receipt
          JOIN investigation_verification_task_advisory_seals seal
            ON seal.advisory_receipt_id=receipt.advisory_receipt_id
          JOIN investigation_verification_task_advisory_members member
            ON member.advisory_receipt_id=receipt.advisory_receipt_id
           AND member.verification_task_id=receipt.verification_task_id
         WHERE receipt.verification_task_id=NEW.verification_task_id
           AND receipt.task_plan_id=NEW.task_plan_id
           AND receipt.status='applied' AND receipt.applied_at IS NOT NULL
           AND member.campaign_id=NEW.campaign_id
           AND member.typed_intent->>'worker_role'=NEW.execution_worker_role
           AND member.typed_intent->>'worker_role' IN(
               'pentester','browser','coder','researcher','installer','memorist','adviser'
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_WORKER_ROLE_INVALID' USING ERRCODE='23514';
    END IF;
    IF ROW(task_plan.operation_id,task_plan.stage_execution_id,task_plan.stage_run_unit_id,
           task_plan.scope_snapshot_id,task_plan.organization_id)
       IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
                            NEW.scope_snapshot_id,NEW.organization_id)
       OR ROW(worker_request.operation_id,worker_request.stage_execution_id,worker_request.stage_run_unit_id,
              worker_request.scope_snapshot_id,worker_request.organization_id)
       IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
                            NEW.scope_snapshot_id,NEW.organization_id)
       OR ROW(work_item.operation_id,work_item.stage_execution_id,work_item.stage_run_unit_id,
              work_item.scope_snapshot_id,work_item.organization_id)
       IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
                            NEW.scope_snapshot_id,NEW.organization_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_FOREIGN_SCOPE' USING ERRCODE='23514';
    END IF;
    IF NEW.canonical_args IS DISTINCT FROM action.private_manifest
       OR NEW.canonical_args_sha256<>action.private_manifest_hash
       OR NEW.private_manifest_sha256<>action.private_manifest_hash
       OR NEW.oracle_contract_sha256<>action.oracle_contract_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_MANIFEST_DRIFT' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_verification_execution_assignments_insert_guard
BEFORE INSERT ON investigation_verification_execution_assignments
FOR EACH ROW EXECUTE FUNCTION investigation_guard_verification_execution_assignment_insert();
CREATE TRIGGER investigation_verification_execution_assignments_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_execution_assignments
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_verification_execution_assignment_heads (
    assignment_id UUID PRIMARY KEY REFERENCES investigation_verification_execution_assignments(assignment_id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK(state IN('pending','claimed','running','completed','failed','outcome_unknown','recovery_required','blocked')),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    attempt_epoch BIGINT NOT NULL DEFAULT 0 CHECK(attempt_epoch>=0),
    worker_run_id UUID,
    worker_lease_token UUID,
    worker_attempt_epoch BIGINT,
    worker_checkpoint_version BIGINT,
    lease_token UUID,
    lease_owner TEXT,
    lease_acquired_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    active_tool_call_request_id UUID,
    active_tool_name TEXT,
    active_tool_type TEXT,
    active_tool_args_sha256 TEXT CHECK(active_tool_args_sha256 IS NULL OR active_tool_args_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    active_tool_started_at TIMESTAMPTZ,
    capability_execution_receipt_id UUID,
    evidence_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[] CHECK(0 < ALL(evidence_ids)),
    evidence_set_sha256 TEXT CHECK(evidence_set_sha256 IS NULL OR evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    oracle_receipt_id UUID,
    oracle_receipt_sha256 TEXT CHECK(oracle_receipt_sha256 IS NULL OR oracle_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    terminal_authority_sha256 TEXT CHECK(terminal_authority_sha256 IS NULL OR terminal_authority_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    redacted_result JSONB,
    recovery_receipt_id UUID,
    recovery_authority_sha256 TEXT CHECK(recovery_authority_sha256 IS NULL OR recovery_authority_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    recovery_reason_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    terminal_at TIMESTAMPTZ,
    CHECK((state IN('pending','claimed','running') AND terminal_at IS NULL) OR (state NOT IN('pending','claimed','running') AND terminal_at IS NOT NULL)),
    CHECK((state='pending' AND lease_token IS NULL AND worker_run_id IS NULL)
       OR (state<>'pending' AND ((lease_token IS NULL AND state NOT IN('claimed','running')) OR lease_token IS NOT NULL))),
    CHECK((active_tool_call_request_id IS NULL AND active_tool_name IS NULL AND active_tool_type IS NULL
           AND active_tool_args_sha256 IS NULL AND active_tool_started_at IS NULL)
       OR (active_tool_call_request_id IS NOT NULL AND active_tool_name IS NOT NULL AND active_tool_type IS NOT NULL
           AND active_tool_args_sha256 IS NOT NULL AND active_tool_started_at IS NOT NULL)),
    FOREIGN KEY(capability_execution_receipt_id) REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_receipt_id) REFERENCES verification_oracle_assessments(oracle_assessment_id) ON DELETE RESTRICT,
    FOREIGN KEY(worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT
);

CREATE INDEX investigation_verification_execution_assignment_heads_pending
ON investigation_verification_execution_assignment_heads(assignment_id,head_version)
WHERE state='pending';

CREATE FUNCTION investigation_guard_verification_execution_assignment_head()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' OR NEW.assignment_id<>OLD.assignment_id
       OR NEW.head_version<>OLD.head_version+1 OR NEW.updated_at<OLD.updated_at
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_HEAD_CAS_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF ROW(OLD.state,NEW.state) NOT IN (
        ROW('pending','claimed'),ROW('claimed','claimed'),ROW('claimed','running'),
        ROW('running','running'),ROW('running','completed'),ROW('running','failed'),
        ROW('running','outcome_unknown'),ROW('claimed','recovery_required'),
        ROW('running','recovery_required'),ROW('outcome_unknown','completed'),
        ROW('outcome_unknown','failed'),ROW('outcome_unknown','blocked'),
        ROW('recovery_required','completed'),ROW('recovery_required','failed'),
        ROW('recovery_required','blocked')
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_STATE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_verification_execution_assignment_heads_cas
BEFORE UPDATE OR DELETE ON investigation_verification_execution_assignment_heads
FOR EACH ROW EXECUTE FUNCTION investigation_guard_verification_execution_assignment_head();

CREATE TABLE investigation_verification_execution_assignment_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    assignment_id UUID NOT NULL REFERENCES investigation_verification_execution_assignments(assignment_id) ON DELETE RESTRICT,
    event_ordinal BIGINT NOT NULL CHECK(event_ordinal>0),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK(btrim(reason_code)<>''),
    event_sha256 TEXT NOT NULL CHECK(event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(assignment_id,event_ordinal)
);
CREATE TRIGGER investigation_verification_execution_assignment_events_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_execution_assignment_events
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TABLE investigation_verification_execution_assignment_recoveries (
    recovery_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    assignment_id UUID NOT NULL,
    action_execution_id UUID NOT NULL,
    from_state TEXT NOT NULL CHECK(from_state IN('outcome_unknown','recovery_required')),
    recovery_disposition TEXT NOT NULL CHECK(recovery_disposition IN('completed','failed','blocked')),
    recovery_authority_sha256 TEXT NOT NULL CHECK(recovery_authority_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT NOT NULL CHECK(btrim(reason_code)<>''),
    recovered_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(assignment_id,recovery_authority_sha256),
    FOREIGN KEY(assignment_id,action_execution_id)
        REFERENCES investigation_verification_execution_assignments(assignment_id,action_execution_id)
        ON DELETE RESTRICT
);
CREATE TRIGGER investigation_verification_execution_assignment_recoveries_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_execution_assignment_recoveries
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

-- A VerificationTask's planning Primary is deliberately reused as the
-- execution coordinator. Reopening its closed Team epoch is a separate,
-- auditable authority transition; the earlier cognitive rearm receipt is not
-- mutated or overloaded.
CREATE TABLE investigation_verification_execution_primary_rearms (
    rearm_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL UNIQUE,
    task_plan_id UUID NOT NULL UNIQUE,
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    subject_fingerprint_sha256 TEXT NOT NULL
        CHECK(subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL
        CHECK(resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK(source_plan_row_version>=0),
    cognitive_primary_work_item_id UUID NOT NULL,
    cognitive_primary_worker_run_id UUID NOT NULL,
    primary_message_chain_id UUID NOT NULL,
    execution_primary_message_chain_id UUID NOT NULL UNIQUE,
    execution_primary_work_item_id UUID NOT NULL UNIQUE,
    execution_primary_worker_run_id UUID NOT NULL UNIQUE,
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
                                    scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(task_id,operation_id,stage_execution_id,
                                                  stage_run_unit_id,scope_snapshot_id,
                                                  organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(task_plan_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        REFERENCES investigation_pentagi_task_plans(task_plan_id,operation_id,
                                                     stage_execution_id,stage_run_unit_id,
                                                     organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(cognitive_primary_work_item_id)
        REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    FOREIGN KEY(cognitive_primary_worker_run_id)
        REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(primary_message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_primary_message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE FUNCTION enforce_investigation_verification_execution_primary_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_EXECUTION_PRIMARY_REARM_APPEND_ONLY';
    END IF;
    SELECT * INTO STRICT plan FROM stage_team_plans
     WHERE id=NEW.stage_team_plan_id FOR SHARE;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
           OR plan.dispatch_epoch<>NEW.source_dispatch_epoch
           OR plan.row_version<>NEW.source_plan_row_version
           OR plan.requests_closed_at IS NULL
           OR plan.final_submitter_worker_run_id IS NOT NULL
           OR plan.stage_kind<>'investigation'
           OR plan.dynamic_request_policy->>'coordination_mode'<>'investigation_task_orchestrator'
           OR EXISTS(SELECT 1 FROM stage_work_items
                      WHERE id=NEW.execution_primary_work_item_id)
           OR EXISTS(SELECT 1 FROM stage_worker_runs
                      WHERE id=NEW.execution_primary_worker_run_id)
           OR EXISTS(SELECT 1 FROM message_chains
                      WHERE id=NEW.execution_primary_message_chain_id)
           OR NOT EXISTS(
                SELECT 1
                  FROM investigation_task_primary_rearms cognitive_rearm
                  JOIN stage_work_items cognitive_item
                    ON cognitive_item.id=cognitive_rearm.primary_work_item_id
                  JOIN stage_worker_runs cognitive_worker
                    ON cognitive_worker.id=cognitive_rearm.primary_worker_run_id
                   AND cognitive_worker.work_item_id=cognitive_item.id
                  JOIN investigation_pentagi_task_plans task_plan
                    ON task_plan.task_plan_id=NEW.task_plan_id
                   AND task_plan.subject_kind='verification_task'
                   AND task_plan.subject_id=NEW.verification_task_id
                   AND task_plan.status='sealed' AND task_plan.sealed_at IS NOT NULL
                  JOIN investigation_verification_task_advisory_receipts advisory
                    ON advisory.verification_task_id=NEW.verification_task_id
                   AND advisory.task_plan_id=task_plan.task_plan_id
                   AND advisory.status='applied' AND advisory.applied_at IS NOT NULL
                  JOIN investigation_verification_task_advisory_seals advisory_seal
                    ON advisory_seal.advisory_receipt_id=advisory.advisory_receipt_id
                 WHERE cognitive_rearm.verification_task_id=NEW.verification_task_id
                   AND cognitive_rearm.stage_team_plan_id=NEW.stage_team_plan_id
                   AND cognitive_rearm.operation_id=NEW.operation_id
                   AND cognitive_rearm.stage_execution_id=NEW.stage_execution_id
                   AND cognitive_rearm.stage_run_unit_id=NEW.stage_run_unit_id
                   AND cognitive_rearm.scope_snapshot_id=NEW.scope_snapshot_id
                   AND cognitive_rearm.organization_id=NEW.organization_id
                   AND cognitive_rearm.subject_fingerprint_sha256=
                       NEW.subject_fingerprint_sha256
                   AND cognitive_rearm.primary_work_item_id=
                       NEW.cognitive_primary_work_item_id
                   AND cognitive_rearm.primary_worker_run_id=
                       NEW.cognitive_primary_worker_run_id
                   AND cognitive_rearm.primary_message_chain_id=
                       NEW.primary_message_chain_id
                   AND cognitive_rearm.status='applied'
                   AND cognitive_item.status='completed'
                   AND cognitive_worker.status='passed'
                   AND cognitive_worker.message_chain_id=NEW.primary_message_chain_id
           )
        THEN
            RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_EXECUTION_PRIMARY_REARM_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.status<>'building' OR NEW.status<>'applied' OR NEW.applied_at IS NULL
       OR ROW(NEW.rearm_receipt_id,NEW.stable_request_id,NEW.verification_task_id,
              NEW.task_plan_id,NEW.stage_team_plan_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.subject_fingerprint_sha256,
              NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
              NEW.source_plan_row_version,NEW.cognitive_primary_work_item_id,
              NEW.cognitive_primary_worker_run_id,NEW.primary_message_chain_id,
              NEW.execution_primary_message_chain_id,
              NEW.execution_primary_work_item_id,NEW.execution_primary_worker_run_id,
              NEW.receipt_sha256,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.rearm_receipt_id,OLD.stable_request_id,OLD.verification_task_id,
              OLD.task_plan_id,OLD.stage_team_plan_id,OLD.operation_id,
              OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
              OLD.organization_id,OLD.subject_fingerprint_sha256,
              OLD.source_dispatch_epoch,OLD.resume_dispatch_epoch,
              OLD.source_plan_row_version,OLD.cognitive_primary_work_item_id,
              OLD.cognitive_primary_worker_run_id,OLD.primary_message_chain_id,
              OLD.execution_primary_message_chain_id,
              OLD.execution_primary_work_item_id,OLD.execution_primary_worker_run_id,
              OLD.receipt_sha256,OLD.created_at)
       OR NOT EXISTS(
            SELECT 1 FROM stage_team_plans current_plan
             WHERE current_plan.id=NEW.stage_team_plan_id
               AND current_plan.dispatch_epoch=NEW.resume_dispatch_epoch
               AND current_plan.row_version=NEW.source_plan_row_version+1
               AND current_plan.requests_closed_at IS NULL
       )
       OR NOT EXISTS(
            SELECT 1 FROM stage_work_items item
             WHERE item.id=NEW.execution_primary_work_item_id
               AND item.team_plan_id=NEW.stage_team_plan_id
               AND item.dispatch_epoch=NEW.resume_dispatch_epoch
               AND item.kind='investigation_verification_execution_primary'
               AND item.stable_key='task:'||NEW.verification_task_id::TEXT||':primary'
               AND item.role=plan.leader_role
               AND item.input_manifest_hash=NEW.subject_fingerprint_sha256
               AND item.status='queued' AND NOT item.required_for_barrier
       )
       OR NOT EXISTS(
            SELECT 1 FROM stage_worker_runs worker
             WHERE worker.id=NEW.execution_primary_worker_run_id
               AND worker.work_item_id=NEW.execution_primary_work_item_id
               AND worker.status='queued'
               AND worker.message_chain_id=NEW.execution_primary_message_chain_id
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_EXECUTION_PRIMARY_REARM_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_verification_execution_primary_rearms_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_verification_execution_primary_rearms
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_verification_execution_primary_rearm();

-- Extend the existing closed-to-open StageTeam transition with the exact
-- execution-Primary rearm receipt above. All historical transition cases are
-- retained unchanged.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
    target_intel_goal_resume_advance BOOLEAN := FALSE;
    investigation_task_rearm_advance BOOLEAN := FALSE;
    investigation_execution_primary_rearm_advance BOOLEAN := FALSE;
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
    investigation_execution_primary_rearm_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS (
            SELECT 1 FROM investigation_verification_execution_primary_rearms authority
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
        AND NOT investigation_execution_primary_rearm_advance
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
        AND NOT investigation_execution_primary_rearm_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT investigation_task_rearm_advance
        AND NOT investigation_execution_primary_rearm_advance
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
