-- Keep same-Turn Gate repair fuel separate from operator-triggered successor
-- Turns. A producer may require three attempts before it can terminalize one
-- exact coverage cell, so a single in-Turn repair must not consume the entire
-- cross-Turn continuation budget. Historical plans intentionally fall back to
-- the hard cap of two because they predate max_controller_turn_resumes.

CREATE OR REPLACE FUNCTION enforce_stage_team_controller_turn_resume_contract()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_IMMUTABLE';
    END IF;

    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'building' THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_MUST_BUILD_FIRST';
        END IF;
        IF NOT EXISTS (
            SELECT 1
              FROM operation_turns prior_turn
              JOIN operation_turns resume_turn
                ON resume_turn.operation_id=prior_turn.operation_id
               AND resume_turn.ordinal=prior_turn.ordinal+1
             WHERE prior_turn.id=NEW.prior_turn_id
               AND resume_turn.id=NEW.resume_turn_id
               AND prior_turn.operation_id=NEW.operation_id
               AND prior_turn.status='interrupted'
               AND resume_turn.status='running'
        ) THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_TURN_MISMATCH';
        END IF;
        IF NOT EXISTS (
            SELECT 1
              FROM stage_team_plans plan
              JOIN operation_state operation
                ON operation.operation_id=plan.operation_id
              JOIN stage_run_units unit ON unit.id=plan.stage_run_unit_id
              JOIN stage_work_items item ON item.id=NEW.leader_work_item_id
              JOIN stage_worker_runs worker ON worker.id=NEW.leader_worker_run_id
              JOIN stage_deliverable_submissions submission
                ON submission.id=NEW.deliverable_submission_id
              LEFT JOIN stage_team_unit_gaps gap ON gap.id=NEW.source_gap_id
             WHERE plan.id=NEW.team_plan_id
               AND plan.operation_id=NEW.operation_id
               AND plan.stage_execution_id=NEW.stage_execution_id
               AND plan.stage_run_unit_id=NEW.stage_run_unit_id
               AND plan.scope_snapshot_id=NEW.scope_snapshot_id
               AND plan.organization_id=NEW.organization_id
               AND plan.dispatch_epoch=NEW.source_dispatch_epoch
               AND plan.row_version=NEW.source_plan_row_version
               AND plan.requests_closed_at IS NOT NULL
               AND plan.final_submitter_worker_run_id=worker.id
               AND operation.runtime_memory_contract::TEXT='v2_only'
               AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
               AND plan.aggregator_kind='worker'
               AND plan.aggregator_role=plan.leader_role
               AND plan.final_submitter_kind='worker'
               AND (
                   SELECT COUNT(*) FROM stage_team_controller_turn_resumes prior_resume
                    WHERE prior_resume.team_plan_id=plan.id
               ) < LEAST(
                   2,
                   GREATEST(
                       0,
                       CASE
                           WHEN (plan.dynamic_request_policy->>'max_controller_turn_resumes')
                               ~ '^-?[0-9]+$'
                           THEN (plan.dynamic_request_policy->>'max_controller_turn_resumes')::BIGINT
                           ELSE 2
                       END
                   )
               )
               AND unit.status='gate_blocked'
               AND unit.row_version=NEW.source_unit_row_version
               AND item.team_plan_id=plan.id
               AND item.stable_key='leader:primary'
               AND item.role=plan.leader_role
               AND item.required_for_barrier=FALSE
               AND item.created_by='server_seed'
               AND item.status='superseded'
               AND item.terminal_at IS NOT NULL
               AND item.row_version=NEW.source_item_row_version
               AND worker.work_item_id=item.id
               AND worker.status='gate_blocked'
               AND worker.attempt_epoch=NEW.source_attempt_epoch
               AND worker.checkpoint_version=NEW.source_checkpoint_version
               AND worker.checkpoint=NEW.source_checkpoint
               AND worker.message_chain_id=NEW.message_chain_id
               AND worker.lease_token IS NULL
               AND worker.active_tool_call_id IS NULL
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,request_id}'=NEW.source_request_id
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,deliverable_submission_id}'=NEW.deliverable_submission_id::text
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,source_manifest_hash}'=NEW.source_manifest_hash
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,gate_decision_hash}'=NEW.source_gate_decision_hash
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,gap_manifest_hash}'=NEW.source_gap_manifest_hash
               AND (worker.checkpoint #>> '{_runtime_stage_team_gate_block,repair_generation}')::INTEGER=NEW.source_repair_generation
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,fuel_exhausted}'='true'
               AND NEW.source_request_id=
                   'stage-team-repair:' || plan.id::TEXT || ':' ||
                   plan.dispatch_epoch::TEXT || ':' || NEW.source_gate_decision_hash
               AND NEW.source_repair_generation=(
                   SELECT COUNT(*)::INTEGER+1
                     FROM stage_team_repair_generations source_generation
                    WHERE source_generation.team_plan_id=plan.id
               )
               AND submission.operation_id=plan.operation_id
               AND submission.stage_execution_id=plan.stage_execution_id
               AND submission.stage_run_unit_id=plan.stage_run_unit_id
               AND submission.organization_id=plan.organization_id
               AND submission.worker_run_id=worker.id
               AND submission.attempt_epoch=worker.attempt_epoch
               AND submission.lease_token=NEW.source_lease_token
               AND (
                   (
                       NEW.source_gap_id IS NOT NULL
                       AND gap.team_plan_id=plan.id
                       AND gap.request_id=NEW.source_request_id
                       AND gap.source_dispatch_epoch=plan.dispatch_epoch
                       AND gap.source_manifest_hash=NEW.source_manifest_hash
                       AND gap.source_attempt_epoch=worker.attempt_epoch
                       AND gap.source_checkpoint_version=worker.checkpoint_version-1
                       AND gap.source_lease_token=NEW.source_lease_token
                       AND gap.source_aggregator_work_item_id=item.id
                       AND gap.source_aggregator_worker_run_id=worker.id
                       AND gap.deliverable_submission_id=submission.id
                       AND gap.gate_decision_hash=NEW.source_gate_decision_hash
                       AND gap.gap_manifest_hash=NEW.source_gap_manifest_hash
                       AND gap.repair_generation=NEW.source_repair_generation
                       AND gap.disposition='fuel_exhausted'
                   )
                   OR (
                       NEW.source_gap_id IS NULL
                       AND gap.id IS NULL
                       AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,schema_version}'='1'
                       AND worker.legacy_controller_gap_checkpoint_hash=
                           NEW.source_checkpoint_hash
                       AND NOT EXISTS (
                           SELECT 1 FROM stage_team_unit_gaps persisted_gap
                            WHERE persisted_gap.request_id=NEW.source_request_id
                       )
                   )
               )
        ) THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_SOURCE_MISMATCH';
        END IF;
        IF NEW.source_checkpoint_hash <>
            'sha256:' || attack_fact_delta_sha256_jsonb(NEW.source_checkpoint)
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_CHECKPOINT_HASH_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.id, NEW.operation_id, NEW.prior_turn_id, NEW.resume_turn_id,
        NEW.team_plan_id, NEW.source_gap_id, NEW.source_request_id,
        NEW.deliverable_submission_id, NEW.source_lease_token,
        NEW.source_manifest_hash, NEW.source_gate_decision_hash,
        NEW.source_gap_manifest_hash, NEW.source_repair_generation,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id, NEW.scope_snapshot_id, NEW.organization_id,
        NEW.leader_work_item_id, NEW.leader_worker_run_id, NEW.message_chain_id,
        NEW.source_dispatch_epoch, NEW.resume_dispatch_epoch,
        NEW.source_plan_row_version, NEW.source_unit_row_version,
        NEW.source_item_row_version, NEW.source_attempt_epoch,
        NEW.source_checkpoint_version, NEW.source_checkpoint,
        NEW.source_checkpoint_hash, NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id, OLD.operation_id, OLD.prior_turn_id, OLD.resume_turn_id,
        OLD.team_plan_id, OLD.source_gap_id, OLD.source_request_id,
        OLD.deliverable_submission_id, OLD.source_lease_token,
        OLD.source_manifest_hash, OLD.source_gate_decision_hash,
        OLD.source_gap_manifest_hash, OLD.source_repair_generation,
        OLD.stage_execution_id,
        OLD.stage_run_unit_id, OLD.scope_snapshot_id, OLD.organization_id,
        OLD.leader_work_item_id, OLD.leader_worker_run_id, OLD.message_chain_id,
        OLD.source_dispatch_epoch, OLD.resume_dispatch_epoch,
        OLD.source_plan_row_version, OLD.source_unit_row_version,
        OLD.source_item_row_version, OLD.source_attempt_epoch,
        OLD.source_checkpoint_version, OLD.source_checkpoint,
        OLD.source_checkpoint_hash, OLD.created_at
    ) OR OLD.status <> 'building' OR NEW.status <> 'applied'
       OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_IMMUTABLE';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM stage_team_plans plan
          JOIN stage_run_units unit ON unit.id=plan.stage_run_unit_id
          JOIN stage_work_items item ON item.id=NEW.leader_work_item_id
          JOIN stage_worker_runs worker ON worker.id=NEW.leader_worker_run_id
          LEFT JOIN stage_team_unit_gaps gap ON gap.id=NEW.source_gap_id
         WHERE plan.id=NEW.team_plan_id
           AND plan.dispatch_epoch=NEW.resume_dispatch_epoch
           AND plan.row_version=NEW.source_plan_row_version+1
           AND plan.requests_closed_at IS NULL
           AND plan.final_submitter_worker_run_id IS NULL
           AND unit.status='running'
           AND unit.row_version=NEW.source_unit_row_version+1
           AND item.status='waiting_dependency'
           AND item.terminal_at IS NULL
           AND item.row_version=NEW.source_item_row_version+1
           AND worker.status='waiting_background'
           AND worker.attempt_epoch=NEW.source_attempt_epoch
           AND worker.checkpoint_version=NEW.source_checkpoint_version+1
           AND worker.message_chain_id=NEW.message_chain_id
           AND worker.lease_token IS NULL
           AND worker.active_tool_call_id IS NULL
           AND worker.terminal_at IS NULL
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,authority_id}'=NEW.id::text
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,prior_turn_id}'=NEW.prior_turn_id::text
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,resume_turn_id}'=NEW.resume_turn_id::text
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gap_id}'
               IS NOT DISTINCT FROM NEW.source_gap_id::text
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gap_manifest_hash}'=NEW.source_gap_manifest_hash
           AND worker.checkpoint #> '{_runtime_stage_team_turn_resume,source_gap_manifest}'
               IS NOT DISTINCT FROM gap.gap_manifest
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gate_decision_hash}'=NEW.source_gate_decision_hash
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_request_id}'=NEW.source_request_id
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
