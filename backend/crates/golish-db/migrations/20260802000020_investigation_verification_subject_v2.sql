-- Bind VerificationTask Primary rearm to the same immutable Campaign
-- reservation denominator used by the production analysis host. Campaign ids
-- alone do not bind the plan objective, verification objective, and reservation
-- authority that the Primary is allowed to reason about.

CREATE OR REPLACE FUNCTION enforce_investigation_task_primary_rearm_receipt()
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
                   'verification_task_campaigns.v2',
                   array_agg(campaign.reservation_sha256 ORDER BY campaign.campaign_id)
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
