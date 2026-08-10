-- Campaign reservations become runnable only after an exact same-id Campaign,
-- sealed capability assessment set, and Wave authority exist. Bind Primary
-- rearm to that materialized authority and make start-run replay validate only
-- immutable identity (change_seq is mutable run state, not start input).

CREATE OR REPLACE FUNCTION unified_investigation_campaign_authority_sha256_v4(
    p_campaign_id UUID,
    p_reservation_sha256 TEXT
)
RETURNS TEXT
LANGUAGE SQL
STABLE
PARALLEL SAFE
AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'contract_version','unified-investigation-campaign-authority.v4',
        'reservation_sha256',p_reservation_sha256,
        'campaign_id',campaign.campaign_id,
        'operation_id',campaign.operation_id,
        'project_scope_id',campaign.project_scope_id,
        'organization_id',campaign.organization_id,
        'hypothesis_revision_id',campaign.hypothesis_revision_id,
        'verification_plan_id',campaign.verification_plan_id,
        'verification_plan_hash',campaign.verification_plan_hash,
        'plan_objective_id',campaign.plan_objective_id,
        'verification_objective_id',campaign.verification_objective_id,
        'verification_contract_id',campaign.verification_contract_id,
        'verification_contract_hash',campaign.verification_contract_hash,
        'capability_assessment_set_seal_id',campaign.capability_assessment_set_seal_id,
        'capability_assessment_member_set_hash',assessment_set.member_set_hash,
        'capability_assessment_policy_snapshot_hash',assessment_set.policy_snapshot_hash,
        'capability_assessment_source_snapshot_hash',assessment_set.source_snapshot_hash,
        'capability_assessment_registry_contract_hash',assessment_set.registry_contract_hash,
        'wave_denominator_id',campaign.wave_denominator_id,
        'wave_generation_seal_id',wave.generation_seal_id,
        'wave_contract_version',wave.contract_version,
        'wave_source_snapshot_hash',wave.source_snapshot_hash,
        'wave_member_set_hash',wave.member_set_hash,
        'tool_truth_authority_bundle_seal_id',campaign.tool_truth_authority_bundle_seal_id,
        'relevant_root_set_hash',campaign.relevant_root_set_hash,
        'authority_member_set_hash',campaign.authority_member_set_hash,
        'semantic_authority_bundle_hash',campaign.semantic_authority_bundle_hash,
        'freshness_attestation_bundle_hash',campaign.freshness_attestation_bundle_hash,
        'temporal_validity_bundle_hash',campaign.temporal_validity_bundle_hash,
        'temporal_validity_policy_set_hash',bundle.temporal_validity_policy_set_hash,
        'target_state_epoch_set_hash',bundle.target_state_epoch_set_hash,
        'source_snapshot_hash',campaign.source_snapshot_hash,
        'campaign_version',campaign.campaign_version,
        'effective_valid_until',campaign.effective_valid_until
    )::TEXT)
      FROM verification_campaigns campaign
      JOIN verification_capability_assessment_set_seals assessment_set
        ON assessment_set.assessment_set_seal_id=campaign.capability_assessment_set_seal_id
       AND assessment_set.sealed_at IS NOT NULL
      JOIN verification_wave_coverage_denominators wave
        ON wave.wave_denominator_id=campaign.wave_denominator_id
       AND wave.sealed_at IS NOT NULL
      JOIN tool_truth_authority_bundle_seals bundle
        ON bundle.id=campaign.tool_truth_authority_bundle_seal_id
       AND bundle.operation_id=campaign.operation_id
       AND bundle.organization_id=campaign.organization_id
       AND bundle.consumer_kind='verification_campaign'
       AND bundle.sealed_at IS NOT NULL
     WHERE campaign.campaign_id=p_campaign_id;
$$;

CREATE OR REPLACE FUNCTION unified_investigation_verification_campaign_denominator_v4(
    p_task_id UUID,
    p_assignment_set_id UUID
)
RETURNS TEXT
LANGUAGE SQL
STABLE
PARALLEL SAFE
AS $$
    WITH authority AS (
        SELECT reservation.campaign_id,
               unified_investigation_campaign_authority_sha256_v4(
                   reservation.campaign_id,reservation.reservation_sha256
               ) AS authority_sha256
          FROM hypothesis_verification_task_campaigns reservation
         WHERE reservation.task_id=p_task_id
           AND reservation.assignment_set_id=p_assignment_set_id
    )
    SELECT CASE
        WHEN COUNT(*)>0 AND COUNT(authority_sha256)=COUNT(*)
        THEN unified_investigation_exact_set_hash(
                 'verification_task_campaigns.v4',
                 array_agg(authority_sha256 ORDER BY campaign_id)
             )
        ELSE NULL
    END
      FROM authority;
$$;

CREATE OR REPLACE FUNCTION register_investigation_run_v1(
    p_authority_id UUID,
    p_stable_start_request_id UUID,
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_owning_stage_run_request_id TEXT,
    p_scope_snapshot_id UUID,
    p_initial_change_seq BIGINT
)
RETURNS investigation_run_heads
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_run_heads%ROWTYPE;
    result investigation_run_heads%ROWTYPE;
BEGIN
    SELECT * INTO existing FROM investigation_run_heads
     WHERE stable_start_request_id=p_stable_start_request_id;
    IF FOUND THEN
        IF ROW(existing.authority_id,existing.operation_id,existing.stage_execution_id,
               existing.owning_stage_run_request_id,existing.scope_snapshot_id)
           IS DISTINCT FROM
           ROW(p_authority_id,p_operation_id,p_stage_execution_id,
               p_owning_stage_run_request_id,p_scope_snapshot_id)
           OR p_initial_change_seq<>0
        THEN
            RAISE EXCEPTION 'INVESTIGATION_RUN_START_REPLAY_MISMATCH' USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;
    IF p_initial_change_seq<0 THEN
        RAISE EXCEPTION 'INVESTIGATION_RUN_CHANGE_SEQ_INVALID' USING ERRCODE='23514';
    END IF;
    PERFORM set_config('golish.investigation_run_head_write','on',TRUE);
    INSERT INTO investigation_run_heads(
        authority_id,stable_start_request_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
        stop_epoch,change_seq,head_version,head_sha256
    ) VALUES(
        p_authority_id,p_stable_start_request_id,p_operation_id,p_stage_execution_id,
        p_owning_stage_run_request_id,p_scope_snapshot_id,'running',TRUE,
        0,p_initial_change_seq,0,
        unified_investigation_runtime_head_sha256(
            p_authority_id,'running',TRUE,0,p_initial_change_seq,0
        )
    ) RETURNING * INTO result;
    PERFORM set_config('golish.investigation_run_head_write','off',TRUE);
    RETURN result;
END;
$$;

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
               unified_investigation_verification_campaign_denominator_v4(
                   task.task_id,assignment.assignment_set_id
               )
          INTO STRICT revision_sha256,verification_plan_sha256,assignment_sha256,
                      semantic_attempt_fingerprint,campaign_denominator_sha256
          FROM hypothesis_verification_tasks task
          JOIN hypothesis_verification_task_assignment_sets assignment
            ON assignment.task_id=task.task_id AND assignment.status='sealed'
          JOIN hypothesis_verification_task_campaigns reservation
            ON reservation.task_id=task.task_id
           AND reservation.assignment_set_id=assignment.assignment_set_id
          JOIN verification_campaigns campaign
            ON campaign.campaign_id=reservation.campaign_id
           AND campaign.operation_id=task.operation_id
           AND campaign.organization_id=task.organization_id
           AND campaign.state IN ('admitted','running')
           AND campaign.terminal_at IS NULL
           AND campaign.superseded_at IS NULL
           AND campaign.effective_valid_until>statement_timestamp()
          JOIN verification_capability_assessment_set_seals assessment_set
            ON assessment_set.assessment_set_seal_id=campaign.capability_assessment_set_seal_id
           AND assessment_set.sealed_at IS NOT NULL
         WHERE task.task_id=NEW.verification_task_id
           AND task.operation_id=NEW.operation_id
           AND task.stage_execution_id=NEW.stage_execution_id
           AND task.stage_run_unit_id=NEW.stage_run_unit_id
           AND task.scope_snapshot_id=NEW.scope_snapshot_id
           AND task.organization_id=NEW.organization_id
         GROUP BY task.task_id,assignment.assignment_set_id,
                  task.hypothesis_revision_sha256,task.verification_plan_sha256,
                  assignment.member_set_sha256,task.semantic_attempt_fingerprint
        HAVING COUNT(*)=(
            SELECT COUNT(*)
              FROM hypothesis_verification_task_campaigns expected
             WHERE expected.task_id=task.task_id
               AND expected.assignment_set_id=assignment.assignment_set_id
        );
        expected_subject_fingerprint := tool_truth_sha256(jsonb_build_object(
            'task_id',NEW.verification_task_id,
            'revision_sha256',revision_sha256,
            'plan_sha256',verification_plan_sha256,
            'assignment_sha256',assignment_sha256,
            'campaign_denominator_sha256',campaign_denominator_sha256,
            'semantic_attempt_fingerprint',semantic_attempt_fingerprint
        )::TEXT);
        IF campaign_denominator_sha256 IS NULL
           OR expected_subject_fingerprint<>NEW.subject_fingerprint_sha256 THEN
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
