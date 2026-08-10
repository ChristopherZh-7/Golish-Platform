-- Recover only the exact Application Understanding finalizer that was
-- terminalized after an earlier finished submit_stage_deliverable receipt.
-- The authority row makes the otherwise-forbidden exhausted -> queued
-- WorkItem transition explicit, append-only, and auditable.

CREATE TABLE application_model_finalizer_recoveries (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    stage_team_plan_id UUID NOT NULL,
    leader_work_item_id UUID NOT NULL,
    worker_run_id UUID NOT NULL,
    deliverable_submission_id UUID NOT NULL,
    source_submission_attempt_epoch BIGINT NOT NULL CHECK (source_submission_attempt_epoch >= 0),
    source_submission_lease_token UUID NOT NULL,
    source_unit_row_version BIGINT NOT NULL CHECK (source_unit_row_version >= 0),
    source_work_item_row_version BIGINT NOT NULL CHECK (source_work_item_row_version >= 0),
    source_attempt_epoch BIGINT NOT NULL CHECK (source_attempt_epoch >= 0),
    source_checkpoint_version BIGINT NOT NULL CHECK (source_checkpoint_version >= 0),
    status TEXT NOT NULL CHECK (status IN ('building','applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    applied_at TIMESTAMPTZ,
    UNIQUE (worker_run_id, source_attempt_epoch, source_checkpoint_version),
    FOREIGN KEY (stage_team_plan_id) REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    FOREIGN KEY (leader_work_item_id) REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    FOREIGN KEY (worker_run_id) REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY (deliverable_submission_id)
        REFERENCES stage_deliverable_submissions(id) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_application_model_finalizer_recovery()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FINALIZER_RECOVERY_IMMUTABLE';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF ROW(
            NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
            NEW.stage_team_plan_id,NEW.leader_work_item_id,NEW.worker_run_id,
            NEW.deliverable_submission_id,NEW.source_submission_attempt_epoch,
            NEW.source_submission_lease_token,NEW.source_unit_row_version,
            NEW.source_work_item_row_version,NEW.source_attempt_epoch,
            NEW.source_checkpoint_version,NEW.created_at
        ) IS DISTINCT FROM ROW(
            OLD.id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
            OLD.stage_team_plan_id,OLD.leader_work_item_id,OLD.worker_run_id,
            OLD.deliverable_submission_id,OLD.source_submission_attempt_epoch,
            OLD.source_submission_lease_token,OLD.source_unit_row_version,
            OLD.source_work_item_row_version,OLD.source_attempt_epoch,
            OLD.source_checkpoint_version,OLD.created_at
        ) OR OLD.status<>'building' OR NEW.status<>'applied'
          OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL THEN
            RAISE EXCEPTION 'APPLICATION_MODEL_FINALIZER_RECOVERY_IMMUTABLE';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FINALIZER_RECOVERY_INVALID_STATE';
    END IF;
    PERFORM 1
      FROM operation_state operation
      JOIN stage_runs execution
        ON execution.id=NEW.stage_execution_id
       AND execution.operation_id=operation.operation_id
       AND execution.stage_kind='application_understanding'
       AND execution.stage_kind=operation.current_stage
       AND execution.status='started'
      JOIN stage_run_units unit
        ON unit.id=NEW.stage_run_unit_id
       AND unit.operation_id=operation.operation_id
       AND unit.stage_execution_id=execution.id
       AND unit.stage_kind='application_understanding'
       AND unit.status='gate_blocked'
       AND unit.row_version=NEW.source_unit_row_version
      JOIN stage_team_plans plan
        ON plan.id=NEW.stage_team_plan_id
       AND plan.operation_id=operation.operation_id
       AND plan.stage_execution_id=execution.id
       AND plan.stage_run_unit_id=unit.id
       AND plan.scope_snapshot_id=unit.scope_snapshot_id
       AND plan.organization_id=unit.organization_id
      JOIN stage_work_items item
        ON item.id=NEW.leader_work_item_id
       AND item.team_plan_id=plan.id
       AND item.operation_id=plan.operation_id
       AND item.stage_execution_id=plan.stage_execution_id
       AND item.stage_run_unit_id=plan.stage_run_unit_id
       AND item.organization_id=plan.organization_id
       AND item.status='exhausted'
       AND item.row_version=NEW.source_work_item_row_version
       AND item.terminal_at IS NOT NULL
      JOIN stage_worker_runs worker
        ON worker.id=NEW.worker_run_id
       AND worker.id=plan.final_submitter_worker_run_id
       AND worker.work_item_id=item.id
       AND worker.operation_id=plan.operation_id
       AND worker.stage_execution_id=plan.stage_execution_id
       AND worker.stage_run_unit_id=plan.stage_run_unit_id
       AND worker.organization_id=plan.organization_id
       AND worker.status='failed'
       AND worker.attempt_epoch=NEW.source_attempt_epoch
       AND worker.checkpoint_version=NEW.source_checkpoint_version
       AND worker.lease_token IS NULL
       AND worker.active_tool_call_id IS NULL
      JOIN stage_deliverable_submissions submission
        ON submission.id=NEW.deliverable_submission_id
       AND submission.operation_id=plan.operation_id
       AND submission.stage_execution_id=plan.stage_execution_id
       AND submission.stage_run_unit_id=plan.stage_run_unit_id
       AND submission.organization_id=plan.organization_id
       AND submission.worker_run_id=worker.id
       AND submission.stage_kind='application_understanding'
       AND submission.attempt_epoch=NEW.source_submission_attempt_epoch
       AND submission.lease_token=NEW.source_submission_lease_token
       AND submission.attempt_epoch<=worker.attempt_epoch
      JOIN tool_calls tool
        ON tool.id=submission.tool_call_record_id
       AND tool.call_id=submission.tool_request_id
       AND tool.name='submit_stage_deliverable'
       AND tool.status='finished'
       AND tool.operation_id=submission.operation_id
       AND tool.stage_execution_id=submission.stage_execution_id
       AND tool.stage_run_unit_id=submission.stage_run_unit_id
       AND tool.worker_run_id=submission.worker_run_id
       AND tool.organization_id=submission.organization_id
       AND tool.attempt_epoch=submission.attempt_epoch
       AND tool.lease_token=submission.lease_token
      JOIN application_model_manifests manifest
        ON manifest.operation_id=plan.operation_id
       AND manifest.scope_snapshot_id=plan.scope_snapshot_id
       AND manifest.stage_execution_id=plan.stage_execution_id
       AND manifest.stage_run_unit_id=plan.stage_run_unit_id
       AND manifest.organization_id=plan.organization_id
     WHERE operation.operation_id=NEW.operation_id
       AND operation.superseded_by IS NULL
       AND operation.runtime_memory_contract='v2_only'
       AND operation.application_model_contract='application_model_v1'
       AND plan.requests_closed_at IS NOT NULL
       AND plan.final_submitter_kind='worker'
       AND plan.aggregator_kind='worker'
       AND plan.aggregator_role=plan.leader_role
       AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
       AND item.stable_key='leader:primary'
       AND item.role=plan.leader_role
       AND item.required_for_barrier=FALSE
       AND worker.message_chain_id IS NOT NULL
       AND worker.checkpoint #>> '{stage_team_execution_failure,code}' IN (
           'application_model_submission_outcome_unknown',
           'application_model_closeout_blocked_before_submission',
           'application_model_closeout_failed_before_submission'
       )
       AND tool.result::jsonb->>'accepted'='true'
       AND (tool.result::jsonb->>'deliverable_submission_id')::UUID=submission.id
       AND NOT EXISTS (
           SELECT 1 FROM application_model_current_revisions current_revision
            WHERE current_revision.manifest_id=manifest.id
       )
       AND NOT EXISTS (
           SELECT 1 FROM stage_handoffs handoff
            WHERE handoff.operation_id=plan.operation_id
              AND handoff.stage_execution_id=plan.stage_execution_id
              AND handoff.source_stage_run_unit_id=plan.stage_run_unit_id
              AND handoff.from_stage_kind='application_understanding'
       )
       AND NOT EXISTS (
           SELECT 1 FROM stage_worker_outputs output
            WHERE output.work_item_id=item.id
       )
       AND NOT EXISTS (
           SELECT 1 FROM stage_worker_runs live
            WHERE live.stage_run_unit_id=unit.id
              AND live.status IN ('queued','running','waiting_background','recovery_required')
       )
       AND NOT EXISTS (
           SELECT 1
             FROM stage_work_items required_item
             LEFT JOIN stage_worker_outputs required_output
               ON required_output.work_item_id=required_item.id
            WHERE required_item.team_plan_id=plan.id
              AND required_item.required_for_barrier
              AND (
                  required_item.status<>'completed'
                  OR required_output.id IS NULL
                  OR required_output.business_disposition='blocked'
              )
       )
     FOR SHARE OF operation,execution,unit,plan,item,worker,submission,tool,manifest;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FINALIZER_RECOVERY_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_finalizer_recoveries_contract
BEFORE INSERT OR UPDATE OR DELETE ON application_model_finalizer_recoveries
FOR EACH ROW EXECUTE FUNCTION enforce_application_model_finalizer_recovery();

CREATE OR REPLACE FUNCTION enforce_stage_work_item_contract()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    controller_turn_resume BOOLEAN := FALSE;
    application_model_finalizer_recovery BOOLEAN := FALSE;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;

    IF TG_OP = 'INSERT' THEN
        SELECT * INTO plan FROM stage_team_plans persisted
         WHERE persisted.id=NEW.team_plan_id
           AND persisted.operation_id=NEW.operation_id
           AND persisted.stage_execution_id=NEW.stage_execution_id
           AND persisted.stage_run_unit_id=NEW.stage_run_unit_id
           AND persisted.scope_snapshot_id=NEW.scope_snapshot_id
           AND persisted.organization_id=NEW.organization_id
         FOR UPDATE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_OWNER_MISMATCH';
        END IF;
        IF plan.requests_closed_at IS NOT NULL OR NEW.dispatch_epoch<>plan.dispatch_epoch THEN
            RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CLOSED';
        END IF;
        IF NOT (plan.allowed_worker_roles ? NEW.role) THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_ROLE_NOT_ALLOWED';
        END IF;
        IF NEW.created_by='gate_repair' AND NOT EXISTS (
            SELECT 1 FROM stage_team_repair_generations generation
             WHERE generation.team_plan_id=plan.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
        ) THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_REPAIR_GENERATION_REQUIRED';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.id, NEW.team_plan_id, NEW.operation_id, NEW.stage_execution_id,
        NEW.stage_run_unit_id, NEW.scope_snapshot_id, NEW.organization_id,
        NEW.dispatch_epoch, NEW.kind, NEW.stable_key, NEW.role,
        NEW.input_manifest_hash, NEW.input_refs, NEW.required_for_barrier,
        NEW.conflict_key, NEW.priority, NEW.attempt_policy, NEW.budget,
        NEW.output_schema, NEW.created_by, NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id, OLD.team_plan_id, OLD.operation_id, OLD.stage_execution_id,
        OLD.stage_run_unit_id, OLD.scope_snapshot_id, OLD.organization_id,
        OLD.dispatch_epoch, OLD.kind, OLD.stable_key, OLD.role,
        OLD.input_manifest_hash, OLD.input_refs, OLD.required_for_barrier,
        OLD.conflict_key, OLD.priority, OLD.attempt_policy, OLD.budget,
        OLD.output_schema, OLD.created_by, OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_VERSION_CAS_REQUIRED';
    END IF;

    controller_turn_resume :=
        OLD.status='superseded'
        AND NEW.status='waiting_dependency'
        AND OLD.terminal_at IS NOT NULL
        AND NEW.terminal_at IS NULL
        AND EXISTS (
            SELECT 1
              FROM stage_team_controller_turn_resumes authority
              JOIN stage_team_plans resumed_plan ON resumed_plan.id=authority.team_plan_id
             WHERE authority.team_plan_id=OLD.team_plan_id
               AND authority.leader_work_item_id=OLD.id
               AND authority.status='building'
               AND authority.source_item_row_version=OLD.row_version
               AND resumed_plan.dispatch_epoch=authority.resume_dispatch_epoch
               AND resumed_plan.requests_closed_at IS NULL
        );
    application_model_finalizer_recovery :=
        OLD.status='exhausted'
        AND NEW.status='queued'
        AND OLD.terminal_at IS NOT NULL
        AND NEW.terminal_at IS NULL
        AND EXISTS (
            SELECT 1
              FROM application_model_finalizer_recoveries authority
             WHERE authority.stage_team_plan_id=OLD.team_plan_id
               AND authority.leader_work_item_id=OLD.id
               AND authority.status='building'
               AND authority.source_work_item_row_version=OLD.row_version
        );
    IF NOT (
        (OLD.status='queued' AND NEW.status IN ('claimed','running','superseded'))
        OR (OLD.status='claimed' AND NEW.status IN ('queued','running','recovery_required','superseded'))
        OR (OLD.status='running' AND NEW.status IN (
            'waiting_dependency','completed','retry_pending','recovery_required',
            'exhausted','superseded'
        ))
        OR (OLD.status='waiting_dependency' AND NEW.status IN (
            'queued','running','recovery_required','superseded'
        ))
        OR (OLD.status='retry_pending' AND NEW.status IN ('queued','exhausted','superseded'))
        OR (OLD.status='recovery_required' AND NEW.status IN (
            'queued','completed','exhausted','superseded'
        ))
        OR controller_turn_resume
        OR application_model_finalizer_recovery
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
