-- Durable successor-Analysis rearm for an exact open Verification evolution
-- authority.  The receipt is the sole narrow authority that can reopen the
-- closed Investigation StageTeam epoch for an Evolution Analysis Primary.

CREATE FUNCTION investigation_evolution_analysis_subject_fingerprint(
    requested_pending_evolution_authority_id UUID,
    requested_attempt_input_hash TEXT
) RETURNS TEXT
LANGUAGE SQL
STABLE
STRICT
AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'domain','investigation_analysis_subject_fingerprint.v3',
        'pending_evolution_authority_id',pending.pending_evolution_authority_id,
        'consolidation_batch_id',pending.consolidation_batch_id,
        'source_generation_id',pending.source_generation_id,
        'source_wave_denominator_id',pending.source_wave_denominator_id,
        'wave_coverage_receipt_id',pending.wave_coverage_receipt_id,
        'fact_delta_member_count',pending.fact_delta_member_count,
        'applied_fact_delta_set_hash',pending.applied_fact_delta_set_hash,
        'residual_set_hash',pending.residual_set_hash,
        'source_snapshot_hash',pending.source_snapshot_hash,
        'attempt_input_hash',requested_attempt_input_hash
    )::TEXT)
      FROM hypothesis_pending_evolution_authorities pending
     WHERE pending.pending_evolution_authority_id=
           requested_pending_evolution_authority_id
$$;

CREATE TABLE investigation_evolution_analysis_primary_rearms (
    rearm_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    pending_evolution_authority_id UUID NOT NULL UNIQUE,
    stage_team_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_generation_id UUID NOT NULL,
    subject_fingerprint_sha256 TEXT NOT NULL
        CHECK(subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_dispatch_epoch BIGINT NOT NULL CHECK(source_dispatch_epoch>=0),
    resume_dispatch_epoch BIGINT NOT NULL
        CHECK(resume_dispatch_epoch=source_dispatch_epoch+1),
    source_plan_row_version BIGINT NOT NULL CHECK(source_plan_row_version>=0),
    previous_analysis_primary_work_item_id UUID NOT NULL,
    previous_analysis_primary_worker_run_id UUID NOT NULL,
    previous_analysis_primary_message_chain_id UUID NOT NULL,
    primary_work_item_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL UNIQUE,
    primary_message_chain_id UUID NOT NULL UNIQUE,
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
    FOREIGN KEY(pending_evolution_authority_id)
        REFERENCES hypothesis_pending_evolution_authorities(
            pending_evolution_authority_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(previous_analysis_primary_work_item_id)
        REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    FOREIGN KEY(previous_analysis_primary_worker_run_id)
        REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY(previous_analysis_primary_message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT,
    FOREIGN KEY(primary_message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

-- The durable runtime specialist identifies the successor-Analysis actor,
-- while the WorkItem role remains the StageTeam's coordinator role.  Permit
-- that one distinction only behind the exact building rearm receipt.
CREATE OR REPLACE FUNCTION enforce_stage_worker_work_item_contract()
RETURNS trigger AS $$
DECLARE
    item stage_work_items%ROWTYPE;
    evolution_primary_exact BOOLEAN := FALSE;
BEGIN
    IF EXISTS (
        SELECT 1 FROM stage_team_plans AS plan
         WHERE plan.stage_run_unit_id=NEW.stage_run_unit_id
    ) AND NEW.work_item_id IS NULL THEN
        RAISE EXCEPTION 'STAGE_TEAM_WORKER_REQUIRES_WORK_ITEM';
    END IF;
    IF NEW.work_item_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT * INTO item
      FROM stage_work_items AS persisted
     WHERE persisted.id=NEW.work_item_id
       AND persisted.operation_id=NEW.operation_id
       AND persisted.stage_execution_id=NEW.stage_execution_id
       AND persisted.stage_run_unit_id=NEW.stage_run_unit_id
       AND persisted.organization_id=NEW.organization_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'STAGE_TEAM_WORKER_OWNER_MISMATCH';
    END IF;
    evolution_primary_exact :=
        NEW.specialist='investigation_evolution_primary'
        AND item.kind='investigation_primary'
        AND EXISTS(
            SELECT 1
              FROM investigation_evolution_analysis_primary_rearms rearm
              JOIN stage_team_plans plan ON plan.id=rearm.stage_team_plan_id
             WHERE rearm.primary_work_item_id=item.id
               AND rearm.primary_worker_run_id=NEW.id
               AND rearm.stage_team_plan_id=item.team_plan_id
               AND rearm.operation_id=NEW.operation_id
               AND rearm.stage_execution_id=NEW.stage_execution_id
               AND rearm.stage_run_unit_id=NEW.stage_run_unit_id
               AND rearm.organization_id=NEW.organization_id
               AND rearm.resume_dispatch_epoch=item.dispatch_epoch
               AND rearm.subject_fingerprint_sha256=item.input_manifest_hash
               AND rearm.status='building'
               AND item.role=plan.leader_role
               AND plan.aggregator_role=item.role
        );
    IF NEW.work_item_kind<>item.kind
       OR NEW.work_item_key<>item.stable_key
       OR (NEW.specialist<>item.role AND NOT evolution_primary_exact)
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_WORKER_WORK_ITEM_IDENTITY_MISMATCH';
    END IF;
    IF TG_OP='UPDATE'
       AND OLD.work_item_id IS NOT NULL
       AND NEW.work_item_id IS DISTINCT FROM OLD.work_item_id
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_WORKER_WORK_ITEM_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION enforce_investigation_evolution_analysis_primary_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    pending hypothesis_pending_evolution_authorities%ROWTYPE;
    expected_attempt_input_hash TEXT;
    expected_subject_fingerprint TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_EVOLUTION_ANALYSIS_PRIMARY_REARM_APPEND_ONLY';
    END IF;
    SELECT * INTO STRICT plan FROM stage_team_plans
     WHERE id=NEW.stage_team_plan_id FOR SHARE;
    SELECT * INTO STRICT pending FROM hypothesis_pending_evolution_authorities
     WHERE pending_evolution_authority_id=NEW.pending_evolution_authority_id FOR SHARE;
    SELECT attempt.attempt_input_hash INTO STRICT expected_attempt_input_hash
      FROM hypothesis_generation_seals source_seal
      JOIN LATERAL(
           SELECT snapshot.snapshot_id
             FROM candidate_analysis_snapshots snapshot
            WHERE snapshot.operation_id=NEW.operation_id
              AND snapshot.organization_id=NEW.organization_id
              AND snapshot.scope_snapshot_id=NEW.scope_snapshot_id
              AND snapshot.previous_generation_seal_id=source_seal.seal_id
              AND snapshot.snapshot_status='sealed_ready'
            ORDER BY snapshot.wave_ordinal DESC,snapshot.created_at DESC,
                     snapshot.snapshot_id DESC
            LIMIT 1
      ) latest_snapshot ON TRUE
      JOIN candidate_analysis_attempts attempt
        ON attempt.snapshot_id=latest_snapshot.snapshot_id
       AND attempt.operation_id=NEW.operation_id
       AND attempt.organization_id=NEW.organization_id
       AND attempt.attempt_ordinal=0
     WHERE source_seal.generation_id=pending.source_generation_id;
    expected_subject_fingerprint := investigation_evolution_analysis_subject_fingerprint(
        NEW.pending_evolution_authority_id,
        expected_attempt_input_hash
    );
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
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
           OR pending.operation_id<>NEW.operation_id
           OR pending.project_scope_id<>NEW.project_scope_id
           OR pending.organization_id<>NEW.organization_id
           OR pending.source_generation_id<>NEW.source_generation_id
           OR expected_subject_fingerprint<>NEW.subject_fingerprint_sha256
           OR NEW.rearm_receipt_id<>uuid_generate_v5(
                NEW.pending_evolution_authority_id,
                'investigation-evolution-analysis-primary-rearm-receipt-v1')
           OR NEW.primary_work_item_id<>uuid_generate_v5(
                NEW.pending_evolution_authority_id,
                'investigation-evolution-analysis-primary-work-item-v1')
           OR NEW.primary_worker_run_id<>uuid_generate_v5(
                NEW.pending_evolution_authority_id,
                'investigation-evolution-analysis-primary-worker-v1')
           OR NEW.primary_message_chain_id<>uuid_generate_v5(
                NEW.pending_evolution_authority_id,
                'investigation-evolution-analysis-primary-chain-v1')
           OR EXISTS(
                SELECT 1 FROM hypothesis_consolidation_receipts terminal
                 WHERE terminal.consolidation_batch_id=pending.consolidation_batch_id)
           OR NOT EXISTS(
                SELECT 1
                  FROM operation_state operation
                  JOIN hypothesis_generations generation
                    ON generation.generation_id=pending.source_generation_id
                   AND generation.operation_id=pending.operation_id
                   AND generation.organization_id=pending.organization_id
                  JOIN hypothesis_generation_seals generation_seal
                    ON generation_seal.generation_id=generation.generation_id
                 WHERE operation.operation_id=pending.operation_id
                   AND operation.project_scope_id=pending.project_scope_id)
           OR NOT EXISTS(
                SELECT 1
                  FROM stage_work_items source_item
                  JOIN stage_worker_runs source_worker
                    ON source_worker.id=NEW.previous_analysis_primary_worker_run_id
                   AND source_worker.work_item_id=source_item.id
                  JOIN message_chains source_chain
                    ON source_chain.id=NEW.previous_analysis_primary_message_chain_id
                   AND source_chain.id=source_worker.message_chain_id
                   AND source_chain.task_id=NEW.operation_id
                 WHERE source_item.id=NEW.previous_analysis_primary_work_item_id
                   AND source_item.team_plan_id=NEW.stage_team_plan_id
                   AND source_item.operation_id=NEW.operation_id
                   AND source_item.stage_execution_id=NEW.stage_execution_id
                   AND source_item.stage_run_unit_id=NEW.stage_run_unit_id
                   AND source_item.scope_snapshot_id=NEW.scope_snapshot_id
                   AND source_item.organization_id=NEW.organization_id
                   AND source_item.role=plan.leader_role
                   AND NOT source_item.required_for_barrier
                   AND source_item.status='completed'
                   AND source_worker.status='passed'
                   AND source_worker.organization_id=NEW.organization_id
                   AND (
                        EXISTS(
                            SELECT 1
                              FROM investigation_evolution_analysis_primary_rearms prior
                             WHERE prior.primary_work_item_id=source_item.id
                               AND prior.primary_worker_run_id=source_worker.id
                               AND prior.primary_message_chain_id=source_chain.id
                               AND prior.stage_team_plan_id=NEW.stage_team_plan_id
                               AND prior.operation_id=NEW.operation_id
                               AND prior.organization_id=NEW.organization_id
                               AND prior.status='applied')
                        OR (
                            (source_item.stable_key='leader:primary'
                             OR source_item.stable_key LIKE 'leader:synthesis-recovery:%')
                            AND EXISTS(
                                SELECT 1
                                  FROM investigation_pentagi_task_plans task_plan
                                  JOIN pentagi_logical_dispatch_receipts dispatch
                                    ON dispatch.task_plan_id=task_plan.task_plan_id
                                   AND dispatch.actor_kind='primary'
                                   AND dispatch.subtask_id IS NULL
                                   AND dispatch.stage_work_item_id=source_item.id
                                   AND dispatch.worker_run_id=source_worker.id
                                 WHERE task_plan.stage_team_plan_id=NEW.stage_team_plan_id
                                   AND task_plan.operation_id=NEW.operation_id
                                   AND task_plan.stage_execution_id=NEW.stage_execution_id
                                   AND task_plan.stage_run_unit_id=NEW.stage_run_unit_id
                                   AND task_plan.organization_id=NEW.organization_id
                                   AND task_plan.subject_kind='analysis_attempt'
                                   AND task_plan.status IN('open','sealed'))
                        )
                   )
           )
           OR EXISTS(SELECT 1 FROM stage_work_items WHERE id=NEW.primary_work_item_id)
           OR EXISTS(SELECT 1 FROM stage_worker_runs WHERE id=NEW.primary_worker_run_id)
           OR EXISTS(SELECT 1 FROM message_chains WHERE id=NEW.primary_message_chain_id)
        THEN
            RAISE EXCEPTION 'INVESTIGATION_EVOLUTION_ANALYSIS_PRIMARY_REARM_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.status<>'building' OR NEW.status<>'applied' OR NEW.applied_at IS NULL
       OR ROW(NEW.rearm_receipt_id,NEW.stable_request_id,
              NEW.pending_evolution_authority_id,NEW.stage_team_plan_id,
              NEW.operation_id,NEW.project_scope_id,NEW.stage_execution_id,
              NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
              NEW.source_generation_id,NEW.subject_fingerprint_sha256,
              NEW.source_dispatch_epoch,NEW.resume_dispatch_epoch,
              NEW.source_plan_row_version,
              NEW.previous_analysis_primary_work_item_id,
              NEW.previous_analysis_primary_worker_run_id,
              NEW.previous_analysis_primary_message_chain_id,
              NEW.primary_work_item_id,NEW.primary_worker_run_id,
              NEW.primary_message_chain_id,NEW.receipt_sha256,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.rearm_receipt_id,OLD.stable_request_id,
              OLD.pending_evolution_authority_id,OLD.stage_team_plan_id,
              OLD.operation_id,OLD.project_scope_id,OLD.stage_execution_id,
              OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
              OLD.source_generation_id,OLD.subject_fingerprint_sha256,
              OLD.source_dispatch_epoch,OLD.resume_dispatch_epoch,
              OLD.source_plan_row_version,
              OLD.previous_analysis_primary_work_item_id,
              OLD.previous_analysis_primary_worker_run_id,
              OLD.previous_analysis_primary_message_chain_id,
              OLD.primary_work_item_id,OLD.primary_worker_run_id,
              OLD.primary_message_chain_id,OLD.receipt_sha256,OLD.created_at)
       OR NOT EXISTS(
            SELECT 1 FROM stage_team_plans current_plan
             WHERE current_plan.id=NEW.stage_team_plan_id
               AND current_plan.dispatch_epoch=NEW.resume_dispatch_epoch
               AND current_plan.row_version=NEW.source_plan_row_version+1
               AND current_plan.requests_closed_at IS NULL)
       OR NOT EXISTS(
            SELECT 1 FROM stage_work_items item
             WHERE item.id=NEW.primary_work_item_id
               AND item.team_plan_id=NEW.stage_team_plan_id
               AND item.dispatch_epoch=NEW.resume_dispatch_epoch
               AND item.kind='investigation_primary'
               AND item.stable_key='evolution:' ||
                    NEW.pending_evolution_authority_id::TEXT || ':primary'
               AND item.role=plan.leader_role
               AND item.input_manifest_hash=NEW.subject_fingerprint_sha256
               AND item.input_refs=jsonb_build_array(jsonb_build_object(
                    'kind','pending_evolution_authority',
                    'id',NEW.pending_evolution_authority_id,
                    'source_generation_id',NEW.source_generation_id,
                    'subject_fingerprint_sha256',NEW.subject_fingerprint_sha256))
               AND item.status='queued' AND NOT item.required_for_barrier)
       OR NOT EXISTS(
            SELECT 1
              FROM stage_worker_runs worker
              JOIN message_chains target_chain
                ON target_chain.id=NEW.primary_message_chain_id
               AND target_chain.id=worker.message_chain_id
              JOIN message_chains source_chain
                ON source_chain.id=NEW.previous_analysis_primary_message_chain_id
             WHERE worker.id=NEW.primary_worker_run_id
               AND worker.work_item_id=NEW.primary_work_item_id
               AND worker.specialist='investigation_evolution_primary'
               AND worker.status='queued'
               AND target_chain.session_id=source_chain.session_id
               AND target_chain.task_id=source_chain.task_id
               AND target_chain.agent=source_chain.agent
               AND target_chain.model IS NOT DISTINCT FROM source_chain.model
               AND target_chain.provider IS NOT DISTINCT FROM source_chain.provider
               AND target_chain.chain=source_chain.chain)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EVOLUTION_ANALYSIS_PRIMARY_REARM_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_evolution_analysis_primary_rearms_contract
BEFORE INSERT OR UPDATE OR DELETE ON investigation_evolution_analysis_primary_rearms
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_evolution_analysis_primary_rearm();

-- Retain every prior StageTeam transition and add only the exact Evolution
-- Analysis closed-to-open receipt above.
CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
    target_intel_goal_resume_advance BOOLEAN := FALSE;
    investigation_task_rearm_advance BOOLEAN := FALSE;
    investigation_execution_primary_rearm_advance BOOLEAN := FALSE;
    investigation_evolution_analysis_rearm_advance BOOLEAN := FALSE;
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
    investigation_evolution_analysis_rearm_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND OLD.final_submitter_worker_run_id IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND OLD.stage_kind='investigation'
        AND OLD.dynamic_request_policy->>'coordination_mode'='investigation_task_orchestrator'
        AND EXISTS (
            SELECT 1 FROM investigation_evolution_analysis_primary_rearms authority
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
        AND NOT investigation_evolution_analysis_rearm_advance
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
        AND NOT investigation_evolution_analysis_rearm_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT investigation_task_rearm_advance
        AND NOT investigation_execution_primary_rearm_advance
        AND NOT investigation_evolution_analysis_rearm_advance
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
