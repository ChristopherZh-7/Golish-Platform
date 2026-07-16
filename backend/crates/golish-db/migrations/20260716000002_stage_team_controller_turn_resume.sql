-- One explicitly claimed successor Operation Turn may re-arm the exact same
-- fuel-exhausted Company Controller.  This is an additive recovery authority:
-- it does not replace the immutable Gate gap, mint another WorkerRun/message
-- chain, or relax deterministic Gate validation.

-- A stable Controller may produce more than one immutable Gate decision over
-- its lifetime.  Per-epoch/hash uniqueness below remains authoritative.
ALTER TABLE stage_team_unit_gaps
    DROP CONSTRAINT IF EXISTS stage_team_unit_gaps_source_aggregator_worker_run_id_key;

CREATE INDEX IF NOT EXISTS stage_team_unit_gaps_source_worker
    ON stage_team_unit_gaps(source_aggregator_worker_run_id, source_dispatch_epoch);

ALTER TABLE stage_team_unit_gaps
    ADD CONSTRAINT stage_team_unit_gaps_exact_worker_attempt_submission_unique
    UNIQUE (
        source_aggregator_worker_run_id,
        source_attempt_epoch,
        deliverable_submission_id
    );

-- Freeze only the pre-migration Controllers that already reached the old v1
-- fuel-exhausted boundary without a gap row.  Future missing-gap bugs cannot
-- opt into this compatibility path: new Workers start NULL and the witness is
-- immutable after this one-time backfill.
ALTER TABLE stage_worker_runs
    ADD COLUMN legacy_controller_gap_checkpoint_hash TEXT CHECK (
        legacy_controller_gap_checkpoint_hash IS NULL
        OR legacy_controller_gap_checkpoint_hash ~ '^sha256:[0-9a-f]{64}$'
    );

UPDATE stage_worker_runs worker
   SET legacy_controller_gap_checkpoint_hash=
       'sha256:' || attack_fact_delta_sha256_jsonb(worker.checkpoint)
  FROM stage_team_plans plan,
       stage_work_items item,
       stage_run_units unit,
       operation_state operation
 WHERE worker.id=plan.final_submitter_worker_run_id
   AND item.id=worker.work_item_id
   AND item.team_plan_id=plan.id
   AND unit.id=plan.stage_run_unit_id
   AND operation.operation_id=plan.operation_id
   AND operation.runtime_memory_contract::TEXT='v2_only'
   AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
   AND plan.requests_closed_at IS NOT NULL
   AND unit.status='gate_blocked'
   AND item.stable_key='leader:primary'
   AND item.role=plan.leader_role
   AND item.status='superseded'
   AND item.terminal_at IS NOT NULL
   AND worker.status='gate_blocked'
   AND worker.message_chain_id IS NOT NULL
   AND worker.lease_token IS NULL
   AND worker.active_tool_call_id IS NULL
   AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,schema_version}'='1'
   AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,fuel_exhausted}'='true'
   AND NOT EXISTS (
       SELECT 1 FROM stage_team_unit_gaps gap
        WHERE gap.request_id=
              worker.checkpoint #>> '{_runtime_stage_team_gate_block,request_id}'
   );

CREATE FUNCTION freeze_legacy_controller_gap_checkpoint_witness()
RETURNS trigger AS $$
BEGIN
    IF TG_OP='INSERT' AND NEW.legacy_controller_gap_checkpoint_hash IS NOT NULL THEN
        RAISE EXCEPTION 'LEGACY_CONTROLLER_GAP_WITNESS_BACKFILL_ONLY';
    END IF;
    IF TG_OP='UPDATE' AND NEW.legacy_controller_gap_checkpoint_hash
        IS DISTINCT FROM OLD.legacy_controller_gap_checkpoint_hash
    THEN
        RAISE EXCEPTION 'LEGACY_CONTROLLER_GAP_WITNESS_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_worker_runs_legacy_controller_gap_witness_insert
BEFORE INSERT ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION freeze_legacy_controller_gap_checkpoint_witness();

CREATE TRIGGER stage_worker_runs_legacy_controller_gap_witness_update
BEFORE UPDATE OF legacy_controller_gap_checkpoint_hash ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION freeze_legacy_controller_gap_checkpoint_witness();

ALTER TABLE operation_turns
    ADD CONSTRAINT operation_turns_id_operation_unique
    UNIQUE (id, operation_id);

CREATE FUNCTION enforce_operation_turn_transition_contract()
RETURNS trigger AS $$
BEGIN
    IF ROW(
        NEW.id, NEW.operation_id, NEW.ordinal, NEW.trigger_input, NEW.started_at
    ) IS DISTINCT FROM ROW(
        OLD.id, OLD.operation_id, OLD.ordinal, OLD.trigger_input, OLD.started_at
    ) THEN
        RAISE EXCEPTION 'OPERATION_TURN_IDENTITY_IMMUTABLE';
    END IF;
    IF NOT (
        (OLD.status='running' AND NEW.status='waiting'
            AND NEW.terminal_at IS NULL)
        OR (OLD.status IN ('running','waiting')
            AND NEW.status IN ('completed','interrupted','failed')
            AND NEW.terminal_at IS NOT NULL)
    ) THEN
        RAISE EXCEPTION 'OPERATION_TURN_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_turns_transition_contract
BEFORE UPDATE ON operation_turns
FOR EACH ROW EXECUTE FUNCTION enforce_operation_turn_transition_contract();

CREATE TABLE stage_team_controller_turn_resumes (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    prior_turn_id UUID NOT NULL,
    resume_turn_id UUID NOT NULL,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    source_gap_id UUID REFERENCES stage_team_unit_gaps(id) ON DELETE RESTRICT,
    source_request_id TEXT NOT NULL CHECK (btrim(source_request_id) <> ''),
    deliverable_submission_id UUID NOT NULL
        REFERENCES stage_deliverable_submissions(id) ON DELETE RESTRICT,
    source_lease_token UUID NOT NULL,
    source_manifest_hash TEXT NOT NULL CHECK (
        source_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_gate_decision_hash TEXT NOT NULL CHECK (
        source_gate_decision_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_gap_manifest_hash TEXT NOT NULL CHECK (
        source_gap_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_repair_generation INTEGER NOT NULL CHECK (source_repair_generation > 0),
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    leader_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    leader_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    message_chain_id UUID NOT NULL REFERENCES message_chains(id) ON DELETE RESTRICT,
    source_dispatch_epoch BIGINT NOT NULL CHECK (source_dispatch_epoch >= 0),
    resume_dispatch_epoch BIGINT NOT NULL CHECK (
        resume_dispatch_epoch = source_dispatch_epoch + 1
    ),
    source_plan_row_version BIGINT NOT NULL CHECK (source_plan_row_version >= 0),
    source_unit_row_version BIGINT NOT NULL CHECK (source_unit_row_version >= 0),
    source_item_row_version BIGINT NOT NULL CHECK (source_item_row_version >= 0),
    source_attempt_epoch BIGINT NOT NULL CHECK (source_attempt_epoch >= 0),
    source_checkpoint_version BIGINT NOT NULL CHECK (source_checkpoint_version >= 0),
    source_checkpoint JSONB NOT NULL CHECK (jsonb_typeof(source_checkpoint) = 'object'),
    source_checkpoint_hash TEXT NOT NULL CHECK (
        source_checkpoint_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    status TEXT NOT NULL CHECK (status IN ('building', 'applied')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    applied_at TIMESTAMPTZ,
    UNIQUE (resume_turn_id, team_plan_id),
    UNIQUE (team_plan_id, resume_dispatch_epoch),
    CHECK (
        (status='building' AND applied_at IS NULL)
        OR (status='applied' AND applied_at IS NOT NULL)
    ),
    FOREIGN KEY (prior_turn_id, operation_id)
        REFERENCES operation_turns(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (resume_turn_id, operation_id)
        REFERENCES operation_turns(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans(
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_gap_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_team_unit_gaps(
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        leader_work_item_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_work_items(
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        leader_worker_run_id, leader_work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs(
        id, work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_stage_team_controller_turn_resume_contract()
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
                   SELECT COUNT(*) FROM stage_team_repair_generations generation
                    WHERE generation.team_plan_id=plan.id
               ) + (
                   SELECT COUNT(*) FROM stage_team_controller_turn_resumes prior_resume
                    WHERE prior_resume.team_plan_id=plan.id
               ) < LEAST(
                   3,
                   GREATEST(
                       0,
                       COALESCE(
                           (plan.dynamic_request_policy->>'max_repair_generations')::BIGINT,
                           (plan.dynamic_request_policy->>'max_controller_gate_repairs')::BIGINT,
                           1
                       )
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
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gate_decision_hash}'=NEW.source_gate_decision_hash
           AND worker.checkpoint #>> '{_runtime_stage_team_turn_resume,source_request_id}'=NEW.source_request_id
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_team_controller_turn_resumes_contract
BEFORE INSERT OR UPDATE OR DELETE ON stage_team_controller_turn_resumes
FOR EACH ROW EXECUTE FUNCTION enforce_stage_team_controller_turn_resume_contract();

CREATE FUNCTION enforce_stage_team_controller_turn_resume_applied_at_commit()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM stage_team_controller_turn_resumes authority
         WHERE authority.id=NEW.id AND authority.status<>'applied'
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_TURN_RESUME_MUST_APPLY_AT_COMMIT';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER stage_team_controller_turn_resumes_applied_at_commit
AFTER INSERT OR UPDATE ON stage_team_controller_turn_resumes
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_stage_team_controller_turn_resume_applied_at_commit();

CREATE FUNCTION enforce_stage_team_controller_worker_turn_resume_contract()
RETURNS trigger AS $$
BEGIN
    IF OLD.status='gate_blocked' AND NEW.status='waiting_background' THEN
        IF NEW.attempt_epoch<>OLD.attempt_epoch
            OR NEW.checkpoint_version<>OLD.checkpoint_version+1
            OR NEW.message_chain_id IS DISTINCT FROM OLD.message_chain_id
            OR NEW.lease_token IS NOT NULL
            OR NEW.active_tool_call_id IS NOT NULL
            OR NEW.terminal_at IS NOT NULL
            OR NOT EXISTS (
                SELECT 1
                  FROM stage_team_controller_turn_resumes authority
                 WHERE authority.status='building'
                   AND authority.leader_worker_run_id=OLD.id
                   AND authority.source_attempt_epoch=OLD.attempt_epoch
                   AND authority.source_checkpoint_version=OLD.checkpoint_version
                   AND authority.source_checkpoint=OLD.checkpoint
                   AND authority.message_chain_id=OLD.message_chain_id
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,authority_id}'=authority.id::text
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,prior_turn_id}'=authority.prior_turn_id::text
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,resume_turn_id}'=authority.resume_turn_id::text
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gap_id}'
                       IS NOT DISTINCT FROM authority.source_gap_id::text
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gap_manifest_hash}'=authority.source_gap_manifest_hash
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,source_gate_decision_hash}'=authority.source_gate_decision_hash
                   AND NEW.checkpoint #>> '{_runtime_stage_team_turn_resume,source_request_id}'=authority.source_request_id
            )
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_CONTROLLER_WORKER_TURN_RESUME_AUTHORITY_REQUIRED';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_team_controller_worker_turn_resume_contract
BEFORE UPDATE OF status,checkpoint,checkpoint_version,message_chain_id,
    attempt_epoch,lease_token,active_tool_call_id,terminal_at
ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION enforce_stage_team_controller_worker_turn_resume_contract();

CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;

    IF TG_OP = 'INSERT' THEN
        IF NOT (NEW.allowed_worker_roles ? NEW.leader_role)
            OR (
                NEW.aggregator_kind = 'worker'
                AND NOT (NEW.allowed_worker_roles ? NEW.aggregator_role)
            )
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_ROLE_NOT_ALLOWED';
        END IF;
        IF EXISTS (
            SELECT 1 FROM stage_worker_runs worker
             WHERE worker.stage_run_unit_id=NEW.stage_run_unit_id
        ) THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.id, NEW.operation_id, NEW.stage_execution_id,
        NEW.stage_run_unit_id, NEW.scope_snapshot_id, NEW.organization_id,
        NEW.stage_kind, NEW.unit_generation, NEW.schema_version,
        NEW.plan_version, NEW.plan_hash, NEW.leader_role,
        NEW.aggregator_kind, NEW.aggregator_role, NEW.allowed_worker_roles,
        NEW.max_workers_total, NEW.max_workers_active,
        NEW.dynamic_requests_allowed, NEW.dynamic_request_policy,
        NEW.final_submitter_kind, NEW.created_from_stage_spec_hash, NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id, OLD.operation_id, OLD.stage_execution_id,
        OLD.stage_run_unit_id, OLD.scope_snapshot_id, OLD.organization_id,
        OLD.stage_kind, OLD.unit_generation, OLD.schema_version,
        OLD.plan_version, OLD.plan_hash, OLD.leader_role,
        OLD.aggregator_kind, OLD.aggregator_role, OLD.allowed_worker_roles,
        OLD.max_workers_total, OLD.max_workers_active,
        OLD.dynamic_requests_allowed, OLD.dynamic_request_policy,
        OLD.final_submitter_kind, OLD.created_from_stage_spec_hash, OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;

    repair_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1
              FROM stage_team_repair_generations generation
              JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
             WHERE generation.team_plan_id=OLD.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
               AND gap.source_dispatch_epoch=OLD.dispatch_epoch
               AND gap.source_aggregator_worker_run_id=OLD.final_submitter_worker_run_id
        );
    controller_turn_resume_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_controller_turn_resumes authority
             WHERE authority.team_plan_id=OLD.id
               AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.leader_worker_run_id=OLD.final_submitter_worker_run_id
        );
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch
        AND NOT repair_advance
        AND NOT controller_turn_resume_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR';
    END IF;
    IF NEW.row_version <> OLD.row_version+1 OR NEW.updated_at < OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED';
    END IF;
    IF OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
        AND NOT repair_advance
        AND NOT controller_turn_resume_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance
        AND NOT controller_turn_resume_advance
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

CREATE OR REPLACE FUNCTION enforce_stage_work_item_contract()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    controller_turn_resume BOOLEAN := FALSE;
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
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
