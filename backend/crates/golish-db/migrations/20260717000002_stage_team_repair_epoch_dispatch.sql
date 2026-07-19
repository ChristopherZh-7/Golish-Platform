-- Allow one stable Company Controller WorkItem to parent requests in a later
-- server-authorized repair/successor epoch. Request and accepted child rows
-- remain bound to the current open TeamPlan epoch.

ALTER TABLE stage_worker_requests
    DROP CONSTRAINT IF EXISTS stage_worker_requests_parent_work_item_id_team_plan_id_ope_fkey;

ALTER TABLE stage_worker_requests
    ADD CONSTRAINT stage_worker_requests_parent_work_item_owner_fk
    FOREIGN KEY (
        parent_work_item_id,
        team_plan_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES stage_work_items(
        id,
        team_plan_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT;

CREATE OR REPLACE FUNCTION enforce_stage_worker_request_contract()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    parent_item stage_work_items%ROWTYPE;
    cross_epoch_authorized BOOLEAN;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'STAGE_WORKER_REQUEST_IMMUTABLE';
    END IF;
    SELECT * INTO plan
      FROM stage_team_plans AS persisted
     WHERE persisted.id = NEW.team_plan_id
       AND persisted.operation_id = NEW.operation_id
       AND persisted.stage_execution_id = NEW.stage_execution_id
       AND persisted.stage_run_unit_id = NEW.stage_run_unit_id
       AND persisted.scope_snapshot_id = NEW.scope_snapshot_id
       AND persisted.organization_id = NEW.organization_id
     FOR UPDATE;
    IF NOT FOUND OR NEW.dispatch_epoch <> plan.dispatch_epoch THEN
        RAISE EXCEPTION 'STAGE_WORKER_REQUEST_OWNER_OR_EPOCH_MISMATCH';
    END IF;

    SELECT * INTO parent_item
      FROM stage_work_items AS item
     WHERE item.id = NEW.parent_work_item_id
       AND item.team_plan_id = NEW.team_plan_id
       AND item.operation_id = NEW.operation_id
       AND item.stage_execution_id = NEW.stage_execution_id
       AND item.stage_run_unit_id = NEW.stage_run_unit_id
       AND item.scope_snapshot_id = NEW.scope_snapshot_id
       AND item.organization_id = NEW.organization_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'STAGE_WORKER_REQUEST_PARENT_OWNER_MISMATCH';
    END IF;

    IF parent_item.dispatch_epoch <> NEW.dispatch_epoch THEN
        cross_epoch_authorized :=
            plan.dynamic_request_policy->>'coordination_mode'='company_controller'
            AND parent_item.stable_key='leader:primary'
            AND parent_item.role=plan.leader_role
            AND plan.aggregator_role=parent_item.role
            AND parent_item.required_for_barrier=FALSE
            AND parent_item.status='running'
            AND EXISTS (
                SELECT 1
                  FROM stage_worker_runs AS worker
                 WHERE worker.id=NEW.parent_worker_run_id
                   AND worker.work_item_id=parent_item.id
                   AND worker.operation_id=NEW.operation_id
                   AND worker.stage_execution_id=NEW.stage_execution_id
                   AND worker.stage_run_unit_id=NEW.stage_run_unit_id
                   AND worker.organization_id=NEW.organization_id
                   AND worker.status='running'
            )
            AND (
                EXISTS (
                    SELECT 1
                      FROM stage_team_repair_generations AS generation
                     WHERE generation.team_plan_id=plan.id
                       AND generation.operation_id=plan.operation_id
                       AND generation.stage_execution_id=plan.stage_execution_id
                       AND generation.stage_run_unit_id=plan.stage_run_unit_id
                       AND generation.scope_snapshot_id=plan.scope_snapshot_id
                       AND generation.organization_id=plan.organization_id
                       AND generation.dispatch_epoch=NEW.dispatch_epoch
                       AND generation.status IN ('building','sealed')
                       AND generation.manifest->>'kind'='company_controller_gate_reopen'
                       AND generation.manifest->>'leader_work_item_id'=
                           parent_item.id::TEXT
                       AND generation.manifest->>'leader_worker_run_id'=
                           NEW.parent_worker_run_id::TEXT
                )
                OR EXISTS (
                    SELECT 1
                      FROM stage_team_controller_turn_resumes AS resume
                     WHERE resume.team_plan_id=plan.id
                       AND resume.operation_id=plan.operation_id
                       AND resume.stage_execution_id=plan.stage_execution_id
                       AND resume.stage_run_unit_id=plan.stage_run_unit_id
                       AND resume.scope_snapshot_id=plan.scope_snapshot_id
                       AND resume.organization_id=plan.organization_id
                       AND resume.resume_dispatch_epoch=NEW.dispatch_epoch
                       AND resume.leader_work_item_id=parent_item.id
                       AND resume.leader_worker_run_id=NEW.parent_worker_run_id
                       AND resume.status='applied'
                )
            );
        IF NOT cross_epoch_authorized THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_PARENT_EPOCH_NOT_AUTHORIZED';
        END IF;
    END IF;

    IF NEW.status = 'accepted' THEN
        IF plan.requests_closed_at IS NOT NULL THEN
            RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CLOSED';
        END IF;
        IF NOT plan.dynamic_requests_allowed THEN
            RAISE EXCEPTION 'STAGE_TEAM_DYNAMIC_REQUESTS_DISABLED';
        END IF;
        IF NOT (plan.allowed_worker_roles ? NEW.requested_role) THEN
            RAISE EXCEPTION 'STAGE_WORKER_REQUEST_ROLE_NOT_ALLOWED';
        END IF;
        IF NOT EXISTS (
            SELECT 1
              FROM stage_work_items AS item
             WHERE item.id = NEW.accepted_work_item_id
               AND item.team_plan_id = NEW.team_plan_id
               AND item.operation_id = NEW.operation_id
               AND item.stage_execution_id = NEW.stage_execution_id
               AND item.stage_run_unit_id = NEW.stage_run_unit_id
               AND item.scope_snapshot_id = NEW.scope_snapshot_id
               AND item.organization_id = NEW.organization_id
               AND item.dispatch_epoch = NEW.dispatch_epoch
               AND item.role = NEW.requested_role
               AND item.kind = NEW.request_kind
               AND item.output_schema = NEW.expected_output_schema
               AND item.created_by = 'accepted_worker_request'
        ) THEN
            RAISE EXCEPTION 'STAGE_WORKER_REQUEST_ACCEPTED_ITEM_MISMATCH';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
