-- Immutable source-operation lineage for CLI/GUI stage-test forks.
--
-- A fork creates a new operation and never mutates or copies source runtime
-- rows.  These tables retain only the exact final-seal authorities adopted by
-- the new operation plus a creation-time snapshot of its live Targets.  The
-- runtime resolver and Candidate-specific entry/evidence rules are added by
-- later migrations/tasks; this migration establishes the additive ownership
-- spine they reference.

CREATE FUNCTION operation_stage_fork_stage_rank(stage_kind TEXT)
RETURNS SMALLINT AS $$
    SELECT CASE stage_kind
        WHEN 'scoping' THEN 1
        WHEN 'target_intel' THEN 2
        WHEN 'external_attack_surface' THEN 3
        WHEN 'enumeration' THEN 4
        WHEN 'vuln_triage' THEN 5
        WHEN 'attack_candidate' THEN 6
        ELSE NULL
    END
$$ LANGUAGE sql IMMUTABLE PARALLEL SAFE;

CREATE TABLE operation_stage_forks (
    operation_id UUID PRIMARY KEY
        REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    source_operation_id UUID NOT NULL
        REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL
        REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    source_scope_snapshot_id UUID NOT NULL,
    target_scope_snapshot_id UUID NOT NULL UNIQUE,
    source_profile TEXT NOT NULL,
    target_profile TEXT NOT NULL,
    source_runtime_memory_contract TEXT NOT NULL,
    target_runtime_memory_contract TEXT NOT NULL,
    source_attack_execution_contract TEXT NOT NULL,
    target_attack_execution_contract TEXT NOT NULL,
    entry_stage TEXT NOT NULL,
    terminal_stage TEXT NOT NULL,
    adopted_stage_kinds TEXT[] NOT NULL,
    expected_input_count INTEGER NOT NULL CHECK (expected_input_count > 0),
    expected_target_count INTEGER NOT NULL CHECK (expected_target_count >= 0),
    manifest JSONB NOT NULL,
    manifest_sha256 TEXT NOT NULL CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, source_operation_id),
    UNIQUE (operation_id, target_scope_snapshot_id),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_scope_snapshot_id, source_operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    CHECK (operation_id <> source_operation_id),
    CHECK (source_profile = target_profile),
    CHECK (source_runtime_memory_contract = target_runtime_memory_contract),
    CHECK (source_attack_execution_contract = target_attack_execution_contract),
    CHECK (operation_stage_fork_stage_rank(entry_stage) BETWEEN 2 AND 6),
    CHECK (
        operation_stage_fork_stage_rank(terminal_stage)
            BETWEEN operation_stage_fork_stage_rank(entry_stage) AND 6
    ),
    CHECK (cardinality(adopted_stage_kinds) > 0),
    CHECK (adopted_stage_kinds[1] = 'scoping'),
    CHECK (NOT entry_stage = ANY(adopted_stage_kinds))
);

CREATE TABLE operation_stage_fork_inputs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    source_operation_id UUID NOT NULL,
    source_scope_snapshot_id UUID NOT NULL,
    target_scope_snapshot_id UUID NOT NULL,
    source_stage_kind TEXT NOT NULL,
    organization_id UUID NOT NULL,
    source_stage_execution_id UUID NOT NULL,
    source_stage_run_unit_id UUID NOT NULL,
    source_worker_run_id UUID,
    source_deliverable_submission_id UUID NOT NULL,
    source_handoff_id UUID,
    source_scope_hash TEXT NOT NULL CHECK (source_scope_hash ~ '^[0-9a-f]{64}$'),
    source_payload JSONB NOT NULL,
    source_payload_sha256 TEXT NOT NULL CHECK (source_payload_sha256 ~ '^[0-9a-f]{64}$'),
    source_evidence_ids BIGINT[] NOT NULL DEFAULT '{}',
    source_coverage_watermark JSONB NOT NULL,
    source_unit_gate_decision_hash TEXT NOT NULL
        CHECK (source_unit_gate_decision_hash ~ '^[0-9a-f]{64}$'),
    source_aggregate_pass_token_hash TEXT
        CHECK (
            source_aggregate_pass_token_hash IS NULL
            OR source_aggregate_pass_token_hash ~ '^[0-9a-f]{64}$'
        ),
    source_gate_passed_at TIMESTAMPTZ NOT NULL,
    manifest_input_sha256 TEXT NOT NULL CHECK (manifest_input_sha256 ~ '^[0-9a-f]{64}$'),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, source_stage_kind, organization_id),
    UNIQUE (id, operation_id, source_stage_kind, organization_id),
    FOREIGN KEY (operation_id, source_operation_id)
        REFERENCES operation_stage_forks(operation_id, source_operation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (source_scope_snapshot_id, source_operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_stage_execution_id, source_operation_id, source_stage_kind)
        REFERENCES stage_runs(id, operation_id, stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_stage_run_unit_id,
        source_operation_id,
        source_stage_execution_id,
        organization_id,
        source_stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_deliverable_submission_id,
        source_operation_id,
        source_stage_execution_id,
        source_stage_run_unit_id,
        organization_id,
        source_stage_kind
    ) REFERENCES stage_deliverable_submissions(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_worker_run_id,
        source_operation_id,
        source_stage_execution_id,
        source_stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) MATCH SIMPLE ON DELETE RESTRICT,
    FOREIGN KEY (source_handoff_id)
        REFERENCES stage_handoffs(id) ON DELETE RESTRICT,
    CHECK (operation_stage_fork_stage_rank(source_stage_kind) BETWEEN 1 AND 5),
    CHECK (
        (
            source_stage_kind = 'scoping'
            AND source_worker_run_id IS NULL
            AND source_handoff_id IS NULL
        )
        OR
        (
            source_stage_kind <> 'scoping'
            AND source_worker_run_id IS NOT NULL
            AND source_handoff_id IS NOT NULL
        )
    )
);

CREATE INDEX operation_stage_fork_inputs_source_lookup
    ON operation_stage_fork_inputs(
        source_operation_id,
        source_stage_kind,
        organization_id
    );

CREATE INDEX operation_stage_fork_inputs_evidence_gin
    ON operation_stage_fork_inputs USING GIN(source_evidence_ids);

CREATE TABLE operation_stage_fork_targets (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    live_target_id UUID NOT NULL,
    target_name_at_fork TEXT NOT NULL,
    target_type_at_fork TEXT NOT NULL,
    target_value_at_fork TEXT NOT NULL,
    target_scope_at_fork TEXT NOT NULL CHECK (target_scope_at_fork IN ('in', 'out')),
    target_source_at_fork TEXT NOT NULL,
    project_path_at_fork TEXT NOT NULL,
    canonical_identity_sha256 TEXT NOT NULL
        CHECK (canonical_identity_sha256 ~ '^[0-9a-f]{64}$'),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    frozen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, organization_id, ordinal),
    UNIQUE (operation_id, live_target_id),
    FOREIGN KEY (operation_id, scope_snapshot_id)
        REFERENCES operation_stage_forks(operation_id, target_scope_snapshot_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
        ON DELETE RESTRICT
);

CREATE INDEX operation_stage_fork_targets_scope_lookup
    ON operation_stage_fork_targets(operation_id, organization_id, target_scope_at_fork);

CREATE FUNCTION validate_operation_stage_fork_header()
RETURNS trigger AS $$
DECLARE
    source_operation operation_state%ROWTYPE;
    target_operation operation_state%ROWTYPE;
    source_snapshot operation_org_scope_snapshots%ROWTYPE;
    target_snapshot operation_org_scope_snapshots%ROWTYPE;
    stage_kind TEXT;
    stage_rank SMALLINT;
    previous_rank SMALLINT := 0;
BEGIN
    SELECT * INTO STRICT source_operation
      FROM operation_state
     WHERE operation_id = NEW.source_operation_id
     FOR SHARE;
    SELECT * INTO STRICT target_operation
      FROM operation_state
     WHERE operation_id = NEW.operation_id
     FOR SHARE;
    SELECT * INTO STRICT source_snapshot
      FROM operation_org_scope_snapshots
     WHERE id = NEW.source_scope_snapshot_id
       AND operation_id = NEW.source_operation_id
     FOR SHARE;
    SELECT * INTO STRICT target_snapshot
      FROM operation_org_scope_snapshots
     WHERE id = NEW.target_scope_snapshot_id
       AND operation_id = NEW.operation_id
     FOR SHARE;

    IF source_operation.superseded_by IS NOT NULL THEN
        RAISE EXCEPTION 'stage fork source operation is superseded';
    END IF;
    IF source_operation.project_scope_id IS DISTINCT FROM NEW.project_scope_id
        OR target_operation.project_scope_id IS DISTINCT FROM NEW.project_scope_id
        OR source_snapshot.project_scope_id IS DISTINCT FROM NEW.project_scope_id
        OR target_snapshot.project_scope_id IS DISTINCT FROM NEW.project_scope_id
    THEN
        RAISE EXCEPTION 'stage fork project scope mismatch';
    END IF;
    IF source_snapshot.sealed_at IS NULL OR target_snapshot.sealed_at IS NULL THEN
        RAISE EXCEPTION 'stage fork requires sealed source and target scopes';
    END IF;
    IF target_snapshot.mode <> 'reuse_reconfirmed'
        OR source_snapshot.root_organization_id <> target_snapshot.root_organization_id
    THEN
        RAISE EXCEPTION 'stage fork target scope is not a reconfirmed source clone';
    END IF;
    IF source_operation.profile IS DISTINCT FROM NEW.source_profile
        OR target_operation.profile IS DISTINCT FROM NEW.target_profile
        OR source_operation.runtime_memory_contract IS DISTINCT FROM NEW.source_runtime_memory_contract
        OR target_operation.runtime_memory_contract IS DISTINCT FROM NEW.target_runtime_memory_contract
        OR source_operation.attack_execution_contract IS DISTINCT FROM NEW.source_attack_execution_contract
        OR target_operation.attack_execution_contract IS DISTINCT FROM NEW.target_attack_execution_contract
    THEN
        RAISE EXCEPTION 'stage fork frozen operation contract mismatch';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM (
                SELECT organization_id, parent_organization_id,
                       organization_name_at_freeze, role, depth, ordinal,
                       ownership_percent
                  FROM operation_org_scope_units
                 WHERE snapshot_id = NEW.source_scope_snapshot_id
                EXCEPT
                SELECT organization_id, parent_organization_id,
                       organization_name_at_freeze, role, depth, ordinal,
                       ownership_percent
                  FROM operation_org_scope_units
                 WHERE snapshot_id = NEW.target_scope_snapshot_id
          ) AS missing_target_unit
    ) OR EXISTS (
        SELECT 1
          FROM (
                SELECT organization_id, parent_organization_id,
                       organization_name_at_freeze, role, depth, ordinal,
                       ownership_percent
                  FROM operation_org_scope_units
                 WHERE snapshot_id = NEW.target_scope_snapshot_id
                EXCEPT
                SELECT organization_id, parent_organization_id,
                       organization_name_at_freeze, role, depth, ordinal,
                       ownership_percent
                  FROM operation_org_scope_units
                 WHERE snapshot_id = NEW.source_scope_snapshot_id
          ) AS extra_target_unit
    ) THEN
        RAISE EXCEPTION 'stage fork target scope topology differs from source';
    END IF;

    FOREACH stage_kind IN ARRAY NEW.adopted_stage_kinds LOOP
        stage_rank := operation_stage_fork_stage_rank(stage_kind);
        IF stage_rank IS NULL
            OR stage_rank <= previous_rank
            OR stage_rank >= operation_stage_fork_stage_rank(NEW.entry_stage)
        THEN
            RAISE EXCEPTION 'stage fork adopted stages are not a canonical strict prefix';
        END IF;
        previous_rank := stage_rank;
    END LOOP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_stage_forks_validate_header
BEFORE INSERT ON operation_stage_forks
FOR EACH ROW EXECUTE FUNCTION validate_operation_stage_fork_header();

CREATE FUNCTION validate_operation_stage_fork_input()
RETURNS trigger AS $$
DECLARE
    fork operation_stage_forks%ROWTYPE;
    source_row RECORD;
    source_role TEXT;
BEGIN
    SELECT * INTO STRICT fork
      FROM operation_stage_forks
     WHERE operation_id = NEW.operation_id
       AND source_operation_id = NEW.source_operation_id
     FOR SHARE;
    IF NEW.source_scope_snapshot_id <> fork.source_scope_snapshot_id
        OR NEW.target_scope_snapshot_id <> fork.target_scope_snapshot_id
        OR NOT NEW.source_stage_kind = ANY(fork.adopted_stage_kinds)
    THEN
        RAISE EXCEPTION 'stage fork input is outside its immutable header';
    END IF;

    SELECT unit.role INTO STRICT source_role
      FROM operation_org_scope_units AS unit
     WHERE unit.snapshot_id = NEW.source_scope_snapshot_id
       AND unit.organization_id = NEW.organization_id;
    IF NEW.source_stage_kind = 'scoping' AND source_role <> 'root' THEN
        RAISE EXCEPTION 'stage fork Scoping input must be root-only';
    END IF;

    IF NEW.source_stage_kind = 'scoping' THEN
        SELECT run.status AS execution_status,
               unit.status AS unit_status,
               submission.worker_run_id AS submission_worker_run_id,
               snapshot.scope_hash,
               jsonb_build_object(
                   'schema_version', 1,
                   'scope_decision_id', decision.id,
                   'scope_snapshot_id', snapshot.id,
                   'project_scope_id', snapshot.project_scope_id,
                   'root_organization_id', snapshot.root_organization_id,
                   'decision_hash', decision.decision_hash,
                   'scope_hash', snapshot.scope_hash,
                   'decision_rows', decision.decision_rows,
                   'scope_units', (
                       SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'organization_id', scope_unit.organization_id,
                                      'parent_organization_id', scope_unit.parent_organization_id,
                                      'organization_name_at_freeze', scope_unit.organization_name_at_freeze,
                                      'role', scope_unit.role,
                                      'depth', scope_unit.depth,
                                      'ordinal', scope_unit.ordinal,
                                      'ownership_percent', scope_unit.ownership_percent,
                                      'decision_row_id', scope_unit.decision_row_id,
                                      'approval_source', scope_unit.approval_source
                                  ) ORDER BY scope_unit.ordinal
                              )
                         FROM operation_org_scope_units AS scope_unit
                        WHERE scope_unit.snapshot_id=snapshot.id
                   )
               ) AS payload,
               decision.decision_hash AS unit_gate_decision_hash,
               snapshot.sealed_at AS gate_passed_at
          INTO STRICT source_row
          FROM operation_org_scope_snapshots AS snapshot
          JOIN operation_scope_decisions AS decision
            ON decision.id=snapshot.scope_decision_id
           AND decision.operation_id=snapshot.operation_id
           AND decision.project_scope_id=snapshot.project_scope_id
          JOIN stage_runs AS run
            ON run.id=decision.stage_execution_id
           AND run.operation_id=snapshot.operation_id
           AND run.stage_kind='scoping'
          JOIN stage_run_units AS unit
            ON unit.id=NEW.source_stage_run_unit_id
           AND unit.operation_id=snapshot.operation_id
           AND unit.stage_execution_id=decision.stage_execution_id
           AND unit.scope_snapshot_id=snapshot.id
           AND unit.organization_id=snapshot.root_organization_id
           AND unit.stage_kind='scoping'
          JOIN stage_deliverable_submissions AS submission
            ON submission.id=NEW.source_deliverable_submission_id
           AND submission.operation_id=snapshot.operation_id
           AND submission.stage_execution_id=decision.stage_execution_id
           AND submission.stage_run_unit_id=unit.id
           AND submission.organization_id=snapshot.root_organization_id
           AND submission.stage_kind='scoping'
         WHERE snapshot.id=NEW.source_scope_snapshot_id
           AND snapshot.operation_id=NEW.source_operation_id
           AND snapshot.root_organization_id=NEW.organization_id
           AND snapshot.sealed_at IS NOT NULL
         FOR SHARE OF snapshot,decision,run,unit,submission;

        IF NEW.source_handoff_id IS NOT NULL
            OR source_row.execution_status <> 'completed'
            OR source_row.unit_status <> 'passed'
            OR source_row.submission_worker_run_id IS NOT NULL
            OR source_row.scope_hash IS DISTINCT FROM NEW.source_scope_hash
            OR source_row.payload IS DISTINCT FROM NEW.source_payload
            OR attack_fact_delta_sha256_jsonb(source_row.payload)
                IS DISTINCT FROM NEW.source_payload_sha256
            OR NEW.source_evidence_ids <> '{}'::BIGINT[]
            OR NEW.source_coverage_watermark IS DISTINCT FROM
               jsonb_build_object(
                   'scope_snapshot_id', NEW.source_scope_snapshot_id,
                   'scope_hash', source_row.scope_hash,
                   'sealed_at', source_row.gate_passed_at
               )
            OR source_row.unit_gate_decision_hash
                IS DISTINCT FROM NEW.source_unit_gate_decision_hash
            OR NEW.source_aggregate_pass_token_hash IS NOT NULL
            OR source_row.gate_passed_at IS DISTINCT FROM NEW.source_gate_passed_at
        THEN
            RAISE EXCEPTION 'stage fork Scoping input does not match its exact sealed scope';
        END IF;
        RETURN NEW;
    END IF;

    SELECT run.status AS execution_status,
           unit.status AS unit_status,
           submission.worker_run_id AS submission_worker_run_id,
           worker.status AS worker_status,
           handoff.operation_id AS handoff_operation_id,
           handoff.scope_snapshot_id AS handoff_scope_snapshot_id,
           handoff.organization_id AS handoff_organization_id,
           handoff.from_stage_kind AS handoff_stage_kind,
           handoff.stage_execution_id AS handoff_stage_execution_id,
           handoff.source_stage_run_unit_id AS handoff_unit_id,
           handoff.deliverable_submission_id AS handoff_submission_id,
           handoff.scope_hash,
           handoff.payload,
           handoff.payload_sha256,
           handoff.evidence_ids,
           handoff.coverage_watermark,
           handoff.unit_gate_decision_hash,
           handoff.aggregate_pass_token_hash,
           handoff.gate_passed_at,
           handoff.invalidated_at
      INTO STRICT source_row
      FROM stage_handoffs AS handoff
      JOIN stage_runs AS run
        ON run.id = handoff.stage_execution_id
       AND run.operation_id = handoff.operation_id
       AND run.stage_kind = handoff.from_stage_kind
      JOIN stage_run_units AS unit
        ON unit.id = handoff.source_stage_run_unit_id
       AND unit.operation_id = handoff.operation_id
       AND unit.stage_execution_id = handoff.stage_execution_id
       AND unit.organization_id = handoff.organization_id
       AND unit.stage_kind = handoff.from_stage_kind
      JOIN stage_deliverable_submissions AS submission
        ON submission.id = handoff.deliverable_submission_id
       AND submission.operation_id = handoff.operation_id
       AND submission.stage_execution_id = handoff.stage_execution_id
       AND submission.stage_run_unit_id = handoff.source_stage_run_unit_id
       AND submission.organization_id = handoff.organization_id
       AND submission.stage_kind = handoff.from_stage_kind
      LEFT JOIN stage_worker_runs AS worker
        ON worker.id = submission.worker_run_id
       AND worker.operation_id = handoff.operation_id
       AND worker.stage_execution_id = handoff.stage_execution_id
       AND worker.stage_run_unit_id = handoff.source_stage_run_unit_id
       AND worker.organization_id = handoff.organization_id
     WHERE handoff.id = NEW.source_handoff_id
     FOR SHARE OF handoff, run, unit, submission;

    IF source_row.execution_status <> 'completed'
        OR source_row.unit_status <> 'passed'
        OR source_row.invalidated_at IS NOT NULL
        OR source_row.handoff_operation_id <> NEW.source_operation_id
        OR source_row.handoff_scope_snapshot_id <> NEW.source_scope_snapshot_id
        OR source_row.handoff_organization_id <> NEW.organization_id
        OR source_row.handoff_stage_kind <> NEW.source_stage_kind
        OR source_row.handoff_stage_execution_id <> NEW.source_stage_execution_id
        OR source_row.handoff_unit_id <> NEW.source_stage_run_unit_id
        OR source_row.handoff_submission_id <> NEW.source_deliverable_submission_id
        OR source_row.submission_worker_run_id IS DISTINCT FROM NEW.source_worker_run_id
        OR source_row.worker_status IS DISTINCT FROM 'passed'
        OR source_row.scope_hash IS DISTINCT FROM NEW.source_scope_hash
        OR source_row.payload IS DISTINCT FROM NEW.source_payload
        OR source_row.payload_sha256 IS DISTINCT FROM NEW.source_payload_sha256
        OR source_row.evidence_ids IS DISTINCT FROM NEW.source_evidence_ids
        OR source_row.coverage_watermark IS DISTINCT FROM NEW.source_coverage_watermark
        OR source_row.unit_gate_decision_hash IS DISTINCT FROM NEW.source_unit_gate_decision_hash
        OR source_row.aggregate_pass_token_hash IS DISTINCT FROM NEW.source_aggregate_pass_token_hash
        OR source_row.gate_passed_at IS DISTINCT FROM NEW.source_gate_passed_at
    THEN
        RAISE EXCEPTION 'stage fork input does not match an exact live final seal';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_stage_fork_inputs_validate_source
BEFORE INSERT ON operation_stage_fork_inputs
FOR EACH ROW EXECUTE FUNCTION validate_operation_stage_fork_input();

CREATE FUNCTION validate_operation_stage_fork_target()
RETURNS trigger AS $$
DECLARE
    fork operation_stage_forks%ROWTYPE;
    canonical_project_path TEXT;
    live_target RECORD;
BEGIN
    SELECT * INTO STRICT fork
      FROM operation_stage_forks
     WHERE operation_id = NEW.operation_id
       AND target_scope_snapshot_id = NEW.scope_snapshot_id
     FOR SHARE;
    SELECT project.canonical_project_path INTO STRICT canonical_project_path
      FROM project_scopes AS project
     WHERE project.project_scope_id = fork.project_scope_id
     FOR SHARE;
    SELECT name,
           target_type::TEXT AS target_type,
           value,
           scope::TEXT AS target_scope,
           source,
           project_path,
           organization_id
      INTO STRICT live_target
      FROM targets
     WHERE id = NEW.live_target_id
     FOR SHARE;
    IF live_target.organization_id IS DISTINCT FROM NEW.organization_id
        OR live_target.project_path IS DISTINCT FROM canonical_project_path
        OR live_target.name IS DISTINCT FROM NEW.target_name_at_fork
        OR live_target.target_type IS DISTINCT FROM NEW.target_type_at_fork
        OR live_target.value IS DISTINCT FROM NEW.target_value_at_fork
        OR live_target.target_scope IS DISTINCT FROM NEW.target_scope_at_fork
        OR live_target.source IS DISTINCT FROM NEW.target_source_at_fork
        OR live_target.project_path IS DISTINCT FROM NEW.project_path_at_fork
    THEN
        RAISE EXCEPTION 'stage fork Target snapshot does not match the current database row';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_stage_fork_targets_validate_live_row
BEFORE INSERT ON operation_stage_fork_targets
FOR EACH ROW EXECUTE FUNCTION validate_operation_stage_fork_target();

-- A fork that enters an active stage consumes the creation-time identity and
-- scope of each snapshotted Target. Recon enrichment columns may continue to
-- change, and a TargetIntel-entry fork may legitimately establish a new
-- denominator. From EAS onward, however, changing/deleting the frozen target
-- identity while the fork is non-terminal would make tool reads diverge from
-- the manifest that authorized dispatch.
CREATE FUNCTION protect_active_operation_stage_fork_target()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM operation_stage_fork_targets AS frozen_target
          JOIN operation_stage_forks AS fork
            ON fork.operation_id=frozen_target.operation_id
           AND operation_stage_fork_stage_rank(fork.entry_stage) >= 3
          JOIN tasks AS task
            ON task.id=fork.operation_id
           AND task.status IN ('created','running','waiting')
         WHERE frozen_target.live_target_id=OLD.id
    ) AND (
        TG_OP='DELETE'
        OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
        OR NEW.name IS DISTINCT FROM OLD.name
        OR NEW.target_type IS DISTINCT FROM OLD.target_type
        OR NEW.value IS DISTINCT FROM OLD.value
        OR NEW.scope IS DISTINCT FROM OLD.scope
        OR NEW.source IS DISTINCT FROM OLD.source
        OR NEW.project_path IS DISTINCT FROM OLD.project_path
    ) THEN
        RAISE EXCEPTION 'active stage fork Target identity/scope is frozen';
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER targets_protect_active_operation_stage_fork
BEFORE UPDATE OF organization_id,name,target_type,value,scope,source,project_path OR DELETE
ON targets
FOR EACH ROW EXECUTE FUNCTION protect_active_operation_stage_fork_target();

CREATE FUNCTION reject_operation_stage_fork_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'operation stage fork rows are immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_stage_forks_immutable
BEFORE UPDATE OR DELETE ON operation_stage_forks
FOR EACH ROW EXECUTE FUNCTION reject_operation_stage_fork_change();

CREATE TRIGGER operation_stage_fork_inputs_immutable
BEFORE UPDATE OR DELETE ON operation_stage_fork_inputs
FOR EACH ROW EXECUTE FUNCTION reject_operation_stage_fork_change();

CREATE TRIGGER operation_stage_fork_targets_immutable
BEFORE UPDATE OR DELETE ON operation_stage_fork_targets
FOR EACH ROW EXECUTE FUNCTION reject_operation_stage_fork_change();

-- The header is inserted before its children, but all rows are created by one
-- caller-owned transaction.  Validate the declared cardinalities and the
-- no-hole stage/org matrix at commit, after the children are visible.
CREATE FUNCTION validate_operation_stage_fork_complete()
RETURNS trigger AS $$
DECLARE
    actual_input_count INTEGER;
    actual_target_count INTEGER;
BEGIN
    SELECT COUNT(*)::INTEGER INTO actual_input_count
      FROM operation_stage_fork_inputs
     WHERE operation_id = NEW.operation_id;
    SELECT COUNT(*)::INTEGER INTO actual_target_count
      FROM operation_stage_fork_targets
     WHERE operation_id = NEW.operation_id;
    IF actual_input_count <> NEW.expected_input_count
        OR actual_target_count <> NEW.expected_target_count
    THEN
        RAISE EXCEPTION 'stage fork materialization count mismatch';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM unnest(NEW.adopted_stage_kinds) AS adopted(stage_kind)
          JOIN operation_org_scope_units AS unit
            ON unit.snapshot_id = NEW.source_scope_snapshot_id
           AND (adopted.stage_kind <> 'scoping' OR unit.role = 'root')
          LEFT JOIN operation_stage_fork_inputs AS input
            ON input.operation_id = NEW.operation_id
           AND input.source_stage_kind = adopted.stage_kind
           AND input.organization_id = unit.organization_id
         WHERE input.id IS NULL
    ) THEN
        RAISE EXCEPTION 'stage fork adopted prefix contains a missing organization final seal';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER operation_stage_forks_complete_at_commit
AFTER INSERT ON operation_stage_forks
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_operation_stage_fork_complete();
