-- Runtime-memory V2 expand migration.
--
-- Existing operations remain on the legacy contract. New operations freeze one
-- explicit contract at creation; the immutable trigger prevents an in-flight
-- operation from switching stores. All runtime tables are additive and retain
-- the legacy JSON checkpoint unchanged during the compatibility window.

CREATE TABLE runtime_memory_rollout (
    singleton_id SMALLINT PRIMARY KEY CHECK (singleton_id = 1),
    contract TEXT NOT NULL CHECK (
        contract IN (
            'legacy_v1',
            'dual_write_legacy_read',
            'dual_write_v2_preferred',
            'v2_only'
        )
    ),
    contract_rank SMALLINT NOT NULL CHECK (contract_rank BETWEEN 0 AND 3),
    row_version BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        contract_rank = CASE contract
            WHEN 'legacy_v1' THEN 0
            WHEN 'dual_write_legacy_read' THEN 1
            WHEN 'dual_write_v2_preferred' THEN 2
            WHEN 'v2_only' THEN 3
        END
    )
);

INSERT INTO runtime_memory_rollout (singleton_id, contract, contract_rank)
VALUES (1, 'legacy_v1', 0)
ON CONFLICT (singleton_id) DO NOTHING;

CREATE FUNCTION enforce_runtime_memory_rollout_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'runtime memory rollout singleton cannot be deleted';
    END IF;
    IF NEW.singleton_id IS DISTINCT FROM OLD.singleton_id
        OR NEW.contract_rank <> OLD.contract_rank + 1
        OR NEW.row_version <> OLD.row_version + 1
    THEN
        RAISE EXCEPTION 'runtime memory rollout must advance one rank and one row version';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER runtime_memory_rollout_forward_only
BEFORE UPDATE OR DELETE ON runtime_memory_rollout
FOR EACH ROW EXECUTE FUNCTION enforce_runtime_memory_rollout_transition();

ALTER TABLE operation_state
    ADD COLUMN runtime_memory_contract TEXT NOT NULL DEFAULT 'legacy_v1';

ALTER TABLE operation_state
    ADD CONSTRAINT operation_state_runtime_memory_contract_check
    CHECK (
        runtime_memory_contract IN (
            'legacy_v1',
            'dual_write_legacy_read',
            'dual_write_v2_preferred',
            'v2_only'
        )
    );

CREATE FUNCTION reject_operation_runtime_contract_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.runtime_memory_contract IS DISTINCT FROM OLD.runtime_memory_contract THEN
        RAISE EXCEPTION 'operation runtime memory contract is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_runtime_contract_immutable
BEFORE UPDATE OF runtime_memory_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_operation_runtime_contract_change();

ALTER TABLE stage_runs
    ADD CONSTRAINT stage_runs_id_operation_unique UNIQUE (id, operation_id);

ALTER TABLE stage_runs
    ADD CONSTRAINT stage_runs_id_operation_kind_unique
    UNIQUE (id, operation_id, stage_kind);

ALTER TABLE stage_runs
    ADD CONSTRAINT stage_runs_status_check
    CHECK (status IN ('started', 'completed', 'failed', 'paused_needs_user')) NOT VALID;

CREATE TABLE project_scopes (
    project_scope_id UUID PRIMARY KEY,
    canonical_project_path TEXT NOT NULL,
    path_sha256 TEXT NOT NULL,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX project_scopes_one_active_path
    ON project_scopes (canonical_project_path)
    WHERE retired_at IS NULL;

ALTER TABLE operation_state
    ADD COLUMN project_scope_id UUID
    REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT;

ALTER TABLE operation_state
    ADD CONSTRAINT operation_state_operation_project_unique
    UNIQUE (operation_id, project_scope_id);

CREATE TABLE operation_scope_decisions (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    root_organization_id UUID NOT NULL,
    mode TEXT NOT NULL
        CHECK (mode IN ('root_only', 'included', 'reuse_reconfirmed', 'cli_flags')),
    choice_tool_call_id UUID,
    proposal_tool_call_id UUID,
    review_tool_call_id UUID,
    decision_rows JSONB NOT NULL,
    decision_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, stage_execution_id),
    UNIQUE (id, operation_id, project_scope_id, root_organization_id, mode),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (stage_execution_id, operation_id)
        REFERENCES stage_runs(id, operation_id)
);

CREATE TABLE operation_org_scope_snapshots (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE,
    project_scope_id UUID NOT NULL,
    scope_decision_id UUID NOT NULL UNIQUE,
    project_path_at_freeze TEXT NOT NULL,
    root_organization_id UUID NOT NULL,
    mode TEXT NOT NULL,
    scope_hash TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    frozen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_at TIMESTAMPTZ,
    UNIQUE (id, operation_id),
    UNIQUE (id, operation_id, project_scope_id, root_organization_id, mode),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        scope_decision_id,
        operation_id,
        project_scope_id,
        root_organization_id,
        mode
    ) REFERENCES operation_scope_decisions(
        id,
        operation_id,
        project_scope_id,
        root_organization_id,
        mode
    ) ON DELETE RESTRICT,
    CHECK (schema_version > 0),
    CHECK (sealed_at IS NULL OR sealed_at >= frozen_at)
);

CREATE TABLE operation_org_scope_units (
    snapshot_id UUID NOT NULL
        REFERENCES operation_org_scope_snapshots(id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL,
    parent_organization_id UUID,
    organization_name_at_freeze TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('root', 'subsidiary')),
    depth INTEGER NOT NULL CHECK (depth >= 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    ownership_percent NUMERIC CHECK (
        ownership_percent IS NULL OR ownership_percent BETWEEN 0 AND 100
    ),
    decision_row_id TEXT NOT NULL,
    approval_source JSONB NOT NULL,
    PRIMARY KEY (snapshot_id, organization_id),
    UNIQUE (snapshot_id, ordinal),
    FOREIGN KEY (snapshot_id, parent_organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (role = 'root' AND depth = 0 AND parent_organization_id IS NULL)
        OR
        (role = 'subsidiary' AND depth > 0 AND parent_organization_id IS NOT NULL)
    )
);

CREATE UNIQUE INDEX operation_org_scope_units_one_root
    ON operation_org_scope_units(snapshot_id)
    WHERE role = 'root';

ALTER TABLE operation_org_scope_snapshots
    ADD CONSTRAINT operation_org_scope_snapshot_root_fk
    FOREIGN KEY (id, root_organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
        DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION reject_frozen_runtime_scope_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'frozen runtime scope rows are immutable';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION restrict_runtime_scope_snapshot_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'frozen runtime scope snapshots cannot be deleted';
    END IF;
    IF OLD.sealed_at IS NULL
        AND NEW.sealed_at IS NOT NULL
        AND (to_jsonb(NEW) - 'sealed_at') = (to_jsonb(OLD) - 'sealed_at')
    THEN
        PERFORM 1
          FROM operation_org_scope_units AS scope_unit
         WHERE scope_unit.snapshot_id = NEW.id
           AND scope_unit.organization_id = NEW.root_organization_id
           AND scope_unit.role = 'root';
        IF NOT FOUND THEN
            RAISE EXCEPTION 'runtime scope root must reference the root-role unit';
        END IF;
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'frozen runtime scope snapshots only allow one-way sealing';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION reject_late_runtime_scope_unit_insert()
RETURNS trigger AS $$
DECLARE
    snapshot_sealed_at TIMESTAMPTZ;
BEGIN
    SELECT sealed_at
     INTO snapshot_sealed_at
      FROM operation_org_scope_snapshots
     WHERE id = NEW.snapshot_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'runtime scope snapshot is missing';
    END IF;
    IF snapshot_sealed_at IS NOT NULL THEN
        RAISE EXCEPTION 'sealed runtime scope cannot accept new organization units';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION require_sealed_runtime_scope_for_stage_unit()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM operation_org_scope_snapshots AS snapshot
         WHERE snapshot.id = NEW.scope_snapshot_id
           AND snapshot.operation_id = NEW.operation_id
           AND snapshot.sealed_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'stage unit requires a sealed runtime scope snapshot';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_scope_decisions_immutable
BEFORE UPDATE OR DELETE ON operation_scope_decisions
FOR EACH ROW EXECUTE FUNCTION reject_frozen_runtime_scope_change();

CREATE TRIGGER operation_org_scope_snapshots_immutable
BEFORE UPDATE OR DELETE ON operation_org_scope_snapshots
FOR EACH ROW EXECUTE FUNCTION restrict_runtime_scope_snapshot_change();

CREATE TRIGGER operation_org_scope_units_immutable
BEFORE UPDATE OR DELETE ON operation_org_scope_units
FOR EACH ROW EXECUTE FUNCTION reject_frozen_runtime_scope_change();

CREATE TRIGGER operation_org_scope_units_insert_before_seal
BEFORE INSERT ON operation_org_scope_units
FOR EACH ROW EXECUTE FUNCTION reject_late_runtime_scope_unit_insert();

CREATE TABLE stage_run_units (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL
        REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    specialist TEXT,
    status TEXT NOT NULL CHECK (
        status IN ('queued', 'running', 'gate_blocked', 'passed', 'exhausted', 'superseded')
    ),
    gate_attempt INTEGER NOT NULL DEFAULT 0 CHECK (gate_attempt >= 0),
    pass_watermark JSONB NOT NULL DEFAULT '{}'::jsonb,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (stage_execution_id, organization_id),
    UNIQUE (id, operation_id, stage_execution_id, organization_id),
    UNIQUE (id, operation_id, stage_execution_id, organization_id, stage_kind),
    FOREIGN KEY (stage_execution_id, operation_id, stage_kind)
        REFERENCES stage_runs(id, operation_id, stage_kind),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
);

CREATE TRIGGER stage_run_units_require_sealed_scope
BEFORE INSERT OR UPDATE OF operation_id, scope_snapshot_id ON stage_run_units
FOR EACH ROW EXECUTE FUNCTION require_sealed_runtime_scope_for_stage_unit();

ALTER TABLE tool_calls
    ADD COLUMN operation_id UUID,
    ADD COLUMN stage_execution_id UUID,
    ADD COLUMN stage_run_unit_id UUID,
    ADD COLUMN worker_run_id UUID,
    ADD COLUMN organization_id UUID,
    ADD COLUMN attempt_epoch BIGINT,
    ADD COLUMN lease_token UUID;

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_runtime_context_shape_check
    CHECK (
        (
            operation_id IS NULL
            AND stage_execution_id IS NULL
            AND stage_run_unit_id IS NULL
            AND worker_run_id IS NULL
            AND organization_id IS NULL
            AND attempt_epoch IS NULL
            AND lease_token IS NULL
        )
        OR
        (
            operation_id IS NOT NULL
            AND stage_execution_id IS NOT NULL
            AND stage_run_unit_id IS NULL
            AND worker_run_id IS NULL
            AND organization_id IS NULL
            AND attempt_epoch IS NULL
            AND lease_token IS NULL
        )
        OR
        (
            operation_id IS NOT NULL
            AND stage_execution_id IS NOT NULL
            AND stage_run_unit_id IS NOT NULL
            AND worker_run_id IS NULL
            AND organization_id IS NOT NULL
            AND attempt_epoch IS NULL
            AND lease_token IS NULL
        )
        OR
        (
            operation_id IS NOT NULL
            AND stage_execution_id IS NOT NULL
            AND stage_run_unit_id IS NOT NULL
            AND worker_run_id IS NOT NULL
            AND organization_id IS NOT NULL
            AND attempt_epoch IS NOT NULL
            AND attempt_epoch >= 0
            AND lease_token IS NOT NULL
        )
    ),
    ADD CONSTRAINT tool_calls_runtime_task_owner_check
    CHECK (
        operation_id IS NULL
        OR (task_id IS NOT NULL AND task_id = operation_id)
    );

CREATE FUNCTION restrict_tool_call_runtime_context_change()
RETURNS trigger AS $$
BEGIN
    IF ROW(
        NEW.operation_id,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id,
        NEW.worker_run_id,
        NEW.organization_id,
        NEW.attempt_epoch,
        NEW.lease_token
    ) IS NOT DISTINCT FROM ROW(
        OLD.operation_id,
        OLD.stage_execution_id,
        OLD.stage_run_unit_id,
        OLD.worker_run_id,
        OLD.organization_id,
        OLD.attempt_epoch,
        OLD.lease_token
    ) THEN
        RETURN NEW;
    END IF;
    IF OLD.operation_id IS NOT NULL
        AND OLD.stage_execution_id IS NOT NULL
        AND OLD.stage_run_unit_id IS NULL
        AND OLD.worker_run_id IS NULL
        AND OLD.organization_id IS NULL
        AND OLD.attempt_epoch IS NULL
        AND OLD.lease_token IS NULL
        AND NEW.operation_id = OLD.operation_id
        AND NEW.stage_execution_id = OLD.stage_execution_id
        AND NEW.stage_run_unit_id IS NOT NULL
        AND NEW.worker_run_id IS NULL
        AND NEW.organization_id IS NOT NULL
        AND NEW.attempt_epoch IS NULL
        AND NEW.lease_token IS NULL
        AND EXISTS (
            SELECT 1
              FROM stage_runs
             WHERE id = OLD.stage_execution_id
               AND operation_id = OLD.operation_id
               AND stage_kind = 'scoping'
        )
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'tool-call runtime identity is immutable outside scoping bind';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tool_calls_runtime_context_transition
BEFORE UPDATE OF operation_id, stage_execution_id, stage_run_unit_id,
    worker_run_id, organization_id, attempt_epoch, lease_token
ON tool_calls
FOR EACH ROW EXECUTE FUNCTION restrict_tool_call_runtime_context_change();

ALTER TABLE message_chains
    ADD CONSTRAINT message_chains_id_task_unique UNIQUE (id, task_id);

CREATE TABLE stage_worker_runs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    worker_generation INTEGER NOT NULL CHECK (worker_generation >= 0),
    specialist TEXT NOT NULL,
    work_item_kind TEXT NOT NULL,
    work_item_key TEXT NOT NULL,
    agent_path TEXT NOT NULL,
    parent_request_id TEXT,
    message_chain_id UUID,
    status TEXT NOT NULL CHECK (
        status IN (
            'queued',
            'running',
            'waiting_background',
            'gate_blocked',
            'passed',
            'failed',
            'exhausted',
            'superseded',
            'recovery_required'
        )
    ),
    gate_attempt INTEGER NOT NULL DEFAULT 0 CHECK (gate_attempt >= 0),
    checkpoint JSONB NOT NULL DEFAULT '{}'::jsonb,
    checkpoint_version BIGINT NOT NULL DEFAULT 0 CHECK (checkpoint_version >= 0),
    lease_token UUID,
    lease_owner TEXT,
    lease_acquired_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    attempt_epoch BIGINT NOT NULL DEFAULT 0 CHECK (attempt_epoch >= 0),
    active_tool_call_id UUID,
    active_tool_started_at TIMESTAMPTZ,
    evidence_watermark BIGINT,
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (stage_run_unit_id, work_item_kind, work_item_key, worker_generation),
    UNIQUE (id, operation_id, stage_execution_id, stage_run_unit_id, organization_id),
    FOREIGN KEY (stage_run_unit_id, operation_id, stage_execution_id, organization_id)
        REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id),
    FOREIGN KEY (message_chain_id, operation_id)
        REFERENCES message_chains(id, task_id) ON DELETE RESTRICT,
    CHECK (
        (
            lease_token IS NULL
            AND lease_owner IS NULL
            AND lease_acquired_at IS NULL
            AND lease_expires_at IS NULL
            AND heartbeat_at IS NULL
        )
        OR
        (
            lease_token IS NOT NULL
            AND lease_owner IS NOT NULL
            AND lease_acquired_at IS NOT NULL
            AND lease_expires_at IS NOT NULL
            AND lease_expires_at > lease_acquired_at
        )
    ),
    CHECK (
        (active_tool_call_id IS NULL AND active_tool_started_at IS NULL)
        OR
        (active_tool_call_id IS NOT NULL AND active_tool_started_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX stage_worker_runs_chain_owner
    ON stage_worker_runs (message_chain_id)
    WHERE message_chain_id IS NOT NULL;

CREATE FUNCTION verify_runtime_worker_fence()
RETURNS trigger AS $$
BEGIN
    IF NEW.worker_run_id IS NULL THEN
        RETURN NEW;
    END IF;
    PERFORM 1
      FROM stage_worker_runs AS worker
     WHERE worker.id = NEW.worker_run_id
       AND worker.operation_id = NEW.operation_id
       AND worker.stage_execution_id = NEW.stage_execution_id
       AND worker.stage_run_unit_id = NEW.stage_run_unit_id
       AND worker.organization_id = NEW.organization_id
       AND worker.attempt_epoch = NEW.attempt_epoch
       AND worker.lease_token = NEW.lease_token
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'runtime worker fence does not match the active lease';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tool_calls_runtime_worker_fence
BEFORE INSERT OR UPDATE OF operation_id, stage_execution_id, stage_run_unit_id,
    worker_run_id, organization_id, attempt_epoch, lease_token
ON tool_calls
FOR EACH ROW EXECUTE FUNCTION verify_runtime_worker_fence();

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_stage_execution_fk
    FOREIGN KEY (stage_execution_id, operation_id)
        REFERENCES stage_runs(id, operation_id) MATCH FULL;

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_stage_unit_fk
    FOREIGN KEY (stage_run_unit_id, operation_id, stage_execution_id, organization_id)
        REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id);

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_id_execution_unique
    UNIQUE (id, operation_id, stage_execution_id),
    ADD CONSTRAINT tool_calls_id_unit_unique
    UNIQUE (id, stage_run_unit_id, organization_id),
    ADD CONSTRAINT tool_calls_id_worker_fence_unique
    UNIQUE (id, worker_run_id, attempt_epoch, lease_token),
    ADD CONSTRAINT tool_calls_id_worker_unique
    UNIQUE (id, worker_run_id);

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_worker_fence_fk
    FOREIGN KEY (
        worker_run_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    );

ALTER TABLE stage_worker_runs
    ADD CONSTRAINT stage_worker_runs_active_tool_owner_fk
    FOREIGN KEY (active_tool_call_id, id)
        REFERENCES tool_calls(id, worker_run_id);

CREATE UNIQUE INDEX tool_calls_one_active_per_worker
    ON tool_calls(worker_run_id)
    WHERE worker_run_id IS NOT NULL AND status IN ('received', 'running');

CREATE TABLE stage_deliverable_submissions (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID,
    worker_run_id UUID,
    organization_id UUID,
    tool_call_record_id UUID NOT NULL UNIQUE
        REFERENCES tool_calls(id) ON DELETE RESTRICT,
    tool_request_id TEXT NOT NULL,
    stage_kind TEXT NOT NULL,
    attempt_epoch BIGINT,
    lease_token UUID,
    payload JSONB NOT NULL,
    payload_sha256 TEXT NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, operation_id, stage_execution_id),
    UNIQUE (
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ),
    FOREIGN KEY (stage_execution_id, operation_id, stage_kind)
        REFERENCES stage_runs(id, operation_id, stage_kind),
    FOREIGN KEY (
        stage_run_unit_id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ),
    FOREIGN KEY (
        worker_run_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ),
    FOREIGN KEY (tool_call_record_id, operation_id, stage_execution_id)
        REFERENCES tool_calls(id, operation_id, stage_execution_id),
    FOREIGN KEY (tool_call_record_id, stage_run_unit_id, organization_id)
        REFERENCES tool_calls(id, stage_run_unit_id, organization_id),
    FOREIGN KEY (tool_call_record_id, worker_run_id, attempt_epoch, lease_token)
        REFERENCES tool_calls(id, worker_run_id, attempt_epoch, lease_token),
    CHECK (
        (
            stage_kind = 'scoping'
            AND worker_run_id IS NULL
            AND attempt_epoch IS NULL
            AND lease_token IS NULL
            AND (
                (
                    stage_run_unit_id IS NULL
                    AND organization_id IS NULL
                )
                OR
                (
                    stage_run_unit_id IS NOT NULL
                    AND organization_id IS NOT NULL
                )
            )
        )
        OR
        (
            stage_kind <> 'scoping'
            AND stage_run_unit_id IS NOT NULL
            AND worker_run_id IS NOT NULL
            AND organization_id IS NOT NULL
            AND attempt_epoch IS NOT NULL
            AND attempt_epoch >= 0
            AND lease_token IS NOT NULL
        )
    )
);

CREATE TRIGGER stage_deliverable_submissions_worker_fence
BEFORE INSERT OR UPDATE OF operation_id, stage_execution_id, stage_run_unit_id,
    worker_run_id, organization_id, attempt_epoch, lease_token
ON stage_deliverable_submissions
FOR EACH ROW EXECUTE FUNCTION verify_runtime_worker_fence();

CREATE FUNCTION restrict_stage_deliverable_submission_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'stage deliverable submissions cannot be deleted';
    END IF;
    IF OLD.stage_kind = 'scoping'
        AND OLD.stage_run_unit_id IS NULL
        AND OLD.organization_id IS NULL
        AND OLD.worker_run_id IS NULL
        AND OLD.attempt_epoch IS NULL
        AND OLD.lease_token IS NULL
        AND NEW.stage_run_unit_id IS NOT NULL
        AND NEW.organization_id IS NOT NULL
        AND NEW.worker_run_id IS NULL
        AND NEW.attempt_epoch IS NULL
        AND NEW.lease_token IS NULL
        AND ROW(
            NEW.id,
            NEW.operation_id,
            NEW.stage_execution_id,
            NEW.tool_call_record_id,
            NEW.tool_request_id,
            NEW.stage_kind,
            NEW.payload,
            NEW.payload_sha256,
            NEW.submitted_at
        ) IS NOT DISTINCT FROM ROW(
            OLD.id,
            OLD.operation_id,
            OLD.stage_execution_id,
            OLD.tool_call_record_id,
            OLD.tool_request_id,
            OLD.stage_kind,
            OLD.payload,
            OLD.payload_sha256,
            OLD.submitted_at
        )
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'stage deliverable submission is immutable outside scoping bind';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_deliverable_submissions_immutable
BEFORE UPDATE OR DELETE ON stage_deliverable_submissions
FOR EACH ROW EXECUTE FUNCTION restrict_stage_deliverable_submission_change();

CREATE TABLE stage_handoffs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    from_stage_kind TEXT NOT NULL,
    stage_execution_id UUID NOT NULL,
    source_stage_run_unit_id UUID NOT NULL UNIQUE,
    deliverable_submission_id UUID NOT NULL UNIQUE,
    scope_hash TEXT NOT NULL,
    payload JSONB NOT NULL,
    payload_sha256 TEXT NOT NULL,
    evidence_ids BIGINT[] NOT NULL DEFAULT '{}',
    coverage_watermark JSONB NOT NULL DEFAULT '{}'::jsonb,
    unit_gate_decision_hash TEXT NOT NULL,
    aggregate_pass_token_hash TEXT,
    gate_passed_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE (stage_execution_id, organization_id),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id),
    FOREIGN KEY (
        source_stage_run_unit_id,
        operation_id,
        stage_execution_id,
        organization_id,
        from_stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ),
    FOREIGN KEY (
        deliverable_submission_id,
        operation_id,
        stage_execution_id,
        source_stage_run_unit_id,
        organization_id,
        from_stage_kind
    ) REFERENCES stage_deliverable_submissions(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ),
    CHECK (schema_version > 0),
    CHECK (invalidated_at IS NULL OR invalidated_at >= gate_passed_at)
);

CREATE FUNCTION restrict_stage_handoff_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'stage handoffs cannot be deleted';
    END IF;
    IF OLD.invalidated_at IS NULL
        AND NEW.invalidated_at IS NOT NULL
        AND (to_jsonb(NEW) - 'invalidated_at') = (to_jsonb(OLD) - 'invalidated_at')
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'stage handoffs only allow one-way invalidation';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_handoffs_immutable
BEFORE UPDATE OR DELETE ON stage_handoffs
FOR EACH ROW EXECUTE FUNCTION restrict_stage_handoff_change();
