-- Permit one exact Target Intel finalizer-only response-loss recovery without
-- reopening ordinary exhausted StageTeam work. The Worker is re-leased first
-- in the same transaction and carries a server-authored checkpoint binding the
-- immutable Reviewer PASS and original Controller submission. The WorkItem
-- trigger independently revalidates those witnesses before accepting the one
-- exhausted -> running transition.

CREATE OR REPLACE FUNCTION enforce_stage_work_item_contract()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    controller_turn_resume BOOLEAN := FALSE;
    target_intel_reviewer_insert BOOLEAN := FALSE;
    target_intel_goal_resume BOOLEAN := FALSE;
    target_intel_finalizer_recovery BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
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
        target_intel_reviewer_insert :=
            NEW.created_by='target_intel_review_freeze'
            AND NEW.execution_profile='read_only_reviewer'
            AND NEW.terminal_contract='intel_review_v1'
            AND NEW.kind='target_intel_read_only_review'
            AND NEW.role='intel_goal_reviewer'
            AND NEW.output_schema='intel_review.v1'
            AND NEW.required_for_barrier=FALSE
            AND NEW.dispatch_epoch=plan.dispatch_epoch
            AND plan.stage_kind='target_intel'
            AND plan.requests_closed_at IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM target_intel_goal_review_freeze_authorities authority
                 JOIN target_intel_goal_epochs epoch ON epoch.id=authority.goal_epoch_id
                 WHERE authority.reviewer_work_item_id=NEW.id
                   AND authority.operation_id=NEW.operation_id
                   AND authority.organization_id=NEW.organization_id
                   AND authority.stage_execution_id=NEW.stage_execution_id
                   AND authority.stage_run_unit_id=NEW.stage_run_unit_id
                   AND authority.scope_snapshot_id=NEW.scope_snapshot_id
                   AND authority.team_plan_id=NEW.team_plan_id
                   AND authority.bundle_sha256=NEW.input_manifest_hash
                   AND authority.status='building'
                   AND epoch.status='sealed_for_review'
                   AND (
                       plan.final_submitter_worker_run_id IS NULL
                       OR (
                           plan.final_submitter_worker_run_id=epoch.controller_worker_run_id
                           AND EXISTS (
                               SELECT 1 FROM stage_deliverable_submissions submission
                                WHERE submission.worker_run_id=plan.final_submitter_worker_run_id
                                  AND submission.operation_id=plan.operation_id
                                  AND submission.stage_execution_id=plan.stage_execution_id
                                  AND submission.stage_run_unit_id=plan.stage_run_unit_id
                                  AND submission.organization_id=plan.organization_id
                                  AND submission.stage_kind='target_intel'
                           )
                       )
                   )
            );
        IF (plan.requests_closed_at IS NOT NULL OR NEW.dispatch_epoch<>plan.dispatch_epoch)
            AND NOT target_intel_reviewer_insert
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CLOSED';
        END IF;
        IF NOT (plan.allowed_worker_roles ? NEW.role) AND NOT target_intel_reviewer_insert THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_ROLE_NOT_ALLOWED';
        END IF;
        IF NEW.created_by='target_intel_review_freeze' AND NOT target_intel_reviewer_insert THEN
            RAISE EXCEPTION 'TARGET_INTEL_REVIEWER_FREEZE_AUTHORITY_REQUIRED';
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

    target_intel_goal_resume :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.status IN ('running','waiting_dependency')
        AND NEW.status='waiting_dependency'
        AND NEW.terminal_at IS NOT DISTINCT FROM OLD.terminal_at
        AND EXISTS (
            SELECT 1 FROM target_intel_goal_resume_authorities authority
             WHERE authority.controller_work_item_id=OLD.id
               AND authority.team_plan_id=OLD.team_plan_id
               AND authority.source_item_row_version=OLD.row_version
               AND authority.successor_goal_epoch=NEW.dispatch_epoch
               AND authority.status='building'
        );
    IF ROW(
        NEW.id,NEW.team_plan_id,NEW.operation_id,NEW.stage_execution_id,
        NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
        NEW.kind,NEW.stable_key,NEW.role,NEW.input_manifest_hash,NEW.input_refs,
        NEW.required_for_barrier,NEW.conflict_key,NEW.priority,
        NEW.attempt_policy,NEW.budget,NEW.output_schema,NEW.created_by,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.team_plan_id,OLD.operation_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
        OLD.kind,OLD.stable_key,OLD.role,OLD.input_manifest_hash,OLD.input_refs,
        OLD.required_for_barrier,OLD.conflict_key,OLD.priority,
        OLD.attempt_policy,OLD.budget,OLD.output_schema,OLD.created_by,OLD.created_at
    ) OR (NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch AND NOT target_intel_goal_resume)
    THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_VERSION_CAS_REQUIRED';
    END IF;
    IF target_intel_goal_resume THEN
        RETURN NEW;
    END IF;
    controller_turn_resume :=
        OLD.status='superseded' AND NEW.status='waiting_dependency'
        AND OLD.terminal_at IS NOT NULL AND NEW.terminal_at IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_controller_turn_resumes authority
            JOIN stage_team_plans resumed_plan ON resumed_plan.id=authority.team_plan_id
             WHERE authority.team_plan_id=OLD.team_plan_id
               AND authority.leader_work_item_id=OLD.id
               AND authority.status='building'
               AND authority.source_item_row_version=OLD.row_version
               AND resumed_plan.dispatch_epoch=authority.resume_dispatch_epoch
               AND resumed_plan.requests_closed_at IS NULL
        );
    target_intel_finalizer_recovery :=
        OLD.status='exhausted'
        AND NEW.status='running'
        AND OLD.terminal_at IS NOT NULL
        AND NEW.terminal_at IS NULL
        AND OLD.stable_key='leader:primary'
        AND OLD.created_by='server_seed'
        AND EXISTS (
            SELECT 1
              FROM stage_team_plans recovery_plan
              JOIN stage_worker_runs controller
                ON controller.id=recovery_plan.final_submitter_worker_run_id
               AND controller.work_item_id=OLD.id
               AND controller.status='running'
               AND controller.terminal_at IS NULL
               AND controller.lease_token IS NOT NULL
               AND controller.lease_expires_at>NOW()
               AND controller.active_tool_call_id IS NULL
              JOIN stage_worker_outputs exhausted_output
                ON exhausted_output.team_plan_id=recovery_plan.id
               AND exhausted_output.work_item_id=OLD.id
               AND exhausted_output.worker_run_id=controller.id
               AND exhausted_output.business_disposition='blocked'
               AND exhausted_output.canonical_output->>'kind'='stage_team_attempts_exhausted'
               AND exhausted_output.canonical_output->>'failure_code'='stage_team_worker_lease_expired'
               AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=ANY(exhausted_output.blocker_codes)
              JOIN target_intel_goal_reviews review
                ON review.id=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,review_id}')::uuid
               AND review.team_plan_id=recovery_plan.id
               AND review.operation_id=recovery_plan.operation_id
               AND review.organization_id=recovery_plan.organization_id
               AND review.stage_execution_id=recovery_plan.stage_execution_id
               AND review.stage_run_unit_id=recovery_plan.stage_run_unit_id
               AND review.controller_work_item_id=OLD.id
               AND review.controller_worker_run_id=controller.id
               AND review.controller_message_chain_id=controller.message_chain_id
               AND review.status='pass'
               AND review.verdict->>'decision'='PASS'
               AND review.row_version=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,review_row_version}')::bigint
               AND review.bundle_sha256=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,bundle_sha256}')
               AND review.verdict_sha256=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,verdict_sha256}')
               AND review.operation_contract_sha256=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,operation_contract_sha256}')
              JOIN target_intel_goal_epochs epoch
                ON epoch.id=review.goal_epoch_id
               AND epoch.status='sealed_for_review'
              JOIN stage_work_items reviewer_item
                ON reviewer_item.id=review.reviewer_work_item_id
               AND reviewer_item.team_plan_id=recovery_plan.id
               AND reviewer_item.status='completed'
              JOIN stage_worker_runs reviewer_worker
                ON reviewer_worker.id=review.reviewer_worker_run_id
               AND reviewer_worker.work_item_id=reviewer_item.id
               AND reviewer_worker.status='passed'
              JOIN stage_deliverable_submissions submission
                ON submission.id=(controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,deliverable_submission_id}')::uuid
               AND submission.id=((review.completion_claim->>'completion_claim')::jsonb
                                  ->>'deliverable_submission_id')::uuid
               AND submission.operation_id=recovery_plan.operation_id
               AND submission.stage_execution_id=recovery_plan.stage_execution_id
               AND submission.stage_run_unit_id=recovery_plan.stage_run_unit_id
               AND submission.organization_id=recovery_plan.organization_id
               AND submission.worker_run_id=controller.id
               AND submission.stage_kind='target_intel'
             WHERE recovery_plan.id=OLD.team_plan_id
               AND recovery_plan.stage_kind='target_intel'
               AND recovery_plan.requests_closed_at IS NOT NULL
               AND recovery_plan.final_submitter_kind='worker'
               AND recovery_plan.aggregator_kind='worker'
               AND recovery_plan.aggregator_role=recovery_plan.leader_role
               AND recovery_plan.dynamic_request_policy->>'coordination_mode'='company_controller'
               AND controller.checkpoint #>>
                    '{_runtime_target_intel_finalizer_recovery,schema_version}'='1'
               AND (SELECT COUNT(*) FROM stage_worker_outputs exact_output
                     WHERE exact_output.team_plan_id=recovery_plan.id
                       AND exact_output.work_item_id=OLD.id
                       AND exact_output.worker_run_id=controller.id)=1
        );
    IF NOT (
        (OLD.status='queued' AND NEW.status IN ('claimed','running','superseded'))
        OR (OLD.status='claimed' AND NEW.status IN ('queued','running','recovery_required','superseded'))
        OR (OLD.status='running' AND NEW.status IN ('waiting_dependency','completed','retry_pending','recovery_required','exhausted','superseded'))
        OR (OLD.status='waiting_dependency' AND NEW.status IN ('queued','running','recovery_required','superseded'))
        OR (OLD.status='retry_pending' AND NEW.status IN ('queued','exhausted','superseded'))
        OR (OLD.status='recovery_required' AND NEW.status IN ('queued','completed','exhausted','superseded'))
        OR controller_turn_resume
        OR target_intel_finalizer_recovery
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
