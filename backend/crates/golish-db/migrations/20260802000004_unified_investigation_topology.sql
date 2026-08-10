-- Operation-frozen legacy versus unified Investigation stage topology.
--
-- Existing operations are grandfathered as the exact legacy graph they were
-- created under. Every operation created after this migration derives its
-- topology from the server-owned Investigation rollout pair. Stage executions,
-- test forks, and adoption receipts copy that immutable authority instead of
-- consulting a mutable deployment default during resume.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION stage_topology_for_investigation_rollout(rollout_mode TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT CASE rollout_mode
        WHEN 'legacy_only' THEN 'legacy_candidate_verification_v1'
        WHEN 'shadow_registry' THEN 'legacy_candidate_verification_v1'
        WHEN 'dual_read_compare' THEN 'legacy_candidate_verification_v1'
        WHEN 'registry_authoritative_legacy_projection' THEN 'unified_investigation_v1'
        WHEN 'new_only' THEN 'unified_investigation_v1'
        ELSE NULL
    END
$$;

CREATE FUNCTION stage_topology_canonical_json(topology TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT CASE topology
        WHEN 'legacy_candidate_verification_v1' THEN
            '{"contract_version":"stage_topology.v1","graph_resource":"operation_graph.json","topology":"legacy_candidate_verification_v1"}'
        WHEN 'unified_investigation_v1' THEN
            '{"contract_version":"stage_topology.v1","graph_resource":"operation_graph_unified_investigation_v1.json","topology":"unified_investigation_v1"}'
        ELSE NULL
    END
$$;

CREATE FUNCTION stage_topology_contract_sha256(topology TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT CASE WHEN stage_topology_canonical_json(topology) IS NULL THEN NULL ELSE
        'sha256:' || encode(
            digest(
                convert_to('golish.stage_topology_contract.v1','UTF8')
                || decode('00','hex')
                || convert_to(stage_topology_canonical_json(topology),'UTF8'),
                'sha256'
            ),
            'hex'
        )
    END
$$;

-- This rank is topology-specific. It is not StageKind catalog order and must
-- never be called without the operation-frozen topology.
CREATE FUNCTION operation_stage_rank_for_topology(topology TEXT, stage_kind TEXT)
RETURNS SMALLINT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT CASE topology
        WHEN 'legacy_candidate_verification_v1' THEN CASE stage_kind
            WHEN 'scoping' THEN 1
            WHEN 'target_intel' THEN 2
            WHEN 'external_attack_surface' THEN 3
            WHEN 'enumeration' THEN 4
            WHEN 'vuln_triage' THEN 5
            WHEN 'attack_candidate' THEN 6
            WHEN 'verification' THEN 7
            WHEN 'access_validation' THEN 8
            WHEN 'internal_discovery' THEN 9
            WHEN 'objective_pathing' THEN 10
            WHEN 'objective_simulation' THEN 11
            WHEN 'cleanup' THEN 12
            WHEN 'reporting' THEN 13
            ELSE NULL
        END
        WHEN 'unified_investigation_v1' THEN CASE stage_kind
            WHEN 'scoping' THEN 1
            WHEN 'target_intel' THEN 2
            WHEN 'external_attack_surface' THEN 3
            WHEN 'enumeration' THEN 4
            WHEN 'vuln_triage' THEN 5
            WHEN 'application_understanding' THEN 6
            WHEN 'investigation' THEN 7
            WHEN 'reporting' THEN 8
            ELSE NULL
        END
        ELSE NULL
    END
$$;

CREATE FUNCTION operation_stage_transition_allowed(
    topology TEXT,
    from_stage TEXT,
    to_stage TEXT
)
RETURNS BOOLEAN
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT CASE
        WHEN from_stage=to_stage THEN TRUE
        WHEN topology='legacy_candidate_verification_v1' THEN
            ROW(from_stage,to_stage) IN (
                ROW('scoping','target_intel'),
                ROW('target_intel','external_attack_surface'),
                ROW('external_attack_surface','enumeration'),
                ROW('external_attack_surface','reporting'),
                ROW('enumeration','vuln_triage'),
                ROW('enumeration','reporting'),
                ROW('vuln_triage','attack_candidate'),
                ROW('vuln_triage','reporting'),
                ROW('attack_candidate','verification'),
                ROW('attack_candidate','reporting'),
                ROW('verification','access_validation'),
                ROW('verification','reporting'),
                ROW('access_validation','internal_discovery'),
                ROW('internal_discovery','objective_pathing'),
                ROW('objective_pathing','objective_simulation'),
                ROW('objective_simulation','cleanup'),
                ROW('cleanup','reporting')
            )
        WHEN topology='unified_investigation_v1' THEN
            ROW(from_stage,to_stage) IN (
                ROW('scoping','target_intel'),
                ROW('target_intel','external_attack_surface'),
                ROW('external_attack_surface','enumeration'),
                ROW('external_attack_surface','reporting'),
                ROW('enumeration','vuln_triage'),
                ROW('enumeration','reporting'),
                ROW('vuln_triage','application_understanding'),
                ROW('application_understanding','investigation'),
                ROW('investigation','reporting')
            )
        ELSE FALSE
    END
$$;

ALTER TABLE operation_state
    ADD COLUMN stage_topology_contract TEXT,
    ADD COLUMN stage_topology_canonical_json TEXT,
    ADD COLUMN stage_topology_sha256 TEXT,
    ADD COLUMN stage_topology_freeze_source TEXT;

-- Do not reinterpret already-created operations from the current rollout row.
UPDATE operation_state
SET stage_topology_contract='legacy_candidate_verification_v1',
    stage_topology_canonical_json=stage_topology_canonical_json(
        'legacy_candidate_verification_v1'
    ),
    stage_topology_sha256=stage_topology_contract_sha256(
        'legacy_candidate_verification_v1'
    ),
    stage_topology_freeze_source='legacy_backfill_v1';

ALTER TABLE operation_state
    ALTER COLUMN stage_topology_contract SET NOT NULL,
    ALTER COLUMN stage_topology_canonical_json SET NOT NULL,
    ALTER COLUMN stage_topology_sha256 SET NOT NULL,
    ALTER COLUMN stage_topology_freeze_source SET NOT NULL,
    ADD CONSTRAINT operation_state_stage_topology_contract_check CHECK (
        stage_topology_contract IN (
            'legacy_candidate_verification_v1','unified_investigation_v1'
        )
    ),
    ADD CONSTRAINT operation_state_stage_topology_material_check CHECK (
        stage_topology_canonical_json=stage_topology_canonical_json(stage_topology_contract)
        AND stage_topology_sha256=stage_topology_contract_sha256(stage_topology_contract)
    ),
    ADD CONSTRAINT operation_state_stage_topology_freeze_source_check CHECK (
        (
            stage_topology_freeze_source='legacy_backfill_v1'
            AND stage_topology_contract='legacy_candidate_verification_v1'
        )
        OR (
            stage_topology_freeze_source='deployment_pair_v1'
            AND stage_topology_contract=
                stage_topology_for_investigation_rollout(investigation_rollout_mode)
        )
    ),
    ADD CONSTRAINT operation_state_current_stage_topology_check CHECK (
        operation_stage_rank_for_topology(stage_topology_contract,current_stage) IS NOT NULL
    ),
    ADD CONSTRAINT operation_state_stage_topology_owner_unique
        UNIQUE(operation_id,stage_topology_contract);

CREATE FUNCTION freeze_operation_stage_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE selected_topology TEXT;
BEGIN
    selected_topology := stage_topology_for_investigation_rollout(
        NEW.investigation_rollout_mode
    );
    IF selected_topology IS NULL THEN
        RAISE EXCEPTION 'OPERATION_STAGE_TOPOLOGY_ROLLOUT_UNKNOWN' USING ERRCODE='23514';
    END IF;
    NEW.stage_topology_contract := selected_topology;
    NEW.stage_topology_canonical_json := stage_topology_canonical_json(selected_topology);
    NEW.stage_topology_sha256 := stage_topology_contract_sha256(selected_topology);
    NEW.stage_topology_freeze_source := 'deployment_pair_v1';
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_stage_topology_freeze
BEFORE INSERT ON operation_state
FOR EACH ROW EXECUTE FUNCTION freeze_operation_stage_topology();

CREATE FUNCTION guard_operation_stage_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF ROW(
        NEW.stage_topology_contract,
        NEW.stage_topology_canonical_json,
        NEW.stage_topology_sha256,
        NEW.stage_topology_freeze_source
    ) IS DISTINCT FROM ROW(
        OLD.stage_topology_contract,
        OLD.stage_topology_canonical_json,
        OLD.stage_topology_sha256,
        OLD.stage_topology_freeze_source
    ) THEN
        RAISE EXCEPTION 'OPERATION_STAGE_TOPOLOGY_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    -- Preserve historical legacy cursor semantics byte-for-byte. Unified
    -- operations, which did not exist before this migration, use the exact
    -- graph edge contract.
    IF OLD.stage_topology_contract='unified_investigation_v1'
       AND NEW.current_stage IS DISTINCT FROM OLD.current_stage
       AND NOT operation_stage_transition_allowed(
           OLD.stage_topology_contract,OLD.current_stage,NEW.current_stage
       )
    THEN
        RAISE EXCEPTION 'OPERATION_STAGE_TOPOLOGY_TRANSITION_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_stage_topology_immutable
BEFORE UPDATE OF stage_topology_contract,stage_topology_canonical_json,
                 stage_topology_sha256,stage_topology_freeze_source,current_stage
ON operation_state
FOR EACH ROW EXECUTE FUNCTION guard_operation_stage_topology();

-- Every stage execution copies the operation topology. The compound FK and
-- legal-stage check prevent legacy Candidate/Verification rows in a unified
-- operation and prevent unified rows in a legacy operation.
ALTER TABLE stage_runs ADD COLUMN stage_topology_contract TEXT;
UPDATE stage_runs run
SET stage_topology_contract=operation.stage_topology_contract
FROM operation_state operation
WHERE operation.operation_id=run.operation_id;
ALTER TABLE stage_runs
    ALTER COLUMN stage_topology_contract SET NOT NULL,
    ADD CONSTRAINT stage_runs_topology_owner_fk
        FOREIGN KEY(operation_id,stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT,
    ADD CONSTRAINT stage_runs_topology_stage_check CHECK (
        operation_stage_rank_for_topology(stage_topology_contract,stage_kind) IS NOT NULL
    ),
    ADD CONSTRAINT stage_runs_topology_identity_unique
        UNIQUE(id,operation_id,stage_kind,stage_topology_contract);

CREATE FUNCTION freeze_stage_run_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE selected_topology TEXT;
BEGIN
    SELECT stage_topology_contract INTO selected_topology
    FROM operation_state
    WHERE operation_id=NEW.operation_id
    FOR SHARE;
    IF selected_topology IS NULL THEN
        RAISE EXCEPTION 'STAGE_RUN_OPERATION_TOPOLOGY_MISSING' USING ERRCODE='23503';
    END IF;
    NEW.stage_topology_contract := selected_topology;
    RETURN NEW;
END;
$$;

CREATE TRIGGER stage_runs_topology_freeze
BEFORE INSERT ON stage_runs
FOR EACH ROW EXECUTE FUNCTION freeze_stage_run_topology();

CREATE FUNCTION guard_stage_run_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.stage_topology_contract IS DISTINCT FROM OLD.stage_topology_contract THEN
        RAISE EXCEPTION 'STAGE_RUN_TOPOLOGY_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER stage_runs_topology_immutable
BEFORE UPDATE OF stage_topology_contract ON stage_runs
FOR EACH ROW EXECUTE FUNCTION guard_stage_run_topology();

-- Test/recovery forks retain both source and target topology. The target entry
-- and terminal stages are evaluated only with the target operation graph.
ALTER TABLE operation_stage_forks
    ADD COLUMN source_stage_topology_contract TEXT,
    ADD COLUMN target_stage_topology_contract TEXT;
UPDATE operation_stage_forks fork
SET source_stage_topology_contract=source.stage_topology_contract,
    target_stage_topology_contract=target.stage_topology_contract
FROM operation_state source,operation_state target
WHERE source.operation_id=fork.source_operation_id
  AND target.operation_id=fork.operation_id;
ALTER TABLE operation_stage_forks
    ALTER COLUMN source_stage_topology_contract SET NOT NULL,
    ALTER COLUMN target_stage_topology_contract SET NOT NULL,
    ADD CONSTRAINT operation_stage_forks_source_topology_fk
        FOREIGN KEY(source_operation_id,source_stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT,
    ADD CONSTRAINT operation_stage_forks_target_topology_fk
        FOREIGN KEY(operation_id,target_stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT,
    ADD CONSTRAINT operation_stage_forks_topology_identity_unique
        UNIQUE(operation_id,source_operation_id,source_stage_topology_contract);

DO $$
DECLARE constraint_row RECORD;
BEGIN
    FOR constraint_row IN
        SELECT conname
        FROM pg_constraint
        WHERE conrelid='operation_stage_forks'::regclass
          AND contype='c'
          AND pg_get_constraintdef(oid) LIKE '%operation_stage_fork_stage_rank%'
    LOOP
        EXECUTE format(
            'ALTER TABLE operation_stage_forks DROP CONSTRAINT %I',
            constraint_row.conname
        );
    END LOOP;
END;
$$;

ALTER TABLE operation_stage_forks
    ADD CONSTRAINT operation_stage_forks_topology_entry_check CHECK (
        operation_stage_rank_for_topology(target_stage_topology_contract,entry_stage)>=2
        AND entry_stage NOT IN ('reporting','cleanup')
    ),
    ADD CONSTRAINT operation_stage_forks_topology_terminal_check CHECK (
        operation_stage_rank_for_topology(target_stage_topology_contract,terminal_stage)
            >= operation_stage_rank_for_topology(
                target_stage_topology_contract,entry_stage
            )
    );

CREATE FUNCTION freeze_operation_stage_fork_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT stage_topology_contract INTO NEW.source_stage_topology_contract
    FROM operation_state WHERE operation_id=NEW.source_operation_id FOR SHARE;
    SELECT stage_topology_contract INTO NEW.target_stage_topology_contract
    FROM operation_state WHERE operation_id=NEW.operation_id FOR SHARE;
    IF NEW.source_stage_topology_contract IS NULL
       OR NEW.target_stage_topology_contract IS NULL
    THEN
        RAISE EXCEPTION 'OPERATION_STAGE_FORK_TOPOLOGY_MISSING' USING ERRCODE='23503';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_stage_forks_topology_freeze
BEFORE INSERT ON operation_stage_forks
FOR EACH ROW EXECUTE FUNCTION freeze_operation_stage_fork_topology();

-- Replace the legacy header validator rather than widening its one-argument
-- rank helper.  Adopted inputs belong to the frozen source graph while the
-- entry belongs to the frozen target graph; a topology-changing adoption is
-- legal only across their shared strict prefix.
CREATE OR REPLACE FUNCTION validate_operation_stage_fork_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    source_operation operation_state%ROWTYPE;
    target_operation operation_state%ROWTYPE;
    source_snapshot operation_org_scope_snapshots%ROWTYPE;
    target_snapshot operation_org_scope_snapshots%ROWTYPE;
    stage_kind TEXT;
    source_stage_rank SMALLINT;
    target_entry_rank SMALLINT;
    previous_rank SMALLINT := 0;
BEGIN
    SELECT * INTO STRICT source_operation
      FROM operation_state
     WHERE operation_id=NEW.source_operation_id
     FOR SHARE;
    SELECT * INTO STRICT target_operation
      FROM operation_state
     WHERE operation_id=NEW.operation_id
     FOR SHARE;
    SELECT * INTO STRICT source_snapshot
      FROM operation_org_scope_snapshots
     WHERE id=NEW.source_scope_snapshot_id
       AND operation_id=NEW.source_operation_id
     FOR SHARE;
    SELECT * INTO STRICT target_snapshot
      FROM operation_org_scope_snapshots
     WHERE id=NEW.target_scope_snapshot_id
       AND operation_id=NEW.operation_id
     FOR SHARE;

    IF NEW.source_stage_topology_contract IS DISTINCT FROM
           source_operation.stage_topology_contract
       OR NEW.target_stage_topology_contract IS DISTINCT FROM
           target_operation.stage_topology_contract
    THEN
        RAISE EXCEPTION 'stage fork frozen topology mismatch';
    END IF;
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
        OR source_operation.runtime_memory_contract IS DISTINCT FROM
           NEW.source_runtime_memory_contract
        OR target_operation.runtime_memory_contract IS DISTINCT FROM
           NEW.target_runtime_memory_contract
        OR source_operation.attack_execution_contract IS DISTINCT FROM
           NEW.source_attack_execution_contract
        OR target_operation.attack_execution_contract IS DISTINCT FROM
           NEW.target_attack_execution_contract
    THEN
        RAISE EXCEPTION 'stage fork frozen operation contract mismatch';
    END IF;
    IF EXISTS (
        SELECT 1 FROM (
            SELECT organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent
              FROM operation_org_scope_units
             WHERE snapshot_id=NEW.source_scope_snapshot_id
            EXCEPT
            SELECT organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent
              FROM operation_org_scope_units
             WHERE snapshot_id=NEW.target_scope_snapshot_id
        ) AS missing_target_unit
    ) OR EXISTS (
        SELECT 1 FROM (
            SELECT organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent
              FROM operation_org_scope_units
             WHERE snapshot_id=NEW.target_scope_snapshot_id
            EXCEPT
            SELECT organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent
              FROM operation_org_scope_units
             WHERE snapshot_id=NEW.source_scope_snapshot_id
        ) AS extra_target_unit
    ) THEN
        RAISE EXCEPTION 'stage fork target scope topology differs from source';
    END IF;

    target_entry_rank := operation_stage_rank_for_topology(
        NEW.target_stage_topology_contract,NEW.entry_stage
    );
    IF target_entry_rank IS NULL THEN
        RAISE EXCEPTION 'stage fork target entry is outside its frozen topology';
    END IF;
    FOREACH stage_kind IN ARRAY NEW.adopted_stage_kinds LOOP
        source_stage_rank := operation_stage_rank_for_topology(
            NEW.source_stage_topology_contract,stage_kind
        );
        IF source_stage_rank IS NULL
            OR source_stage_rank <= previous_rank
            OR source_stage_rank >= target_entry_rank
        THEN
            RAISE EXCEPTION 'stage fork adopted stages are not a canonical strict prefix';
        END IF;
        previous_rank := source_stage_rank;
    END LOOP;
    RETURN NEW;
END;
$$;

CREATE FUNCTION guard_operation_stage_fork_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF ROW(NEW.source_stage_topology_contract,NEW.target_stage_topology_contract)
       IS DISTINCT FROM
       ROW(OLD.source_stage_topology_contract,OLD.target_stage_topology_contract)
    THEN
        RAISE EXCEPTION 'OPERATION_STAGE_FORK_TOPOLOGY_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_stage_forks_topology_immutable
BEFORE UPDATE OF source_stage_topology_contract,target_stage_topology_contract
ON operation_stage_forks
FOR EACH ROW EXECUTE FUNCTION guard_operation_stage_fork_topology();

ALTER TABLE operation_stage_fork_inputs
    ADD COLUMN source_stage_topology_contract TEXT;
UPDATE operation_stage_fork_inputs input
SET source_stage_topology_contract=fork.source_stage_topology_contract
FROM operation_stage_forks fork
WHERE fork.operation_id=input.operation_id
  AND fork.source_operation_id=input.source_operation_id;
ALTER TABLE operation_stage_fork_inputs
    ALTER COLUMN source_stage_topology_contract SET NOT NULL,
    ADD CONSTRAINT operation_stage_fork_inputs_topology_fk
        FOREIGN KEY(operation_id,source_operation_id,source_stage_topology_contract)
        REFERENCES operation_stage_forks(
            operation_id,source_operation_id,source_stage_topology_contract
        ) ON DELETE RESTRICT;

DO $$
DECLARE constraint_row RECORD;
BEGIN
    FOR constraint_row IN
        SELECT conname
        FROM pg_constraint
        WHERE conrelid='operation_stage_fork_inputs'::regclass
          AND contype='c'
          AND pg_get_constraintdef(oid) LIKE '%operation_stage_fork_stage_rank%'
    LOOP
        EXECUTE format(
            'ALTER TABLE operation_stage_fork_inputs DROP CONSTRAINT %I',
            constraint_row.conname
        );
    END LOOP;
END;
$$;

ALTER TABLE operation_stage_fork_inputs
    ADD CONSTRAINT operation_stage_fork_inputs_topology_stage_check CHECK (
        operation_stage_rank_for_topology(
            source_stage_topology_contract,source_stage_kind
        ) IS NOT NULL
    );

CREATE FUNCTION freeze_operation_stage_fork_input_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT source_stage_topology_contract
    INTO NEW.source_stage_topology_contract
    FROM operation_stage_forks
    WHERE operation_id=NEW.operation_id
      AND source_operation_id=NEW.source_operation_id
    FOR SHARE;
    IF NEW.source_stage_topology_contract IS NULL THEN
        RAISE EXCEPTION 'OPERATION_STAGE_FORK_INPUT_TOPOLOGY_MISSING' USING ERRCODE='23503';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_stage_fork_inputs_topology_freeze
BEFORE INSERT ON operation_stage_fork_inputs
FOR EACH ROW EXECUTE FUNCTION freeze_operation_stage_fork_input_topology();

-- The target protection trigger predates topology freezing.  Keep the same
-- identity/scope protection but derive its active-stage threshold from the
-- fork-owned target graph instead of the ambiguous legacy rank helper.
CREATE OR REPLACE FUNCTION protect_active_operation_stage_fork_target()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM operation_stage_fork_targets AS frozen_target
          JOIN operation_stage_forks AS fork
            ON fork.operation_id=frozen_target.operation_id
           AND operation_stage_rank_for_topology(
                   fork.target_stage_topology_contract,fork.entry_stage
               ) >= 3
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
$$;

-- Promotion/adoption receipts copy both immutable operation authorities. The
-- Rust receipt hash is extended in the repository task before production use.
ALTER TABLE operation_contract_adoptions
    ADD COLUMN source_stage_topology_contract TEXT,
    ADD COLUMN target_stage_topology_contract TEXT;
UPDATE operation_contract_adoptions adoption
SET source_stage_topology_contract=source.stage_topology_contract,
    target_stage_topology_contract=target.stage_topology_contract
FROM operation_state source,operation_state target
WHERE source.operation_id=adoption.source_operation_id
  AND target.operation_id=adoption.target_operation_id;
ALTER TABLE operation_contract_adoptions
    ALTER COLUMN source_stage_topology_contract SET NOT NULL,
    ALTER COLUMN target_stage_topology_contract SET NOT NULL,
    ADD CONSTRAINT operation_contract_adoptions_source_topology_fk
        FOREIGN KEY(source_operation_id,source_stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT,
    ADD CONSTRAINT operation_contract_adoptions_target_topology_fk
        FOREIGN KEY(target_operation_id,target_stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT;

CREATE FUNCTION freeze_operation_contract_adoption_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT stage_topology_contract INTO NEW.source_stage_topology_contract
    FROM operation_state WHERE operation_id=NEW.source_operation_id FOR SHARE;
    SELECT stage_topology_contract INTO NEW.target_stage_topology_contract
    FROM operation_state WHERE operation_id=NEW.target_operation_id FOR SHARE;
    IF NEW.source_stage_topology_contract IS NULL
       OR NEW.target_stage_topology_contract IS NULL
    THEN
        RAISE EXCEPTION 'OPERATION_CONTRACT_ADOPTION_TOPOLOGY_MISSING' USING ERRCODE='23503';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_contract_adoptions_topology_freeze
BEFORE INSERT ON operation_contract_adoptions
FOR EACH ROW EXECUTE FUNCTION freeze_operation_contract_adoption_topology();

CREATE TRIGGER operation_contract_adoptions_topology_immutable
BEFORE UPDATE OF source_stage_topology_contract,target_stage_topology_contract
ON operation_contract_adoptions
FOR EACH ROW EXECUTE FUNCTION guard_operation_stage_fork_topology();
