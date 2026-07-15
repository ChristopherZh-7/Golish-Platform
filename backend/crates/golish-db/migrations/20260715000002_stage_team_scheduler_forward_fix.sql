-- Forward repair for the audited edit to 20260714000003.
--
-- The applied schema predates recovery/gap/repair-generation objects and four
-- current function contracts. The checksum repair is exact; this migration
-- installs only the catalog differences proven by a fresh-schema comparison.

CREATE TABLE IF NOT EXISTS stage_team_recovery_decisions (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (btrim(request_id) <> ''),
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    tool_call_record_id UUID NOT NULL UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    expected_work_item_row_version BIGINT NOT NULL CHECK (expected_work_item_row_version >= 0),
    expected_checkpoint_version BIGINT NOT NULL CHECK (expected_checkpoint_version >= 0),
    expected_attempt_epoch BIGINT NOT NULL CHECK (expected_attempt_epoch >= 0),
    resolution_kind TEXT NOT NULL CHECK (
        resolution_kind = 'mark_blocked_outcome_unknown'
    ),
    resolution_payload JSONB NOT NULL CHECK (jsonb_typeof(resolution_payload) = 'object'),
    resolution_hash TEXT NOT NULL CHECK (resolution_hash ~ '^sha256:[0-9a-f]{64}$'),
    resolved_by TEXT NOT NULL CHECK (btrim(resolved_by) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (
        team_plan_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES stage_team_plans(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        work_item_id,
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
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        worker_run_id,
        work_item_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        work_item_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (tool_call_record_id, worker_run_id)
        REFERENCES tool_calls(id, worker_run_id) ON DELETE RESTRICT
);

CREATE OR REPLACE FUNCTION reject_stage_team_recovery_decision_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'STAGE_TEAM_RECOVERY_DECISION_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS stage_team_recovery_decisions_immutable
    ON stage_team_recovery_decisions;
CREATE TRIGGER stage_team_recovery_decisions_immutable
BEFORE UPDATE OR DELETE ON stage_team_recovery_decisions
FOR EACH ROW EXECUTE FUNCTION reject_stage_team_recovery_decision_change();

CREATE TABLE IF NOT EXISTS stage_team_unit_gaps (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (btrim(request_id) <> ''),
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_dispatch_epoch BIGINT NOT NULL CHECK (source_dispatch_epoch >= 0),
    source_manifest_hash TEXT NOT NULL CHECK (source_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_attempt_epoch BIGINT NOT NULL CHECK (source_attempt_epoch >= 0),
    source_checkpoint_version BIGINT NOT NULL CHECK (source_checkpoint_version >= 0),
    source_lease_token UUID NOT NULL,
    source_aggregator_work_item_id UUID NOT NULL
        REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    source_aggregator_worker_run_id UUID NOT NULL UNIQUE
        REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    deliverable_submission_id UUID NOT NULL
        REFERENCES stage_deliverable_submissions(id) ON DELETE RESTRICT,
    gate_decision_hash TEXT NOT NULL CHECK (gate_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    gap_manifest JSONB NOT NULL CHECK (jsonb_typeof(gap_manifest) = 'object'),
    gap_manifest_hash TEXT NOT NULL CHECK (gap_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    repair_generation INTEGER NOT NULL CHECK (repair_generation > 0),
    disposition TEXT NOT NULL CHECK (disposition IN ('opened', 'fuel_exhausted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (team_plan_id, source_dispatch_epoch, gate_decision_hash),
    UNIQUE (
        id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ),
    FOREIGN KEY (
        team_plan_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES stage_team_plans(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_aggregator_worker_run_id,
        source_aggregator_work_item_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        work_item_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT
);

CREATE OR REPLACE FUNCTION reject_stage_team_unit_gap_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'STAGE_TEAM_UNIT_GAP_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS stage_team_unit_gaps_immutable ON stage_team_unit_gaps;
CREATE TRIGGER stage_team_unit_gaps_immutable
BEFORE UPDATE OR DELETE ON stage_team_unit_gaps
FOR EACH ROW EXECUTE FUNCTION reject_stage_team_unit_gap_change();

CREATE TABLE IF NOT EXISTS stage_team_repair_generations (
    id UUID PRIMARY KEY,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    source_gap_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    dispatch_epoch BIGINT NOT NULL CHECK (dispatch_epoch > 0),
    repair_work_item_id UUID UNIQUE,
    aggregator_work_item_id UUID UNIQUE,
    manifest JSONB NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
    manifest_hash TEXT NOT NULL CHECK (manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('building', 'sealed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_at TIMESTAMPTZ,
    UNIQUE (team_plan_id, dispatch_epoch),
    FOREIGN KEY (
        team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) REFERENCES stage_team_plans(
        id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_gap_id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) REFERENCES stage_team_unit_gaps(
        id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
        scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        repair_work_item_id,team_plan_id,operation_id,stage_execution_id,
        stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch
    ) REFERENCES stage_work_items(
        id,team_plan_id,operation_id,stage_execution_id,
        stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        aggregator_work_item_id,team_plan_id,operation_id,stage_execution_id,
        stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch
    ) REFERENCES stage_work_items(
        id,team_plan_id,operation_id,stage_execution_id,
        stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch
    ) ON DELETE RESTRICT,
    CHECK (
        (status='building' AND repair_work_item_id IS NULL
            AND aggregator_work_item_id IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND repair_work_item_id IS NOT NULL
            AND aggregator_work_item_id IS NOT NULL AND sealed_at IS NOT NULL)
    )
);

CREATE OR REPLACE FUNCTION enforce_stage_team_repair_generation_contract()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'STAGE_TEAM_REPAIR_GENERATION_IMMUTABLE';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'building' THEN
            RAISE EXCEPTION 'STAGE_TEAM_REPAIR_GENERATION_MUST_BUILD_FIRST';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.team_plan_id,NEW.source_gap_id,NEW.operation_id,
        NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
        NEW.organization_id,NEW.dispatch_epoch,
        NEW.manifest,NEW.manifest_hash,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.team_plan_id,OLD.source_gap_id,OLD.operation_id,
        OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
        OLD.organization_id,OLD.dispatch_epoch,
        OLD.manifest,OLD.manifest_hash,OLD.created_at
    ) OR OLD.status <> 'building' OR NEW.status <> 'sealed'
       OR OLD.repair_work_item_id IS NOT NULL OR OLD.aggregator_work_item_id IS NOT NULL
       OR NEW.repair_work_item_id IS NULL OR NEW.aggregator_work_item_id IS NULL
       OR OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REPAIR_GENERATION_IMMUTABLE';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM stage_work_items AS repair
         WHERE repair.id=NEW.repair_work_item_id
           AND repair.team_plan_id=NEW.team_plan_id
           AND repair.dispatch_epoch=NEW.dispatch_epoch
           AND repair.created_by='gate_repair'
           AND repair.required_for_barrier=TRUE
    ) OR NOT EXISTS (
        SELECT 1 FROM stage_work_items AS aggregator
         WHERE aggregator.id=NEW.aggregator_work_item_id
           AND aggregator.team_plan_id=NEW.team_plan_id
           AND aggregator.dispatch_epoch=NEW.dispatch_epoch
           AND aggregator.created_by='gate_repair'
           AND aggregator.required_for_barrier=FALSE
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_REPAIR_GENERATION_ITEM_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS stage_team_repair_generations_contract
    ON stage_team_repair_generations;
CREATE TRIGGER stage_team_repair_generations_contract
BEFORE INSERT OR UPDATE OR DELETE ON stage_team_repair_generations
FOR EACH ROW EXECUTE FUNCTION enforce_stage_team_repair_generation_contract();


CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
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
            SELECT 1
              FROM stage_worker_runs AS worker
             WHERE worker.stage_run_unit_id = NEW.stage_run_unit_id
        ) THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.id,
        NEW.operation_id,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,
        NEW.organization_id,
        NEW.stage_kind,
        NEW.unit_generation,
        NEW.schema_version,
        NEW.plan_version,
        NEW.plan_hash,
        NEW.leader_role,
        NEW.aggregator_kind,
        NEW.aggregator_role,
        NEW.allowed_worker_roles,
        NEW.max_workers_total,
        NEW.max_workers_active,
        NEW.dynamic_requests_allowed,
        NEW.dynamic_request_policy,
        NEW.final_submitter_kind,
        NEW.created_from_stage_spec_hash,
        NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,
        OLD.operation_id,
        OLD.stage_execution_id,
        OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,
        OLD.organization_id,
        OLD.stage_kind,
        OLD.unit_generation,
        OLD.schema_version,
        OLD.plan_version,
        OLD.plan_hash,
        OLD.leader_role,
        OLD.aggregator_kind,
        OLD.aggregator_role,
        OLD.allowed_worker_roles,
        OLD.max_workers_total,
        OLD.max_workers_active,
        OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,
        OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,
        OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;
    repair_advance :=
        NEW.dispatch_epoch = OLD.dispatch_epoch + 1
        AND OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1
              FROM stage_team_repair_generations AS generation
              JOIN stage_team_unit_gaps AS gap ON gap.id=generation.source_gap_id
             WHERE generation.team_plan_id=OLD.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
               AND gap.source_dispatch_epoch=OLD.dispatch_epoch
               AND gap.source_aggregator_worker_run_id=
                   OLD.final_submitter_worker_run_id
        );
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch AND NOT repair_advance THEN
        RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR';
    END IF;
    IF NEW.row_version <> OLD.row_version + 1 OR NEW.updated_at < OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED';
    END IF;
    IF OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
        AND NOT repair_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance
        AND NOT (
            NEW.final_submitter_worker_run_id IS NOT NULL
            AND EXISTS (
                SELECT 1
                  FROM stage_worker_runs AS previous_submitter
                 WHERE previous_submitter.id = OLD.final_submitter_worker_run_id
                   AND previous_submitter.status = 'superseded'
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
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;

    IF TG_OP = 'INSERT' THEN
        SELECT * INTO plan
          FROM stage_team_plans AS persisted
         WHERE persisted.id = NEW.team_plan_id
           AND persisted.operation_id = NEW.operation_id
           AND persisted.stage_execution_id = NEW.stage_execution_id
           AND persisted.stage_run_unit_id = NEW.stage_run_unit_id
           AND persisted.scope_snapshot_id = NEW.scope_snapshot_id
           AND persisted.organization_id = NEW.organization_id
         FOR UPDATE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_OWNER_MISMATCH';
        END IF;
        IF plan.requests_closed_at IS NOT NULL OR NEW.dispatch_epoch <> plan.dispatch_epoch THEN
            RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CLOSED';
        END IF;
        IF NOT (plan.allowed_worker_roles ? NEW.role) THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_ROLE_NOT_ALLOWED';
        END IF;
        IF NEW.created_by = 'gate_repair' AND NOT EXISTS (
            SELECT 1
              FROM stage_team_repair_generations AS generation
             WHERE generation.team_plan_id=plan.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
        ) THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_REPAIR_GENERATION_REQUIRED';
        END IF;
        RETURN NEW;
    END IF;

    IF ROW(
        NEW.id,
        NEW.team_plan_id,
        NEW.operation_id,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,
        NEW.organization_id,
        NEW.dispatch_epoch,
        NEW.kind,
        NEW.stable_key,
        NEW.role,
        NEW.input_manifest_hash,
        NEW.input_refs,
        NEW.required_for_barrier,
        NEW.conflict_key,
        NEW.priority,
        NEW.attempt_policy,
        NEW.budget,
        NEW.output_schema,
        NEW.created_by,
        NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,
        OLD.team_plan_id,
        OLD.operation_id,
        OLD.stage_execution_id,
        OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,
        OLD.organization_id,
        OLD.dispatch_epoch,
        OLD.kind,
        OLD.stable_key,
        OLD.role,
        OLD.input_manifest_hash,
        OLD.input_refs,
        OLD.required_for_barrier,
        OLD.conflict_key,
        OLD.priority,
        OLD.attempt_policy,
        OLD.budget,
        OLD.output_schema,
        OLD.created_by,
        OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF NEW.row_version <> OLD.row_version + 1 OR NEW.updated_at < OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_VERSION_CAS_REQUIRED';
    END IF;
    IF NOT (
        (OLD.status = 'queued' AND NEW.status IN ('claimed', 'running', 'superseded'))
        OR (OLD.status = 'claimed' AND NEW.status IN ('queued', 'running', 'recovery_required', 'superseded'))
        OR (OLD.status = 'running' AND NEW.status IN (
            'waiting_dependency', 'completed', 'retry_pending', 'recovery_required',
            'exhausted', 'superseded'
        ))
        OR (OLD.status = 'waiting_dependency' AND NEW.status IN (
            'queued', 'running', 'recovery_required', 'superseded'
        ))
        OR (OLD.status = 'retry_pending' AND NEW.status IN ('queued', 'exhausted', 'superseded'))
        OR (OLD.status = 'recovery_required' AND NEW.status IN (
            'queued', 'completed', 'exhausted', 'superseded'
        ))
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


CREATE OR REPLACE FUNCTION enforce_terminal_stage_worker_output()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM stage_worker_runs AS worker
          JOIN stage_work_items AS item
            ON item.id = worker.work_item_id
           AND item.operation_id = worker.operation_id
           AND item.stage_execution_id = worker.stage_execution_id
           AND item.stage_run_unit_id = worker.stage_run_unit_id
           AND item.organization_id = worker.organization_id
         WHERE worker.id = NEW.worker_run_id
           AND worker.work_item_id = NEW.work_item_id
           AND worker.terminal_at IS NOT NULL
           AND worker.active_tool_call_id IS NULL
           AND item.terminal_at IS NOT NULL
           AND (
               (
                   worker.status = 'passed'
                   AND item.status = 'completed'
               )
               OR
               (
                   worker.status = 'failed'
                   AND item.status = 'exhausted'
                   AND NEW.business_disposition = 'blocked'
                   AND NEW.canonical_output ->> 'kind' = 'stage_team_attempts_exhausted'
                   AND NEW.canonical_fact_refs = '[]'::jsonb
                   AND NEW.evidence_ids = ARRAY[]::BIGINT[]
                   AND NEW.checked_empty_cells = '[]'::jsonb
                   AND NEW.blocker_codes = ARRAY[
                       'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'
                   ]::TEXT[]
               )
               OR
               (
                   worker.status = 'failed'
                   AND item.status = 'exhausted'
                   AND NEW.business_disposition = 'blocked'
                   AND NEW.canonical_output ->> 'kind' =
                       'stage_team_active_tool_recovery_blocked'
                   AND NEW.canonical_fact_refs = '[]'::jsonb
                   AND NEW.evidence_ids = ARRAY[]::BIGINT[]
                   AND NEW.checked_empty_cells = '[]'::jsonb
                   AND NEW.blocker_codes = ARRAY[
                       'STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED'
                   ]::TEXT[]
                   AND EXISTS (
                       SELECT 1
                         FROM stage_team_recovery_decisions AS decision
                        WHERE decision.worker_run_id = NEW.worker_run_id
                          AND decision.work_item_id = NEW.work_item_id
                          AND decision.tool_call_record_id =
                              (NEW.canonical_output ->> 'tool_call_record_id')::UUID
                          AND decision.resolution_kind =
                              'mark_blocked_outcome_unknown'
                   )
               )
           )
    ) THEN
        RAISE EXCEPTION 'STAGE_WORKER_OUTPUT_REQUIRES_TERMINAL_WORKER_AND_ITEM';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;


CREATE OR REPLACE FUNCTION enforce_stage_team_deliverable_submitter()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
BEGIN
    IF NEW.stage_run_unit_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT * INTO plan
      FROM stage_team_plans AS persisted
     WHERE persisted.stage_run_unit_id = NEW.stage_run_unit_id
     FOR SHARE;
    IF NOT FOUND THEN
        RETURN NEW;
    END IF;
    IF NEW.worker_run_id IS NULL
        OR plan.final_submitter_worker_run_id IS NULL
        OR NEW.worker_run_id <> plan.final_submitter_worker_run_id
        OR NOT EXISTS (
            SELECT 1
              FROM stage_worker_runs AS worker
              JOIN stage_work_items AS item
                ON item.id = worker.work_item_id
               AND item.operation_id = worker.operation_id
               AND item.stage_execution_id = worker.stage_execution_id
               AND item.stage_run_unit_id = worker.stage_run_unit_id
               AND item.organization_id = worker.organization_id
             WHERE worker.id = NEW.worker_run_id
               AND worker.operation_id = NEW.operation_id
               AND worker.stage_execution_id = NEW.stage_execution_id
               AND worker.stage_run_unit_id = NEW.stage_run_unit_id
               AND worker.organization_id = NEW.organization_id
               AND worker.status = 'running'
               AND worker.active_tool_call_id = NEW.tool_call_record_id
               AND item.team_plan_id = plan.id
               AND item.role = plan.aggregator_role
               AND item.required_for_barrier = FALSE
               AND item.status = 'running'
        )
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_SUBMISSION_REQUIRES_UNIQUE_AGGREGATOR';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS stage_deliverable_submissions_team_submitter
    ON stage_deliverable_submissions;
CREATE TRIGGER stage_deliverable_submissions_team_submitter
BEFORE INSERT ON stage_deliverable_submissions
FOR EACH ROW EXECUTE FUNCTION enforce_stage_team_deliverable_submitter();
