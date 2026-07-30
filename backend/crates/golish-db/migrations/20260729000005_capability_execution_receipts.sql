-- Plan A: Tool Truth Coverage Contract.
--
-- This migration is deliberately additive.  Tool Truth rows are audit truth:
-- once any receipt/evidence/business authority has been written, correction is
-- by forward migration only.  A down migration must never delete that truth.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ---------------------------------------------------------------------------
-- Shared helpers
-- ---------------------------------------------------------------------------

CREATE FUNCTION tool_truth_sha256(value TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT 'sha256:' || encode(digest(convert_to(value, 'UTF8'), 'sha256'), 'hex')
$$;

CREATE FUNCTION tool_truth_reject_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION '%', TG_ARGV[0] USING ERRCODE = '23514';
END;
$$;

CREATE FUNCTION tool_truth_reject_append_only()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'tool_truth_append_only' USING ERRCODE = '23514';
END;
$$;

CREATE FUNCTION tool_truth_guard_set_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    min_ordinal BIGINT;
    max_ordinal BIGINT;
    actual_hash TEXT;
    allow_empty BOOLEAN := TG_ARGV[3]::BOOLEAN;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.sealed_at IS NOT NULL THEN
            RAISE EXCEPTION 'tool_truth_unsealed_authority' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'DELETE' OR OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE = '23514';
    END IF;

    IF NEW.sealed_empty IS DISTINCT FROM OLD.sealed_empty THEN
        RAISE EXCEPTION 'tool_truth_sealed_empty_forged' USING ERRCODE = '23514';
    END IF;
    IF (to_jsonb(NEW) - ARRAY['sealed_at','member_count','member_set_hash','sealed_empty'])
        IS DISTINCT FROM
       (to_jsonb(OLD) - ARRAY['sealed_at','member_count','member_set_hash','sealed_empty'])
    THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE = '23514';
    END IF;

    EXECUTE format(
        'SELECT count(*)::bigint, coalesce(min(ordinal),0)::bigint, '
        || 'coalesce(max(ordinal),-1)::bigint, '
        || 'tool_truth_sha256(coalesce(jsonb_agg(%I ORDER BY ordinal), ''[]''::jsonb)::text) '
        || 'FROM %I WHERE %I=$1',
        TG_ARGV[2], TG_ARGV[0], TG_ARGV[1]
    ) INTO actual_count, min_ordinal, max_ordinal, actual_hash USING NEW.id;

    IF actual_count = 0 AND NOT allow_empty THEN
        RAISE EXCEPTION 'tool_truth_set_empty_invalid' USING ERRCODE = '23514';
    END IF;
    IF actual_count > 0 AND (min_ordinal <> 0 OR max_ordinal <> actual_count - 1) THEN
        RAISE EXCEPTION 'tool_truth_set_ordinal_invalid' USING ERRCODE = '23514';
    END IF;
    IF NEW.member_count IS NOT NULL AND NEW.member_count IS DISTINCT FROM actual_count THEN
        RAISE EXCEPTION 'tool_truth_member_count_forged' USING ERRCODE = '23514';
    END IF;
    IF NEW.member_set_hash IS NOT NULL AND NEW.member_set_hash IS DISTINCT FROM actual_hash THEN
        RAISE EXCEPTION 'tool_truth_member_set_hash_forged' USING ERRCODE = '23514';
    END IF;

    NEW.member_count := actual_count;
    NEW.member_set_hash := actual_hash;
    NEW.sealed_empty := actual_count=0;
    NEW.sealed_at := statement_timestamp();
    RETURN NEW;
END;
$$;

CREATE FUNCTION tool_truth_guard_set_member()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    parent_id UUID;
    parent_sealed_at TIMESTAMPTZ;
    parent_exists BOOLEAN := FALSE;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'tool_truth_member_append_only' USING ERRCODE = '23514';
    END IF;
    parent_id := (to_jsonb(NEW)->>TG_ARGV[2])::UUID;
    EXECUTE format('SELECT sealed_at, TRUE FROM %I WHERE %I=$1 FOR SHARE', TG_ARGV[0], TG_ARGV[1])
       INTO parent_sealed_at, parent_exists USING parent_id;
    IF NOT parent_exists THEN
        RETURN NEW;
    END IF;
    IF parent_sealed_at IS NOT NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

-- ---------------------------------------------------------------------------
-- Deployment contract: permanently legacy until a future forward migration.
-- ---------------------------------------------------------------------------

CREATE TABLE tool_truth_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    new_operation_contract TEXT NOT NULL DEFAULT 'legacy_v1',
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_rollout_singleton_check CHECK (singleton),
    CONSTRAINT tool_truth_rollout_contract_check CHECK (
        new_operation_contract IN ('legacy_v1','shadow_v1','receipt_v1')
    )
);

INSERT INTO tool_truth_rollout(singleton,new_operation_contract)
VALUES(TRUE,'legacy_v1');

CREATE FUNCTION tool_truth_reject_rollout_direct_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.singleton IS DISTINCT FROM TRUE THEN
            RAISE EXCEPTION 'tool_truth_rollout_singleton_check' USING ERRCODE = '23514';
        END IF;
        RAISE EXCEPTION 'tool_truth_rollout_singleton_already_seeded' USING ERRCODE = '23505';
    END IF;
    RAISE EXCEPTION 'tool_truth_rollout_direct_mutation_forbidden' USING ERRCODE = '23514';
END;
$$;

CREATE TRIGGER tool_truth_rollout_direct_mutation_guard
BEFORE INSERT OR UPDATE OR DELETE ON tool_truth_rollout
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_rollout_direct_mutation();

ALTER TABLE operation_state ADD COLUMN tool_truth_contract TEXT;
UPDATE operation_state SET tool_truth_contract='legacy_v1' WHERE tool_truth_contract IS NULL;
ALTER TABLE operation_state
    ALTER COLUMN tool_truth_contract SET DEFAULT 'legacy_v1',
    ALTER COLUMN tool_truth_contract SET NOT NULL,
    ADD CONSTRAINT operation_state_tool_truth_contract_check CHECK (
        tool_truth_contract IN ('legacy_v1','shadow_v1','receipt_v1')
    );

CREATE FUNCTION tool_truth_validate_operation_contract()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE deployed TEXT;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.tool_truth_contract IS DISTINCT FROM OLD.tool_truth_contract THEN
            RAISE EXCEPTION 'operation_tool_truth_contract_immutable' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    SELECT new_operation_contract INTO deployed FROM tool_truth_rollout WHERE singleton;
    IF NEW.tool_truth_contract IS DISTINCT FROM deployed THEN
        RAISE EXCEPTION 'tool_truth_operation_contract_not_deployed' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_tool_truth_contract_insert_guard
BEFORE INSERT ON operation_state
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_operation_contract();

CREATE TRIGGER operation_state_tool_truth_contract_immutable
BEFORE UPDATE OF tool_truth_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_operation_contract();

-- ---------------------------------------------------------------------------
-- Existing authority tables: additive candidate keys only.
-- ---------------------------------------------------------------------------

ALTER TABLE operation_org_scope_snapshots
    ADD CONSTRAINT operation_scope_snapshot_execution_authority_unique
    UNIQUE(id,operation_id,project_scope_id,project_path_at_freeze);
ALTER TABLE stage_asset_waves
    ADD CONSTRAINT stage_asset_waves_tool_truth_authority_unique
    UNIQUE(id,operation_id,organization_id,stage_kind);
ALTER TABLE stage_run_units
    ADD CONSTRAINT stage_run_units_tool_truth_scope_authority_unique
    UNIQUE(id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,stage_kind);
ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_tool_truth_worker_authority_unique
    UNIQUE(id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,worker_run_id,attempt_epoch,lease_token);

-- ---------------------------------------------------------------------------
-- Immutable execution authority spine.
-- ---------------------------------------------------------------------------

CREATE TABLE tool_truth_stage_wave_execution_bindings (
    id UUID PRIMARY KEY,
    stage_asset_wave_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL CHECK (BTRIM(project_path_at_freeze)<>''),
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (BTRIM(stage_kind)<>''),
    binding_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_wave_binding_sha256_v1_check
        CHECK (binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
           organization_id,stage_execution_id,stage_kind,binding_hash),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,operation_id,project_scope_id,project_path_at_freeze)
        REFERENCES operation_org_scope_snapshots(id,operation_id,project_scope_id,project_path_at_freeze)
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(stage_execution_id,operation_id,stage_kind)
        REFERENCES stage_runs(id,operation_id,stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY(stage_asset_wave_id,operation_id,organization_id,stage_kind)
        REFERENCES stage_asset_waves(id,operation_id,organization_id,stage_kind) ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_validate_wave_binding()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM operation_org_scope_snapshots s
        WHERE s.id=NEW.scope_snapshot_id AND s.operation_id=NEW.operation_id
          AND s.project_scope_id=NEW.project_scope_id
          AND s.project_path_at_freeze=NEW.project_path_at_freeze
          AND s.sealed_at IS NOT NULL FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_scope_snapshot_unsealed' USING ERRCODE='23514';
    END IF;
    NEW.binding_hash := tool_truth_sha256(jsonb_build_object(
        'stage_asset_wave_id',NEW.stage_asset_wave_id,
        'operation_id',NEW.operation_id,'project_scope_id',NEW.project_scope_id,
        'project_path_at_freeze',NEW.project_path_at_freeze,
        'scope_snapshot_id',NEW.scope_snapshot_id,'organization_id',NEW.organization_id,
        'stage_execution_id',NEW.stage_execution_id,'stage_kind',NEW.stage_kind
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_wave_binding_validate
BEFORE INSERT ON tool_truth_stage_wave_execution_bindings
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_wave_binding();
CREATE TRIGGER tool_truth_wave_binding_immutable
BEFORE UPDATE OR DELETE ON tool_truth_stage_wave_execution_bindings
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_wave_binding_immutable');

CREATE TABLE tool_truth_execution_authorities (
    id UUID PRIMARY KEY,
    stable_authority_request_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL CHECK (BTRIM(project_path_at_freeze)<>''),
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (BTRIM(stage_kind)<>''),
    execution_source_kind TEXT NOT NULL CHECK (
        execution_source_kind IN ('stage_execution','stage_wave','stage_unit')
    ),
    stage_wave_binding_id UUID,
    stage_wave_binding_hash TEXT,
    stage_run_unit_id UUID,
    execution_owner_kind TEXT NOT NULL CHECK (
        execution_owner_kind IN ('host_stage','worker_tool')
    ),
    worker_run_id UUID,
    worker_attempt_epoch BIGINT,
    lease_token UUID,
    source_tool_call_id UUID,
    authority_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_execution_authority_sha256_v1_check CHECK (
        authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (stage_wave_binding_hash IS NULL OR stage_wave_binding_hash ~ '^sha256:[0-9a-f]{64}$')
    ),
    CONSTRAINT tool_truth_execution_source_shape_check CHECK (
        (execution_source_kind='stage_execution' AND stage_wave_binding_id IS NULL
            AND stage_wave_binding_hash IS NULL AND stage_run_unit_id IS NULL)
        OR (execution_source_kind='stage_wave' AND stage_wave_binding_id IS NOT NULL
            AND stage_wave_binding_hash IS NOT NULL AND stage_run_unit_id IS NULL)
        OR (execution_source_kind='stage_unit' AND stage_wave_binding_id IS NULL
            AND stage_wave_binding_hash IS NULL AND stage_run_unit_id IS NOT NULL)
    ),
    CONSTRAINT tool_truth_execution_owner_shape_check CHECK (
        (execution_owner_kind='host_stage' AND worker_run_id IS NULL
            AND worker_attempt_epoch IS NULL AND lease_token IS NULL
            AND source_tool_call_id IS NULL)
        OR (execution_owner_kind='worker_tool' AND execution_source_kind='stage_unit'
            AND worker_run_id IS NOT NULL AND worker_attempt_epoch IS NOT NULL
            AND worker_attempt_epoch>=0 AND lease_token IS NOT NULL
            AND source_tool_call_id IS NOT NULL)
    ),
    UNIQUE(operation_id,stable_authority_request_id),
    UNIQUE(id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
           organization_id,stage_execution_id,stage_kind,authority_hash),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,operation_id,project_scope_id,project_path_at_freeze)
        REFERENCES operation_org_scope_snapshots(id,operation_id,project_scope_id,project_path_at_freeze)
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(stage_execution_id,operation_id,stage_kind)
        REFERENCES stage_runs(id,operation_id,stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY(stage_wave_binding_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,stage_wave_binding_hash)
        REFERENCES tool_truth_stage_wave_execution_bindings(
            id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
            organization_id,stage_execution_id,stage_kind,binding_hash) ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,stage_kind)
        REFERENCES stage_run_units(id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,stage_kind)
        ON DELETE RESTRICT,
    FOREIGN KEY(source_tool_call_id,operation_id,stage_execution_id,stage_run_unit_id,
                organization_id,worker_run_id,worker_attempt_epoch,lease_token)
        REFERENCES tool_calls(id,operation_id,stage_execution_id,stage_run_unit_id,
                              organization_id,worker_run_id,attempt_epoch,lease_token)
        ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_validate_execution_authority()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM operation_org_scope_snapshots s
        WHERE s.id=NEW.scope_snapshot_id
          AND s.operation_id=NEW.operation_id
          AND s.project_scope_id=NEW.project_scope_id
          AND s.project_path_at_freeze=NEW.project_path_at_freeze
          AND s.sealed_at IS NOT NULL
        FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_authority_scope_snapshot_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM operation_org_scope_units u
        WHERE u.snapshot_id=NEW.scope_snapshot_id
          AND u.organization_id=NEW.organization_id
        FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_authority_scope_org_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM stage_runs s
        WHERE s.id=NEW.stage_execution_id
          AND s.operation_id=NEW.operation_id
          AND s.stage_kind=NEW.stage_kind
        FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_authority_stage_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.execution_owner_kind='worker_tool' AND NOT EXISTS (
        SELECT 1 FROM stage_worker_runs w JOIN tool_calls t ON t.id=NEW.source_tool_call_id
        WHERE w.id=NEW.worker_run_id AND w.operation_id=NEW.operation_id
          AND w.stage_execution_id=NEW.stage_execution_id
          AND w.stage_run_unit_id=NEW.stage_run_unit_id
          AND w.organization_id=NEW.organization_id
          AND w.attempt_epoch=NEW.worker_attempt_epoch AND w.lease_token=NEW.lease_token
          AND t.worker_run_id=w.id AND t.attempt_epoch=w.attempt_epoch
          AND t.lease_token=w.lease_token FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_worker_fence_mismatch' USING ERRCODE='23514';
    END IF;
    NEW.authority_hash := tool_truth_sha256(jsonb_build_object(
        'stable_authority_request_id',NEW.stable_authority_request_id,
        'operation_id',NEW.operation_id,'project_scope_id',NEW.project_scope_id,
        'project_path_at_freeze',NEW.project_path_at_freeze,
        'scope_snapshot_id',NEW.scope_snapshot_id,'organization_id',NEW.organization_id,
        'stage_execution_id',NEW.stage_execution_id,'stage_kind',NEW.stage_kind,
        'execution_source_kind',NEW.execution_source_kind,
        'stage_wave_binding_id',NEW.stage_wave_binding_id,
        'stage_wave_binding_hash',NEW.stage_wave_binding_hash,
        'stage_run_unit_id',NEW.stage_run_unit_id,
        'execution_owner_kind',NEW.execution_owner_kind,'worker_run_id',NEW.worker_run_id,
        'worker_attempt_epoch',NEW.worker_attempt_epoch,'lease_token',NEW.lease_token,
        'source_tool_call_id',NEW.source_tool_call_id
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_execution_authority_validate
BEFORE INSERT ON tool_truth_execution_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_execution_authority();
CREATE TRIGGER tool_truth_execution_authority_immutable
BEFORE UPDATE OR DELETE ON tool_truth_execution_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_execution_authority_immutable');

-- ---------------------------------------------------------------------------
-- Evidence production and normalized evidence authority.
-- ---------------------------------------------------------------------------

CREATE TABLE tool_truth_evidence_production_bindings (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    evidence_audit_id BIGINT NOT NULL UNIQUE REFERENCES audit_log(id) ON DELETE RESTRICT,
    evidence_classification_id BIGINT NOT NULL UNIQUE REFERENCES evidence_classifications(id) ON DELETE RESTRICT,
    production_binding_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_evidence_production_sha256_v1_check CHECK (
        execution_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND production_binding_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    UNIQUE(id,execution_authority_id),
    CONSTRAINT tool_truth_evidence_production_authority_fk
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,execution_authority_hash)
        REFERENCES tool_truth_execution_authorities(id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,authority_hash)
        ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_validate_evidence_production_binding()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE a tool_truth_execution_authorities%ROWTYPE;
DECLARE audit audit_log%ROWTYPE;
DECLARE classification evidence_classifications%ROWTYPE;
DECLARE producer JSONB;
BEGIN
    SELECT * INTO a FROM tool_truth_execution_authorities WHERE id=NEW.execution_authority_id FOR SHARE;
    SELECT * INTO audit FROM audit_log WHERE id=NEW.evidence_audit_id FOR SHARE;
    SELECT * INTO classification FROM evidence_classifications WHERE id=NEW.evidence_classification_id FOR SHARE;
    IF audit.audit_role <> 'evidence' THEN
        RAISE EXCEPTION 'tool_truth_evidence_role_invalid' USING ERRCODE='23514';
    END IF;
    IF audit.run_id IS DISTINCT FROM a.operation_id THEN
        RAISE EXCEPTION 'tool_truth_evidence_operation_mismatch' USING ERRCODE='23514';
    END IF;
    IF audit.project_path IS DISTINCT FROM a.project_path_at_freeze THEN
        RAISE EXCEPTION 'tool_truth_evidence_project_mismatch' USING ERRCODE='23514';
    END IF;
    producer := audit.detail->'tool_truth_producer';
    IF producer IS NULL OR jsonb_typeof(producer)<>'object'
       OR producer->>'organization_id' IS DISTINCT FROM a.organization_id::TEXT
       OR producer->>'stage_execution_id' IS DISTINCT FROM a.stage_execution_id::TEXT THEN
        RAISE EXCEPTION 'tool_truth_evidence_producer_envelope_invalid' USING ERRCODE='23514';
    END IF;
    IF classification.evidence_audit_id<>audit.id OR classification.valid_to IS NOT NULL
       OR classification.classification<>'in_scope' THEN
        RAISE EXCEPTION 'tool_truth_evidence_classification_invalid' USING ERRCODE='23514';
    END IF;
    IF classification.producing_stage_run_id IS DISTINCT FROM a.stage_execution_id THEN
        RAISE EXCEPTION 'tool_truth_evidence_stage_mismatch' USING ERRCODE='23514';
    END IF;
    IF a.execution_owner_kind='worker_tool' AND (
        producer->>'source_tool_call_id' IS DISTINCT FROM a.source_tool_call_id::TEXT
        OR producer->>'worker_run_id' IS DISTINCT FROM a.worker_run_id::TEXT
        OR producer->>'worker_attempt_epoch' IS DISTINCT FROM a.worker_attempt_epoch::TEXT
        OR producer->>'lease_token' IS DISTINCT FROM a.lease_token::TEXT
    ) THEN
        RAISE EXCEPTION 'tool_truth_evidence_worker_fence_mismatch' USING ERRCODE='23514';
    END IF;
    IF a.execution_owner_kind='host_stage' AND producer ?| ARRAY[
        'source_tool_call_id','worker_run_id','worker_attempt_epoch','lease_token'
    ] THEN
        RAISE EXCEPTION 'tool_truth_evidence_producer_envelope_invalid' USING ERRCODE='23514';
    END IF;
    NEW.production_binding_hash := tool_truth_sha256(jsonb_build_object(
        'execution_authority_id',NEW.execution_authority_id,
        'execution_authority_hash',NEW.execution_authority_hash,
        'evidence_audit_id',NEW.evidence_audit_id,
        'evidence_classification_id',NEW.evidence_classification_id
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_evidence_production_validate
BEFORE INSERT ON tool_truth_evidence_production_bindings
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_evidence_production_binding();
CREATE TRIGGER tool_truth_evidence_production_immutable
BEFORE UPDATE OR DELETE ON tool_truth_evidence_production_bindings
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_evidence_authority_immutable');

CREATE TABLE tool_truth_evidence_authorities (
    id UUID PRIMARY KEY,
    production_binding_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    evidence_audit_id BIGINT NOT NULL UNIQUE REFERENCES audit_log(id) ON DELETE RESTRICT,
    evidence_classification_id BIGINT NOT NULL REFERENCES evidence_classifications(id) ON DELETE RESTRICT,
    audit_row_hash TEXT NOT NULL,
    classification_row_hash TEXT NOT NULL,
    evidence_chain_hash TEXT NOT NULL,
    authority_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_evidence_authority_sha256_v1_check CHECK (
        execution_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND audit_row_hash ~ '^sha256:[0-9a-f]{64}$'
        AND classification_row_hash ~ '^sha256:[0-9a-f]{64}$'
        AND evidence_chain_hash ~ '^sha256:[0-9a-f]{64}$'
        AND authority_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,authority_hash),
    UNIQUE(execution_authority_id,evidence_audit_id),
    FOREIGN KEY(production_binding_id,execution_authority_id)
        REFERENCES tool_truth_evidence_production_bindings(id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,execution_authority_hash)
        REFERENCES tool_truth_execution_authorities(id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,authority_hash)
        ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_validate_evidence_authority()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE binding tool_truth_evidence_production_bindings%ROWTYPE;
DECLARE audit audit_log%ROWTYPE;
DECLARE classification evidence_classifications%ROWTYPE;
DECLARE expected_audit_hash TEXT;
DECLARE expected_classification_hash TEXT;
DECLARE expected_chain_hash TEXT;
BEGIN
    SELECT * INTO binding FROM tool_truth_evidence_production_bindings
     WHERE id=NEW.production_binding_id AND execution_authority_id=NEW.execution_authority_id FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tool_truth_evidence_production_binding_missing' USING ERRCODE='23514';
    END IF;
    IF binding.evidence_audit_id<>NEW.evidence_audit_id
       OR binding.evidence_classification_id<>NEW.evidence_classification_id THEN
        RAISE EXCEPTION 'tool_truth_evidence_classification_mismatch' USING ERRCODE='23514';
    END IF;
    SELECT * INTO audit FROM audit_log WHERE id=NEW.evidence_audit_id;
    SELECT * INTO classification FROM evidence_classifications WHERE id=NEW.evidence_classification_id;
    IF classification.evidence_audit_id<>audit.id OR classification.valid_to IS NOT NULL
       OR classification.classification<>'in_scope' THEN
        RAISE EXCEPTION 'tool_truth_evidence_classification_invalid' USING ERRCODE='23514';
    END IF;
    expected_audit_hash := tool_truth_sha256(to_jsonb(audit)::TEXT);
    expected_classification_hash := tool_truth_sha256(to_jsonb(classification)::TEXT);
    expected_chain_hash := tool_truth_sha256(jsonb_build_object(
        'audit_row_hash',expected_audit_hash,
        'classification_row_hash',expected_classification_hash
    )::TEXT);
    NEW.audit_row_hash := expected_audit_hash;
    NEW.classification_row_hash := expected_classification_hash;
    NEW.evidence_chain_hash := expected_chain_hash;
    NEW.authority_hash := tool_truth_sha256(jsonb_build_object(
        'production_binding_id',NEW.production_binding_id,
        'execution_authority_id',NEW.execution_authority_id,
        'execution_authority_hash',NEW.execution_authority_hash,
        'evidence_audit_id',NEW.evidence_audit_id,
        'evidence_classification_id',NEW.evidence_classification_id,
        'evidence_chain_hash',expected_chain_hash
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_evidence_authority_validate
BEFORE INSERT ON tool_truth_evidence_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_evidence_authority();
CREATE TRIGGER tool_truth_evidence_authority_immutable
BEFORE UPDATE OR DELETE ON tool_truth_evidence_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_evidence_authority_immutable');

CREATE FUNCTION tool_truth_protect_bound_audit_rows()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM tool_truth_evidence_production_bindings WHERE evidence_audit_id=OLD.id) THEN
        RAISE EXCEPTION 'tool_truth_evidence_authority_immutable' USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE TRIGGER tool_truth_bound_audit_rows_immutable
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION tool_truth_protect_bound_audit_rows();

CREATE FUNCTION tool_truth_protect_bound_classifications()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM tool_truth_evidence_production_bindings WHERE evidence_classification_id=OLD.id) THEN
        RAISE EXCEPTION 'tool_truth_evidence_authority_immutable' USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE TRIGGER tool_truth_bound_classifications_immutable
BEFORE UPDATE OR DELETE ON evidence_classifications
FOR EACH ROW EXECUTE FUNCTION tool_truth_protect_bound_classifications();

-- ---------------------------------------------------------------------------
-- Canonical business-reference capture.
-- ---------------------------------------------------------------------------

CREATE TABLE tool_truth_business_ref_authorities (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    evidence_authority_id UUID NOT NULL,
    ref_kind TEXT NOT NULL,
    ref_uuid UUID,
    ref_bigint BIGINT,
    snapshot_contract_version TEXT NOT NULL DEFAULT 'tool_truth_business_ref_snapshot.v1',
    canonical_snapshot JSONB NOT NULL CHECK (jsonb_typeof(canonical_snapshot)='object'),
    source_observed_at TIMESTAMPTZ NOT NULL,
    source_hash TEXT NOT NULL,
    authority_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_business_ref_kind_check CHECK (ref_kind IN (
        'target_asset','dns_record','web_origin_observation','network_endpoint',
        'enumeration_endpoint_observation'
    )),
    CONSTRAINT tool_truth_business_ref_contract_check CHECK (
        snapshot_contract_version='tool_truth_business_ref_snapshot.v1'
    ),
    CONSTRAINT tool_truth_business_ref_sha256_v1_check CHECK (
        source_hash ~ '^sha256:[0-9a-f]{64}$' AND authority_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT tool_truth_business_ref_id_shape_check CHECK (
        (ref_kind='dns_record' AND ref_bigint IS NOT NULL AND ref_bigint>0 AND ref_uuid IS NULL)
        OR (ref_kind<>'dns_record' AND ref_uuid IS NOT NULL AND ref_bigint IS NULL)
    ),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,authority_hash),
    FOREIGN KEY(evidence_authority_id,execution_authority_id)
        REFERENCES tool_truth_evidence_authorities(id,execution_authority_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX tool_truth_business_ref_uuid_unique
    ON tool_truth_business_ref_authorities(execution_authority_id,ref_kind,ref_uuid)
    WHERE ref_uuid IS NOT NULL;
CREATE UNIQUE INDEX tool_truth_business_ref_bigint_unique
    ON tool_truth_business_ref_authorities(execution_authority_id,ref_kind,ref_bigint)
    WHERE ref_bigint IS NOT NULL;

CREATE FUNCTION tool_truth_validate_business_ref()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE authority tool_truth_execution_authorities%ROWTYPE;
DECLARE expected_snapshot JSONB;
DECLARE expected_observed_at TIMESTAMPTZ;
DECLARE expected_hash TEXT;
BEGIN
    IF (NEW.ref_kind='dns_record' AND
        (NEW.ref_bigint IS NULL OR NEW.ref_bigint<=0 OR NEW.ref_uuid IS NOT NULL))
       OR (NEW.ref_kind<>'dns_record' AND
        (NEW.ref_uuid IS NULL OR NEW.ref_bigint IS NOT NULL)) THEN
        RAISE EXCEPTION 'tool_truth_business_ref_id_shape_invalid' USING ERRCODE='23514';
    END IF;
    SELECT * INTO authority FROM tool_truth_execution_authorities
     WHERE id=NEW.execution_authority_id FOR SHARE;
    IF NEW.ref_kind='target_asset' THEN
        SELECT jsonb_build_object('kind','target_asset','id',a.id,'target_id',a.target_id,
                   'asset_type',a.asset_type,'value',a.value,'port',a.port,'protocol',a.protocol),
               a.discovered_at
          INTO expected_snapshot,expected_observed_at
          FROM target_assets a JOIN targets t ON t.id=a.target_id
         WHERE a.id=NEW.ref_uuid AND t.organization_id=authority.organization_id
           AND a.project_path=authority.project_path_at_freeze
           AND t.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='dns_record' THEN
        SELECT jsonb_build_object('kind','dns_record','id',d.id,'target_id',d.target_id,
                   'record_type',d.record_type,'name',d.name,'value',d.value,'source',d.source),
               d.created_at
          INTO expected_snapshot,expected_observed_at
          FROM dns_records d JOIN targets t ON t.id=d.target_id
         WHERE d.id=NEW.ref_bigint AND t.organization_id=authority.organization_id
           AND d.project_path=authority.project_path_at_freeze
           AND t.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='web_origin_observation' THEN
        SELECT jsonb_build_object('kind','web_origin_observation','id',o.id,
                   'web_origin_id',o.web_origin_id,'network_endpoint_id',o.network_endpoint_id,
                   'target_id',o.target_id,'status_code',o.status_code,'source',o.source),o.observed_at
          INTO expected_snapshot,expected_observed_at
          FROM web_origin_observations o JOIN web_origins w ON w.id=o.web_origin_id
          LEFT JOIN network_endpoints n ON n.id=o.network_endpoint_id
          LEFT JOIN targets t ON t.id=o.target_id
         WHERE o.id=NEW.ref_uuid AND o.organization_id=authority.organization_id
           AND o.project_path=authority.project_path_at_freeze
           AND w.organization_id=authority.organization_id AND w.project_path=authority.project_path_at_freeze
           AND (n.id IS NULL OR (n.organization_id=authority.organization_id AND n.project_path=authority.project_path_at_freeze))
           AND (t.id IS NULL OR (t.organization_id=authority.organization_id AND t.project_path=authority.project_path_at_freeze));
    ELSIF NEW.ref_kind='network_endpoint' THEN
        SELECT jsonb_build_object('kind','network_endpoint','id',n.id,'ip',n.ip,'port',n.port,
                   'transport',n.transport,'state',n.state,'service_name',n.service_name),n.last_seen_at
          INTO expected_snapshot,expected_observed_at FROM network_endpoints n
         WHERE n.id=NEW.ref_uuid AND n.organization_id=authority.organization_id
           AND n.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='enumeration_endpoint_observation' THEN
        SELECT jsonb_build_object('kind','enumeration_endpoint_observation','id',o.id,
                   'target_id',o.target_id,'web_origin_id',o.web_origin_id,
                   'endpoint_id',o.endpoint_id,'source',o.source),o.observed_at
          INTO expected_snapshot,expected_observed_at
          FROM enumeration_endpoint_observations o
          JOIN targets t ON t.id=o.target_id JOIN web_origins w ON w.id=o.web_origin_id
          JOIN api_endpoints e ON e.id=o.endpoint_id
         WHERE o.id=NEW.ref_uuid AND o.operation_id=authority.operation_id
           AND o.organization_id=authority.organization_id AND o.project_path=authority.project_path_at_freeze
           AND t.organization_id=authority.organization_id AND t.project_path=authority.project_path_at_freeze
           AND w.organization_id=authority.organization_id AND w.project_path=authority.project_path_at_freeze
           AND e.target_id=t.id AND e.project_path=authority.project_path_at_freeze;
    END IF;
    IF expected_snapshot IS NULL THEN
        RAISE EXCEPTION 'tool_truth_business_ref_owner_mismatch' USING ERRCODE='23514';
    END IF;
    expected_hash := tool_truth_sha256(expected_snapshot::TEXT);
    NEW.canonical_snapshot := expected_snapshot;
    NEW.source_observed_at := expected_observed_at;
    NEW.source_hash := expected_hash;
    NEW.authority_hash := tool_truth_sha256(jsonb_build_object(
        'execution_authority_id',NEW.execution_authority_id,
        'evidence_authority_id',NEW.evidence_authority_id,
        'ref_kind',NEW.ref_kind,'ref_uuid',NEW.ref_uuid,'ref_bigint',NEW.ref_bigint,
        'source_hash',expected_hash
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_business_ref_validate
BEFORE INSERT ON tool_truth_business_ref_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_business_ref();
CREATE TRIGGER tool_truth_business_ref_immutable
BEFORE UPDATE OR DELETE ON tool_truth_business_ref_authorities
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_business_ref_immutable');

-- ---------------------------------------------------------------------------
-- Sealed denominator and destination/temporal policy authorities.
-- ---------------------------------------------------------------------------

CREATE TABLE coverage_denominators (
    id UUID PRIMARY KEY,
    stable_seal_request_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    denominator_kind TEXT NOT NULL DEFAULT 'root' CHECK (denominator_kind IN ('root','derived_child')),
    parent_denominator_id UUID,
    parent_denominator_item_id UUID,
    derived_ordinal INTEGER,
    contract TEXT NOT NULL CHECK (contract IN ('shadow_v1','receipt_v1')),
    input_manifest_hash TEXT NOT NULL,
    member_count BIGINT,
    member_set_hash TEXT,
    sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    denominator_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CONSTRAINT coverage_denominator_sha256_v1_check CHECK (
        execution_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND input_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
        AND denominator_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')
    ),
    CONSTRAINT coverage_denominator_shape_check CHECK (
        (denominator_kind='root' AND parent_denominator_id IS NULL
            AND parent_denominator_item_id IS NULL AND derived_ordinal IS NULL)
        OR (denominator_kind='derived_child' AND parent_denominator_id IS NOT NULL
            AND parent_denominator_item_id IS NOT NULL AND derived_ordinal IS NOT NULL
            AND derived_ordinal>0)
    ),
    CONSTRAINT coverage_denominator_seal_shape_check CHECK (
        (sealed_at IS NULL AND member_count IS NULL AND member_set_hash IS NULL)
        OR (sealed_at IS NOT NULL AND member_count IS NOT NULL AND member_count>=0
            AND member_set_hash IS NOT NULL AND sealed_empty=(member_count=0))
    ),
    UNIQUE(execution_authority_id,stable_seal_request_id),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,denominator_hash),
    UNIQUE(id,execution_authority_id,input_manifest_hash),
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,execution_authority_hash)
        REFERENCES tool_truth_execution_authorities(id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,authority_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(parent_denominator_id,execution_authority_id)
        REFERENCES coverage_denominators(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE TABLE coverage_denominator_items (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    denominator_hash TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    input_key TEXT NOT NULL CHECK (BTRIM(input_key)<>''),
    target_id UUID REFERENCES targets(id) ON DELETE RESTRICT,
    exact_asset TEXT NOT NULL CHECK (BTRIM(exact_asset)<>''),
    technique TEXT NOT NULL CHECK (BTRIM(technique)<>''),
    expected_capability TEXT NOT NULL CHECK (BTRIM(expected_capability)<>''),
    member_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT coverage_denominator_item_sha256_v1_check CHECK (
        denominator_hash ~ '^sha256:[0-9a-f]{64}$'
        AND member_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    UNIQUE(denominator_id,ordinal),
    UNIQUE(denominator_id,input_key),
    UNIQUE(denominator_id,member_hash),
    UNIQUE(id,denominator_id,execution_authority_id),
    FOREIGN KEY(denominator_id,execution_authority_id,denominator_hash)
        REFERENCES coverage_denominators(id,execution_authority_id,denominator_hash) ON DELETE RESTRICT
);
ALTER TABLE coverage_denominators
    ADD CONSTRAINT coverage_denominator_parent_item_fk
    FOREIGN KEY(parent_denominator_item_id,parent_denominator_id,execution_authority_id)
    REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id) ON DELETE RESTRICT;

CREATE TRIGGER coverage_denominator_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON coverage_denominators
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'coverage_denominator_items','denominator_id','member_hash','true');
CREATE TRIGGER coverage_denominator_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON coverage_denominator_items
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'coverage_denominators','id','denominator_id');

CREATE TABLE capability_execution_destination_policies (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    capability TEXT NOT NULL CHECK (BTRIM(capability)<>''),
    policy_contract_version TEXT NOT NULL DEFAULT 'tool_execution_destination.v1'
        CHECK (policy_contract_version='tool_execution_destination.v1'),
    execution_backend TEXT NOT NULL CHECK (
        execution_backend IN ('host_pinned_http','sandboxed_cli','fixed_provider_transport','none_blocked')),
    governance_status TEXT NOT NULL CHECK (
        governance_status IN ('enforced','shadow_observed_uncontrolled','policy_blocked')),
    redirect_mode TEXT NOT NULL CHECK (redirect_mode IN ('deny','exact_same_origin_allowlist')),
    max_redirect_hops INTEGER NOT NULL CHECK (max_redirect_hops>=0),
    secondary_fetch_mode TEXT NOT NULL DEFAULT 'deny' CHECK (secondary_fetch_mode='deny'),
    proxy_mode TEXT NOT NULL DEFAULT 'none' CHECK (proxy_mode='none'),
    tls_policy_hash TEXT NOT NULL,
    prohibited_range_policy_hash TEXT NOT NULL,
    member_count BIGINT,
    member_set_hash TEXT,
    sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    policy_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CONSTRAINT capability_destination_policy_sha256_v1_check CHECK (
        tls_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND prohibited_range_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')),
    CHECK ((governance_status='policy_blocked')=(execution_backend='none_blocked')),
    UNIQUE(denominator_id,capability,policy_hash),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,policy_hash),
    FOREIGN KEY(denominator_id,execution_authority_id)
        REFERENCES coverage_denominators(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE TABLE capability_execution_destination_policy_members (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    destination_role TEXT NOT NULL CHECK (
        destination_role IN ('authorized_target','fixed_provider_endpoint','fixed_dns_resolver')),
    scheme TEXT NOT NULL CHECK (BTRIM(scheme)<>''),
    normalized_host TEXT NOT NULL CHECK (BTRIM(normalized_host)<>''),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    path_prefix TEXT NOT NULL,
    input_binding_mode TEXT NOT NULL CHECK (
        input_binding_mode IN ('destination_authority','escaped_parameter_only')),
    exact_scope_exception_hash TEXT,
    member_hash TEXT NOT NULL,
    CONSTRAINT capability_destination_member_sha256_v1_check CHECK (
        member_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (exact_scope_exception_hash IS NULL OR exact_scope_exception_hash ~ '^sha256:[0-9a-f]{64}$')),
    UNIQUE(policy_id,ordinal),UNIQUE(policy_id,member_hash),
    UNIQUE(id,policy_id,execution_authority_id),
    FOREIGN KEY(policy_id,execution_authority_id)
        REFERENCES capability_execution_destination_policies(id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER capability_destination_policy_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_destination_policies
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'capability_execution_destination_policy_members','policy_id','member_hash','true');
CREATE TRIGGER capability_destination_policy_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_destination_policy_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'capability_execution_destination_policies','id','policy_id');

CREATE TABLE evidence_temporal_validity_policies (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    policy_contract_version TEXT NOT NULL DEFAULT 'evidence_temporal_validity.v1'
        CHECK (policy_contract_version='evidence_temporal_validity.v1'),
    max_cross_observation_skew_ms BIGINT NOT NULL CHECK (max_cross_observation_skew_ms>=0),
    member_count BIGINT, member_set_hash TEXT, sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    policy_hash TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CONSTRAINT evidence_temporal_policy_sha256_v1_check CHECK (
        policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')),
    UNIQUE(id,execution_authority_id),UNIQUE(id,execution_authority_id,policy_hash),
    UNIQUE(execution_authority_id,policy_hash),
    FOREIGN KEY(execution_authority_id) REFERENCES tool_truth_execution_authorities(id) ON DELETE RESTRICT
);
CREATE TABLE evidence_temporal_validity_policy_members (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    fact_class TEXT NOT NULL CHECK (BTRIM(fact_class)<>''),
    positive_ttl_ms BIGINT NOT NULL CHECK (positive_ttl_ms>0),
    negative_ttl_ms BIGINT NOT NULL CHECK (negative_ttl_ms>0),
    refutation_ttl_ms BIGINT NOT NULL CHECK (refutation_ttl_ms>0),
    require_same_target_state_epoch BOOLEAN NOT NULL,
    required_recheck_source TEXT NOT NULL CHECK (BTRIM(required_recheck_source)<>''),
    member_hash TEXT NOT NULL,
    CONSTRAINT evidence_temporal_policy_member_sha256_v1_check CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(policy_id,ordinal),UNIQUE(policy_id,fact_class),UNIQUE(policy_id,member_hash),
    FOREIGN KEY(policy_id) REFERENCES evidence_temporal_validity_policies(id) ON DELETE RESTRICT,
    CHECK (negative_ttl_ms<positive_ttl_ms AND refutation_ttl_ms<positive_ttl_ms)
);
CREATE TRIGGER evidence_temporal_policy_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON evidence_temporal_validity_policies
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'evidence_temporal_validity_policy_members','policy_id','member_hash','false');
CREATE TRIGGER evidence_temporal_policy_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON evidence_temporal_validity_policy_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'evidence_temporal_validity_policies','id','policy_id');

-- ---------------------------------------------------------------------------
-- Receipt lifecycle and exact raw-witness binding.
-- ---------------------------------------------------------------------------

CREATE TABLE capability_execution_receipts (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    capability TEXT NOT NULL CHECK (BTRIM(capability)<>''),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>0),
    receipt_authority_hash TEXT NOT NULL,
    input_manifest_hash TEXT NOT NULL,
    destination_policy_id UUID NOT NULL,
    destination_policy_hash TEXT NOT NULL,
    temporal_validity_policy_id UUID NOT NULL,
    temporal_validity_policy_hash TEXT NOT NULL,
    attempt_state TEXT NOT NULL CHECK (attempt_state IN (
        'not_started','running','succeeded','failed','outcome_unknown','exhausted','superseded')),
    landing_state TEXT NOT NULL CHECK (landing_state IN ('not_attempted','partial','committed','failed')),
    observation_state TEXT NOT NULL CHECK (observation_state IN ('found','no_match','indeterminate','not_applicable')),
    coverage_extent TEXT NOT NULL CHECK (coverage_extent IN ('none','complete','partial','sampled','template_only')),
    coverage_gap_reason TEXT NOT NULL CHECK (coverage_gap_reason IN (
        'none','transport','tool_failure','parser_reject','budget_exhausted','unsupported','policy_blocked','source_unavailable')),
    reconciliation_state TEXT NOT NULL CHECK (reconciliation_state IN ('pending','consistent','orphaned','superseded')),
    security_interpretation TEXT NOT NULL CHECK (security_interpretation IN ('not_assessed','signal','inconclusive')),
    typed_landing_contract_version TEXT NOT NULL DEFAULT 'capability_landing.v1'
        CHECK (typed_landing_contract_version='capability_landing.v1'),
    typed_landing JSONB NOT NULL CHECK (jsonb_typeof(typed_landing)='object'),
    residual JSONB,
    raw_witness_artifact_id UUID UNIQUE,
    parser_census_id UUID UNIQUE,
    temporal_census_id UUID UNIQUE,
    current_semantic_authority_version BIGINT NOT NULL DEFAULT 0 CHECK (current_semantic_authority_version>=0),
    current_semantic_reconciliation_id UUID,
    current_semantic_reconciliation_hash TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    observation_started_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    observation_completed_at TIMESTAMPTZ,
    valid_until TIMESTAMPTZ,
    finalized_at TIMESTAMPTZ,
    CONSTRAINT capability_execution_receipt_sha256_v1_check CHECK (
        receipt_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND input_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
        AND destination_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND temporal_validity_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (current_semantic_reconciliation_hash IS NULL OR current_semantic_reconciliation_hash ~ '^sha256:[0-9a-f]{64}$')),
    CONSTRAINT capability_execution_receipt_semantic_shape_check CHECK (
        (current_semantic_authority_version=0 AND current_semantic_reconciliation_id IS NULL
            AND current_semantic_reconciliation_hash IS NULL)
        OR (current_semantic_authority_version>0 AND current_semantic_reconciliation_id IS NOT NULL
            AND current_semantic_reconciliation_hash IS NOT NULL)),
    CONSTRAINT capability_execution_receipt_complete_shape_check CHECK (
        coverage_extent<>'complete' OR (attempt_state='succeeded' AND landing_state='committed'
            AND observation_state IN ('found','no_match') AND coverage_gap_reason='none'
            AND reconciliation_state='consistent' AND raw_witness_artifact_id IS NOT NULL
            AND parser_census_id IS NOT NULL AND temporal_census_id IS NOT NULL
            AND observation_completed_at IS NOT NULL AND valid_until>observation_completed_at)),
    UNIQUE(denominator_id,execution_authority_id,capability,attempt_ordinal),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,denominator_id,execution_authority_id),
    UNIQUE(id,execution_authority_id,receipt_authority_hash),
    UNIQUE(id,destination_policy_id,execution_authority_id),
    FOREIGN KEY(denominator_id,execution_authority_id,input_manifest_hash)
        REFERENCES coverage_denominators(id,execution_authority_id,input_manifest_hash) ON DELETE RESTRICT,
    FOREIGN KEY(destination_policy_id,execution_authority_id,destination_policy_hash)
        REFERENCES capability_execution_destination_policies(id,execution_authority_id,policy_hash) ON DELETE RESTRICT,
    FOREIGN KEY(temporal_validity_policy_id,execution_authority_id,temporal_validity_policy_hash)
        REFERENCES evidence_temporal_validity_policies(id,execution_authority_id,policy_hash) ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_guard_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    denominator_hash TEXT;
    expected_receipt_hash TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'tool_truth_receipt_append_only' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(NEW) - ARRAY[
                'attempt_state','landing_state','observation_state','coverage_extent',
                'coverage_gap_reason','reconciliation_state','security_interpretation',
                'typed_landing','residual','raw_witness_artifact_id','parser_census_id',
                'temporal_census_id','current_semantic_authority_version',
                'current_semantic_reconciliation_id','current_semantic_reconciliation_hash',
                'row_version','observation_completed_at','valid_until','finalized_at'
            ]) IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'attempt_state','landing_state','observation_state','coverage_extent',
                'coverage_gap_reason','reconciliation_state','security_interpretation',
                'typed_landing','residual','raw_witness_artifact_id','parser_census_id',
                'temporal_census_id','current_semantic_authority_version',
                'current_semantic_reconciliation_id','current_semantic_reconciliation_hash',
                'row_version','observation_completed_at','valid_until','finalized_at'
            ]) THEN
            RAISE EXCEPTION 'tool_truth_receipt_authority_immutable' USING ERRCODE='23514';
        END IF;
        IF NEW.row_version<>OLD.row_version+1 THEN
            RAISE EXCEPTION 'tool_truth_receipt_cas_required' USING ERRCODE='23514';
        END IF;
        IF (OLD.raw_witness_artifact_id IS NOT NULL
                AND NEW.raw_witness_artifact_id IS DISTINCT FROM OLD.raw_witness_artifact_id)
           OR (OLD.parser_census_id IS NOT NULL
                AND NEW.parser_census_id IS DISTINCT FROM OLD.parser_census_id)
           OR (OLD.temporal_census_id IS NOT NULL
                AND NEW.temporal_census_id IS DISTINCT FROM OLD.temporal_census_id)
           OR (OLD.finalized_at IS NOT NULL AND NEW.finalized_at IS DISTINCT FROM OLD.finalized_at) THEN
            RAISE EXCEPTION 'tool_truth_receipt_terminal_binding_immutable' USING ERRCODE='23514';
        END IF;
        IF NEW.current_semantic_authority_version NOT IN (
                OLD.current_semantic_authority_version,
                OLD.current_semantic_authority_version+1
            ) THEN
            RAISE EXCEPTION 'tool_truth_receipt_semantic_version_invalid' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;

    SELECT d.denominator_hash INTO denominator_hash
      FROM coverage_denominators d
     WHERE d.id=NEW.denominator_id
       AND d.execution_authority_id=NEW.execution_authority_id
       AND d.input_manifest_hash=NEW.input_manifest_hash
       AND d.sealed_at IS NOT NULL
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tool_truth_denominator_unsealed_or_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM capability_execution_destination_policies p
         WHERE p.id=NEW.destination_policy_id
           AND p.execution_authority_id=NEW.execution_authority_id
           AND p.policy_hash=NEW.destination_policy_hash
           AND p.sealed_at IS NOT NULL FOR SHARE
    ) OR NOT EXISTS (
        SELECT 1 FROM evidence_temporal_validity_policies p
         WHERE p.id=NEW.temporal_validity_policy_id
           AND p.execution_authority_id=NEW.execution_authority_id
           AND p.policy_hash=NEW.temporal_validity_policy_hash
           AND p.sealed_at IS NOT NULL FOR SHARE
    ) THEN
        RAISE EXCEPTION 'tool_truth_receipt_policy_unsealed_or_mismatch' USING ERRCODE='23514';
    END IF;
    expected_receipt_hash := tool_truth_sha256(jsonb_build_object(
        'denominator_id',NEW.denominator_id,
        'denominator_hash',denominator_hash,
        'execution_authority_id',NEW.execution_authority_id,
        'capability',NEW.capability,
        'attempt_ordinal',NEW.attempt_ordinal,
        'input_manifest_hash',NEW.input_manifest_hash,
        'destination_policy_hash',NEW.destination_policy_hash,
        'temporal_validity_policy_hash',NEW.temporal_validity_policy_hash
    )::TEXT);
    NEW.receipt_authority_hash := expected_receipt_hash;
    RETURN NEW;
END;
$$;
CREATE TRIGGER capability_execution_receipt_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_receipts
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt();

CREATE TABLE capability_raw_witness_artifacts (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL,
    receipt_authority_hash TEXT NOT NULL,
    content_key TEXT NOT NULL,
    vault_object_ref_token BYTEA NOT NULL CHECK (octet_length(vault_object_ref_token) BETWEEN 32 AND 4096),
    vault_object_ref_token_hash TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    ciphertext_sha256 TEXT NOT NULL,
    encryption_contract_version TEXT NOT NULL DEFAULT 'raw_witness_envelope.v1'
        CHECK (encryption_contract_version='raw_witness_envelope.v1'),
    operation_key_ref_hash TEXT NOT NULL,
    key_generation BIGINT NOT NULL CHECK (key_generation>0),
    retention_policy_id UUID NOT NULL,
    retention_policy_hash TEXT NOT NULL,
    sensitivity_disposition TEXT NOT NULL CHECK (
        sensitivity_disposition IN ('typed_derivative_ready','secret_or_pii_quarantined','raw_only_restricted')),
    original_byte_count BIGINT NOT NULL CHECK (original_byte_count>=0),
    stored_byte_count BIGINT NOT NULL CHECK (stored_byte_count>=0),
    truncated BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_raw_witness_sha256_v1_check CHECK (
        content_key ~ '^sha256:[0-9a-f]{64}$' AND vault_object_ref_token_hash ~ '^sha256:[0-9a-f]{64}$'
        AND sha256 ~ '^sha256:[0-9a-f]{64}$' AND ciphertext_sha256 ~ '^sha256:[0-9a-f]{64}$'
        AND operation_key_ref_hash ~ '^sha256:[0-9a-f]{64}$'
        AND retention_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND receipt_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (stored_byte_count<=original_byte_count),
    UNIQUE(id,receipt_id,execution_authority_id),
    CONSTRAINT capability_raw_witness_receipt_authority_fk
        FOREIGN KEY(receipt_id,execution_authority_id,receipt_authority_hash)
        REFERENCES capability_execution_receipts(id,execution_authority_id,receipt_authority_hash)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE capability_execution_receipts
    ADD CONSTRAINT capability_execution_receipt_raw_witness_exact_fk
    FOREIGN KEY(raw_witness_artifact_id,id,execution_authority_id)
    REFERENCES capability_raw_witness_artifacts(id,receipt_id,execution_authority_id)
    ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE TRIGGER capability_raw_witness_artifact_immutable
BEFORE UPDATE OR DELETE ON capability_raw_witness_artifacts
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_immutable('tool_truth_append_only');

CREATE TABLE capability_raw_witness_access_events (
    id UUID PRIMARY KEY, raw_witness_artifact_id UUID NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>=0), predecessor_event_id UUID,
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    purpose_code TEXT NOT NULL CHECK (BTRIM(purpose_code)<>''),
    decision TEXT NOT NULL CHECK (decision IN ('allowed','denied')),
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(raw_witness_artifact_id,event_ordinal),UNIQUE(id,raw_witness_artifact_id),
    FOREIGN KEY(raw_witness_artifact_id) REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_event_id,raw_witness_artifact_id)
        REFERENCES capability_raw_witness_access_events(id,raw_witness_artifact_id) ON DELETE RESTRICT
);
CREATE TABLE capability_raw_witness_retention_events (
    id UUID PRIMARY KEY, raw_witness_artifact_id UUID NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>=0), predecessor_event_id UUID,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('retention_extended','crypto_erased')),
    previous_policy_hash TEXT NOT NULL, next_policy_hash TEXT,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    event_hash TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_raw_retention_sha256_v1_check CHECK (
        previous_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (next_policy_hash IS NULL OR next_policy_hash ~ '^sha256:[0-9a-f]{64}$')
        AND event_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK ((event_kind='crypto_erased' AND next_policy_hash IS NULL)
        OR (event_kind='retention_extended' AND next_policy_hash IS NOT NULL)),
    UNIQUE(raw_witness_artifact_id,event_ordinal),UNIQUE(id,raw_witness_artifact_id),
    FOREIGN KEY(raw_witness_artifact_id) REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_event_id,raw_witness_artifact_id)
        REFERENCES capability_raw_witness_retention_events(id,raw_witness_artifact_id) ON DELETE RESTRICT
);
CREATE TRIGGER capability_raw_witness_access_append_only
BEFORE UPDATE OR DELETE ON capability_raw_witness_access_events
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();
CREATE TRIGGER capability_raw_witness_retention_append_only
BEFORE UPDATE OR DELETE ON capability_raw_witness_retention_events
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();

-- ---------------------------------------------------------------------------
-- Parser, temporal and input-lineage censuses.
-- ---------------------------------------------------------------------------

CREATE TABLE capability_parser_censuses (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL,
    receipt_authority_hash TEXT NOT NULL,
    raw_witness_artifact_id UUID NOT NULL,
    framer_contract_id TEXT NOT NULL CHECK (BTRIM(framer_contract_id)<>''),
    framer_contract_version TEXT NOT NULL CHECK (BTRIM(framer_contract_version)<>''),
    framer_digest TEXT NOT NULL, framing_manifest_hash TEXT NOT NULL,
    parser_contract_id TEXT NOT NULL CHECK (BTRIM(parser_contract_id)<>''),
    parser_contract_version TEXT NOT NULL CHECK (BTRIM(parser_contract_version)<>''),
    parser_digest TEXT NOT NULL,
    parse_domain_byte_count BIGINT NOT NULL CHECK (parse_domain_byte_count>=0),
    framed_record_count BIGINT NOT NULL CHECK (framed_record_count>=0),
    unaccounted_nonempty_record_count BIGINT NOT NULL DEFAULT 0
        CHECK (unaccounted_nonempty_record_count=0),
    member_count BIGINT, member_set_hash TEXT, sealed_empty BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(), sealed_at TIMESTAMPTZ,
    CONSTRAINT capability_parser_census_sha256_v1_check CHECK (
        framer_digest ~ '^sha256:[0-9a-f]{64}$' AND framing_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
        AND parser_digest ~ '^sha256:[0-9a-f]{64}$'
        AND receipt_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')),
    UNIQUE(id,receipt_id,execution_authority_id),
    FOREIGN KEY(receipt_id,execution_authority_id,receipt_authority_hash)
        REFERENCES capability_execution_receipts(id,execution_authority_id,receipt_authority_hash) ON DELETE RESTRICT,
    FOREIGN KEY(raw_witness_artifact_id,receipt_id,execution_authority_id)
        REFERENCES capability_raw_witness_artifacts(id,receipt_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TABLE capability_parser_census_members (
    id UUID PRIMARY KEY, census_id UUID NOT NULL, receipt_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    stream_kind TEXT NOT NULL CHECK (stream_kind IN ('envelope','stdout','stderr')),
    raw_start BIGINT NOT NULL CHECK (raw_start>=0), raw_end BIGINT NOT NULL CHECK (raw_end>raw_start),
    record_hash TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('parsed_observation','ignored_versioned','control_framing')),
    ignore_reason_code TEXT, ignore_rule_version TEXT, derived_child_identity_hash TEXT,
    member_hash TEXT NOT NULL,
    CONSTRAINT capability_parser_member_sha256_v1_check CHECK (
        record_hash ~ '^sha256:[0-9a-f]{64}$' AND member_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (derived_child_identity_hash IS NULL OR derived_child_identity_hash ~ '^sha256:[0-9a-f]{64}$')),
    CONSTRAINT capability_parser_member_shape_check CHECK (
        (disposition='ignored_versioned' AND ignore_reason_code IS NOT NULL
            AND BTRIM(ignore_reason_code)<>'' AND ignore_rule_version IS NOT NULL
            AND BTRIM(ignore_rule_version)<>'' AND derived_child_identity_hash IS NULL)
        OR (disposition<>'ignored_versioned' AND ignore_reason_code IS NULL AND ignore_rule_version IS NULL)),
    UNIQUE(census_id,ordinal),UNIQUE(census_id,stream_kind,raw_start,raw_end),
    UNIQUE(id,receipt_id,execution_authority_id,raw_start,raw_end),
    FOREIGN KEY(census_id,receipt_id,execution_authority_id)
        REFERENCES capability_parser_censuses(id,receipt_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER capability_parser_census_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_parser_censuses
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'capability_parser_census_members','census_id','member_hash','true');
CREATE TRIGGER capability_parser_census_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_parser_census_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member('capability_parser_censuses','id','census_id');

CREATE TABLE capability_typed_landing_source_members (
    id UUID PRIMARY KEY, receipt_id UUID NOT NULL, execution_authority_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0), input_key TEXT NOT NULL CHECK (BTRIM(input_key)<>''),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('raw_range','server_control')),
    raw_start BIGINT, raw_end BIGINT, parser_census_member_id UUID,
    normalized_observation_hash TEXT NOT NULL,
    CONSTRAINT capability_typed_source_sha256_v1_check CHECK (
        normalized_observation_hash ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT capability_typed_source_shape_check CHECK (
        (source_kind='raw_range' AND raw_start IS NOT NULL AND raw_end IS NOT NULL
            AND raw_start>=0 AND raw_end>raw_start AND parser_census_member_id IS NOT NULL)
        OR (source_kind='server_control' AND raw_start IS NULL AND raw_end IS NULL
            AND parser_census_member_id IS NULL)),
    UNIQUE(receipt_id,ordinal),
    FOREIGN KEY(receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(parser_census_member_id,receipt_id,execution_authority_id,raw_start,raw_end)
        REFERENCES capability_parser_census_members(id,receipt_id,execution_authority_id,raw_start,raw_end)
        ON DELETE RESTRICT
);
CREATE TRIGGER capability_typed_source_append_only
BEFORE UPDATE OR DELETE ON capability_typed_landing_source_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();

CREATE TABLE capability_execution_temporal_censuses (
    id UUID PRIMARY KEY, receipt_id UUID NOT NULL UNIQUE, execution_authority_id UUID NOT NULL,
    receipt_authority_hash TEXT NOT NULL,
    temporal_validity_policy_id UUID NOT NULL, temporal_validity_policy_hash TEXT NOT NULL,
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_completed_at TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL,
    member_count BIGINT, member_set_hash TEXT, sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(), sealed_at TIMESTAMPTZ,
    CONSTRAINT capability_temporal_census_sha256_v1_check CHECK (
        receipt_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND temporal_validity_policy_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')),
    CHECK (observation_window_completed_at>=observation_window_started_at
        AND effective_valid_until>observation_window_completed_at),
    UNIQUE(id,receipt_id,execution_authority_id),
    FOREIGN KEY(receipt_id,execution_authority_id,receipt_authority_hash)
        REFERENCES capability_execution_receipts(id,execution_authority_id,receipt_authority_hash) ON DELETE RESTRICT,
    FOREIGN KEY(temporal_validity_policy_id,execution_authority_id,temporal_validity_policy_hash)
        REFERENCES evidence_temporal_validity_policies(id,execution_authority_id,policy_hash) ON DELETE RESTRICT
);
CREATE TABLE capability_execution_temporal_census_members (
    id UUID PRIMARY KEY, census_id UUID NOT NULL, receipt_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    input_key TEXT NOT NULL, observation_identity_hash TEXT NOT NULL,
    temporal_fact_class TEXT NOT NULL CHECK (BTRIM(temporal_fact_class)<>''),
    observation_polarity TEXT NOT NULL CHECK (observation_polarity IN ('positive','negative','inconclusive')),
    mapping_rule_id TEXT NOT NULL CHECK (BTRIM(mapping_rule_id)<>''),
    mapping_rule_version TEXT NOT NULL CHECK (BTRIM(mapping_rule_version)<>''),
    mapping_rule_digest TEXT NOT NULL, source_valid_until TIMESTAMPTZ,
    selected_ttl_ms BIGINT NOT NULL CHECK (selected_ttl_ms>0), observed_at TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL, member_hash TEXT NOT NULL,
    CONSTRAINT capability_temporal_member_sha256_v1_check CHECK (
        observation_identity_hash ~ '^sha256:[0-9a-f]{64}$'
        AND mapping_rule_digest ~ '^sha256:[0-9a-f]{64}$' AND member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (effective_valid_until>observed_at
        AND (source_valid_until IS NULL OR effective_valid_until<=source_valid_until)),
    UNIQUE(census_id,ordinal),UNIQUE(census_id,input_key,observation_identity_hash),
    FOREIGN KEY(census_id,receipt_id,execution_authority_id)
        REFERENCES capability_execution_temporal_censuses(id,receipt_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER capability_temporal_census_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_temporal_censuses
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'capability_execution_temporal_census_members','census_id','member_hash','false');
CREATE TRIGGER capability_temporal_census_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_temporal_census_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'capability_execution_temporal_censuses','id','census_id');

ALTER TABLE capability_execution_receipts
    ADD CONSTRAINT capability_execution_receipt_parser_census_fk
        FOREIGN KEY(parser_census_id,id,execution_authority_id)
        REFERENCES capability_parser_censuses(id,receipt_id,execution_authority_id) ON DELETE RESTRICT,
    ADD CONSTRAINT capability_execution_receipt_temporal_census_fk
        FOREIGN KEY(temporal_census_id,id,execution_authority_id)
        REFERENCES capability_execution_temporal_censuses(id,receipt_id,execution_authority_id) ON DELETE RESTRICT;

CREATE TABLE capability_execution_budget_contract_axes (
    receipt_id UUID NOT NULL, execution_authority_id UUID NOT NULL,
    axis TEXT NOT NULL CHECK (axis IN ('requests','response_bytes','wall_clock_ms','retries','browser_steps','oast_tokens')),
    required_for_complete BOOLEAN NOT NULL, planned_limit BIGINT CHECK (planned_limit IS NULL OR planned_limit>=0),
    required_observation_source TEXT NOT NULL CHECK (
        required_observation_source IN ('host_governor','adapter_instrumentation','cli_unobserved')),
    PRIMARY KEY(receipt_id,axis),UNIQUE(receipt_id,axis,execution_authority_id),
    FOREIGN KEY(receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TABLE capability_execution_budget_observations (
    receipt_id UUID NOT NULL, execution_authority_id UUID NOT NULL, axis TEXT NOT NULL,
    actual_value BIGINT CHECK (actual_value IS NULL OR actual_value>=0), observed BOOLEAN NOT NULL DEFAULT FALSE,
    observation_source TEXT NOT NULL CHECK (
        observation_source IN ('host_governor','adapter_instrumentation','cli_unobserved')),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(receipt_id,axis),
    FOREIGN KEY(receipt_id,axis,execution_authority_id)
        REFERENCES capability_execution_budget_contract_axes(receipt_id,axis,execution_authority_id) ON DELETE RESTRICT,
    CHECK ((observed AND actual_value IS NOT NULL) OR (NOT observed AND actual_value IS NULL))
);
CREATE TRIGGER capability_budget_contract_append_only
BEFORE UPDATE OR DELETE ON capability_execution_budget_contract_axes
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();
CREATE TRIGGER capability_budget_observation_append_only
BEFORE UPDATE OR DELETE ON capability_execution_budget_observations
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();

CREATE TABLE capability_execution_receipt_inputs (
    id UUID PRIMARY KEY, receipt_id UUID NOT NULL, denominator_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL, input_key TEXT NOT NULL,
    attempt_state TEXT NOT NULL CHECK (attempt_state IN (
        'not_started','running','succeeded','failed','outcome_unknown','exhausted','superseded')),
    landing_state TEXT NOT NULL CHECK (landing_state IN ('not_attempted','partial','committed','failed')),
    observation_state TEXT NOT NULL CHECK (observation_state IN ('found','no_match','indeterminate','not_applicable')),
    coverage_extent TEXT NOT NULL CHECK (coverage_extent IN ('none','complete','partial','sampled','template_only')),
    coverage_gap_reason TEXT NOT NULL CHECK (coverage_gap_reason IN (
        'none','transport','tool_failure','parser_reject','budget_exhausted','unsupported','policy_blocked','source_unavailable')),
    member_count BIGINT, member_set_hash TEXT, sealed_empty BOOLEAN NOT NULL DEFAULT TRUE,
    sealed_at TIMESTAMPTZ,
    CONSTRAINT capability_receipt_input_sha256_v1_check CHECK (
        member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(receipt_id,denominator_item_id),UNIQUE(receipt_id,input_key),
    UNIQUE(id,receipt_id,denominator_item_id,execution_authority_id),
    FOREIGN KEY(receipt_id,denominator_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,denominator_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(denominator_item_id,denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TABLE capability_execution_input_evidence_members (
    id UUID PRIMARY KEY, input_id UUID NOT NULL, receipt_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL, execution_authority_id UUID NOT NULL,
    evidence_authority_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(input_id,ordinal),UNIQUE(input_id,evidence_authority_id),
    FOREIGN KEY(input_id,receipt_id,denominator_item_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(id,receipt_id,denominator_item_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(evidence_authority_id,execution_authority_id)
        REFERENCES tool_truth_evidence_authorities(id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TABLE capability_execution_input_business_ref_members (
    id UUID PRIMARY KEY, input_id UUID NOT NULL, receipt_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL, execution_authority_id UUID NOT NULL,
    business_ref_authority_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(input_id,ordinal),UNIQUE(input_id,business_ref_authority_id),
    FOREIGN KEY(input_id,receipt_id,denominator_item_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(id,receipt_id,denominator_item_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(business_ref_authority_id,execution_authority_id)
        REFERENCES tool_truth_business_ref_authorities(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_guard_receipt_input_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    evidence_count BIGINT;
    evidence_min BIGINT;
    evidence_max BIGINT;
    business_count BIGINT;
    business_min BIGINT;
    business_max BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='INSERT' THEN
        IF NEW.sealed_at IS NOT NULL OR NEW.member_count IS NOT NULL OR NEW.member_set_hash IS NOT NULL THEN
            RAISE EXCEPTION 'tool_truth_unsealed_authority' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP='DELETE' OR OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    IF (to_jsonb(NEW)-ARRAY['sealed_at','member_count','member_set_hash']) IS DISTINCT FROM
       (to_jsonb(OLD)-ARRAY['sealed_at','member_count','member_set_hash']) THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;

    SELECT count(*)::BIGINT,COALESCE(min(ordinal),0)::BIGINT,COALESCE(max(ordinal),-1)::BIGINT
      INTO evidence_count,evidence_min,evidence_max
      FROM capability_execution_input_evidence_members WHERE input_id=NEW.id;
    SELECT count(*)::BIGINT,COALESCE(min(ordinal),0)::BIGINT,COALESCE(max(ordinal),-1)::BIGINT
      INTO business_count,business_min,business_max
      FROM capability_execution_input_business_ref_members WHERE input_id=NEW.id;
    IF (evidence_count>0 AND (evidence_min<>0 OR evidence_max<>evidence_count-1))
       OR (business_count>0 AND (business_min<>0 OR business_max<>business_count-1)) THEN
        RAISE EXCEPTION 'tool_truth_set_ordinal_invalid' USING ERRCODE='23514';
    END IF;
    SELECT tool_truth_sha256(COALESCE(jsonb_agg(member ORDER BY source_kind,ordinal),'[]'::jsonb)::TEXT)
      INTO actual_hash
      FROM (
        SELECT 'evidence'::TEXT AS source_kind,ordinal,
               jsonb_build_object('kind','evidence','hash',member_hash) AS member
          FROM capability_execution_input_evidence_members WHERE input_id=NEW.id
        UNION ALL
        SELECT 'business_ref'::TEXT AS source_kind,ordinal,
               jsonb_build_object('kind','business_ref','hash',member_hash) AS member
          FROM capability_execution_input_business_ref_members WHERE input_id=NEW.id
      ) exact_members;
    NEW.member_count := evidence_count+business_count;
    NEW.member_set_hash := actual_hash;
    NEW.sealed_at := statement_timestamp();
    RETURN NEW;
END;
$$;

CREATE FUNCTION tool_truth_guard_receipt_input_member()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE parent_sealed_at TIMESTAMPTZ;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'tool_truth_member_append_only' USING ERRCODE='23514';
    END IF;
    SELECT sealed_at INTO parent_sealed_at
      FROM capability_execution_receipt_inputs WHERE id=NEW.input_id FOR SHARE;
    IF parent_sealed_at IS NOT NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER capability_receipt_input_header_guard
BEFORE UPDATE OR DELETE ON capability_execution_receipt_inputs
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_header();
CREATE TRIGGER capability_receipt_input_insert_guard
BEFORE INSERT ON capability_execution_receipt_inputs
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_header();
CREATE TRIGGER capability_input_evidence_member_guard
BEFORE UPDATE OR DELETE ON capability_execution_input_evidence_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_member();
CREATE TRIGGER capability_input_evidence_insert_guard
BEFORE INSERT ON capability_execution_input_evidence_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_member();
CREATE TRIGGER capability_input_business_member_guard
BEFORE UPDATE OR DELETE ON capability_execution_input_business_ref_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_member();
CREATE TRIGGER capability_input_business_insert_guard
BEFORE INSERT ON capability_execution_input_business_ref_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_receipt_input_member();

-- ---------------------------------------------------------------------------
-- Semantic reconciliation and freshness.
-- ---------------------------------------------------------------------------

CREATE TABLE capability_execution_reconciliations (
    id UUID PRIMARY KEY, receipt_id UUID NOT NULL, execution_authority_id UUID NOT NULL,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version>0),
    predecessor_reconciliation_id UUID,
    reconciliation_state TEXT NOT NULL CHECK (reconciliation_state IN ('pending','consistent','orphaned','superseded')),
    reason_code TEXT, observed_artifact_sha256 TEXT, observed_artifact_byte_count BIGINT,
    member_count BIGINT, member_set_hash TEXT, sealed_empty BOOLEAN NOT NULL DEFAULT TRUE,
    semantic_reconciliation_hash TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    CONSTRAINT capability_reconciliation_sha256_v1_check CHECK (
        (observed_artifact_sha256 IS NULL OR observed_artifact_sha256 ~ '^sha256:[0-9a-f]{64}$')
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')
        AND (semantic_reconciliation_hash IS NULL OR semantic_reconciliation_hash ~ '^sha256:[0-9a-f]{64}$')),
    CONSTRAINT capability_reconciliation_shape_check CHECK (
        (sealed_at IS NULL AND reconciliation_state='pending' AND semantic_reconciliation_hash IS NULL)
        OR (sealed_at IS NOT NULL AND reconciliation_state<>'pending'
            AND semantic_reconciliation_hash IS NOT NULL
            AND (reconciliation_state='consistent' OR (reason_code IS NOT NULL AND BTRIM(reason_code)<>'')))),
    UNIQUE(receipt_id,semantic_authority_version),UNIQUE(receipt_id,id,execution_authority_id),
    UNIQUE(receipt_id,id,semantic_authority_version,semantic_reconciliation_hash,execution_authority_id),
    FOREIGN KEY(receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(receipt_id,predecessor_reconciliation_id,execution_authority_id)
        REFERENCES capability_execution_reconciliations(receipt_id,id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TABLE capability_execution_reconciliation_members (
    id UUID PRIMARY KEY, reconciliation_id UUID NOT NULL, receipt_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL, ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('evidence','business_ref')),
    evidence_authority_id UUID, business_ref_authority_id UUID, member_hash TEXT NOT NULL,
    CONSTRAINT capability_reconciliation_member_sha256_v1_check CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT capability_reconciliation_member_shape_check CHECK (
        (source_kind='evidence' AND evidence_authority_id IS NOT NULL AND business_ref_authority_id IS NULL)
        OR (source_kind='business_ref' AND business_ref_authority_id IS NOT NULL AND evidence_authority_id IS NULL)),
    UNIQUE(reconciliation_id,ordinal),UNIQUE(reconciliation_id,evidence_authority_id),
    UNIQUE(reconciliation_id,business_ref_authority_id),
    FOREIGN KEY(reconciliation_id,receipt_id,execution_authority_id)
        REFERENCES capability_execution_reconciliations(id,receipt_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(evidence_authority_id,execution_authority_id)
        REFERENCES tool_truth_evidence_authorities(id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(business_ref_authority_id,execution_authority_id)
        REFERENCES tool_truth_business_ref_authorities(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_guard_reconciliation_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    min_ordinal BIGINT;
    max_ordinal BIGINT;
    actual_member_hash TEXT;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.sealed_at IS NOT NULL
           OR NEW.reconciliation_state <> 'pending'
           OR NEW.member_count IS NOT NULL
           OR NEW.member_set_hash IS NOT NULL
           OR NEW.semantic_reconciliation_hash IS NOT NULL THEN
            RAISE EXCEPTION 'tool_truth_unsealed_authority' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'DELETE' OR OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    IF (to_jsonb(NEW) - ARRAY[
            'sealed_at','member_count','member_set_hash','reconciliation_state',
            'reason_code','observed_artifact_sha256','observed_artifact_byte_count',
            'semantic_reconciliation_hash'
        ]) IS DISTINCT FROM
       (to_jsonb(OLD) - ARRAY[
            'sealed_at','member_count','member_set_hash','reconciliation_state',
            'reason_code','observed_artifact_sha256','observed_artifact_byte_count',
            'semantic_reconciliation_hash'
        ]) THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    IF NEW.reconciliation_state = 'pending' THEN
        RAISE EXCEPTION 'tool_truth_reconciliation_terminal_state_required' USING ERRCODE='23514';
    END IF;
    IF NEW.reconciliation_state <> 'consistent'
       AND (NEW.reason_code IS NULL OR BTRIM(NEW.reason_code)='') THEN
        RAISE EXCEPTION 'tool_truth_reconciliation_reason_required' USING ERRCODE='23514';
    END IF;

    SELECT count(*)::BIGINT,
           COALESCE(min(ordinal),0)::BIGINT,
           COALESCE(max(ordinal),-1)::BIGINT,
           tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY ordinal),'[]'::jsonb)::TEXT)
      INTO actual_count,min_ordinal,max_ordinal,actual_member_hash
      FROM capability_execution_reconciliation_members
     WHERE reconciliation_id=NEW.id;
    IF actual_count>0 AND (min_ordinal<>0 OR max_ordinal<>actual_count-1) THEN
        RAISE EXCEPTION 'tool_truth_set_ordinal_invalid' USING ERRCODE='23514';
    END IF;

    NEW.member_count := actual_count;
    NEW.member_set_hash := actual_member_hash;
    NEW.semantic_reconciliation_hash := tool_truth_sha256(jsonb_build_object(
        'receipt_id',NEW.receipt_id,
        'execution_authority_id',NEW.execution_authority_id,
        'semantic_authority_version',NEW.semantic_authority_version,
        'predecessor_reconciliation_id',NEW.predecessor_reconciliation_id,
        'reconciliation_state',NEW.reconciliation_state,
        'reason_code',NEW.reason_code,
        'observed_artifact_sha256',NEW.observed_artifact_sha256,
        'observed_artifact_byte_count',NEW.observed_artifact_byte_count,
        'member_set_hash',actual_member_hash
    )::TEXT);
    NEW.sealed_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER capability_reconciliation_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_reconciliations
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_reconciliation_header();
CREATE TRIGGER capability_reconciliation_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_reconciliation_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'capability_execution_reconciliations','id','reconciliation_id');

ALTER TABLE capability_execution_receipts
    ADD CONSTRAINT capability_execution_receipt_current_reconciliation_fk
    FOREIGN KEY(id,current_semantic_reconciliation_id,current_semantic_authority_version,
                current_semantic_reconciliation_hash,execution_authority_id)
    REFERENCES capability_execution_reconciliations(receipt_id,id,semantic_authority_version,
                semantic_reconciliation_hash,execution_authority_id) ON DELETE RESTRICT;

CREATE TABLE capability_execution_freshness_attestations (
    id UUID PRIMARY KEY, receipt_id UUID NOT NULL, reconciliation_id UUID NOT NULL,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version>0),
    semantic_hash TEXT NOT NULL, execution_authority_id UUID NOT NULL,
    predecessor_attestation_id UUID, event_ordinal BIGINT NOT NULL CHECK (event_ordinal>=0),
    consumer_kind TEXT NOT NULL CHECK (BTRIM(consumer_kind)<>''), stable_consumer_request_id UUID NOT NULL,
    artifact_object_identity_hash TEXT NOT NULL, snapshot_sha256 TEXT NOT NULL,
    snapshot_byte_count BIGINT NOT NULL CHECK (snapshot_byte_count>=0),
    freshness_status TEXT NOT NULL CHECK (freshness_status IN ('consistent','orphaned')),
    attestation_hash TEXT NOT NULL, checked_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_freshness_sha256_v1_check CHECK (
        semantic_hash ~ '^sha256:[0-9a-f]{64}$' AND artifact_object_identity_hash ~ '^sha256:[0-9a-f]{64}$'
        AND snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$' AND attestation_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(stable_consumer_request_id,receipt_id),UNIQUE(receipt_id,event_ordinal),
    UNIQUE(id,receipt_id),
    UNIQUE(id,receipt_id,reconciliation_id,semantic_authority_version,semantic_hash,execution_authority_id),
    FOREIGN KEY(receipt_id,reconciliation_id,semantic_authority_version,semantic_hash,execution_authority_id)
        REFERENCES capability_execution_reconciliations(receipt_id,id,semantic_authority_version,
            semantic_reconciliation_hash,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_attestation_id,receipt_id)
        REFERENCES capability_execution_freshness_attestations(id,receipt_id) ON DELETE RESTRICT
);
CREATE TRIGGER capability_freshness_append_only
BEFORE UPDATE OR DELETE ON capability_execution_freshness_attestations
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();

-- ---------------------------------------------------------------------------
-- Task 9: source freeze, dynamic-child closure and shadow Gate authorities.
-- ---------------------------------------------------------------------------

CREATE UNIQUE INDEX stage_asset_waves_one_running_per_stage_org
    ON stage_asset_waves(operation_id,organization_id,stage_kind)
    WHERE status='running';

CREATE FUNCTION tool_truth_guard_bound_wave_source()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE source_wave_id UUID;
BEGIN
    source_wave_id := COALESCE(
        to_jsonb(NEW)->>'wave_id',to_jsonb(OLD)->>'wave_id',
        to_jsonb(NEW)->>'id',to_jsonb(OLD)->>'id'
    )::UUID;
    IF NOT EXISTS (
        SELECT 1 FROM tool_truth_stage_wave_execution_bindings
         WHERE stage_asset_wave_id=source_wave_id
    ) THEN
        RETURN COALESCE(NEW,OLD);
    END IF;
    IF TG_TABLE_NAME='stage_asset_waves' AND TG_OP='UPDATE'
       AND to_jsonb(OLD)->>'status'='running'
       AND to_jsonb(NEW)->>'status'='completed'
       AND (to_jsonb(NEW)-ARRAY['status','completed_at','updated_at'])
           IS NOT DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['status','completed_at','updated_at']) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'tool_truth_bound_wave_source_immutable' USING ERRCODE='23514';
END;
$$;
CREATE TRIGGER tool_truth_bound_wave_header_guard
BEFORE UPDATE OR DELETE ON stage_asset_waves
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_bound_wave_source();
CREATE TRIGGER tool_truth_bound_wave_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON stage_asset_wave_items
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_bound_wave_source();

CREATE TABLE capability_discovered_child_manifests (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    parent_receipt_id UUID NOT NULL,
    parent_receipt_authority_hash TEXT NOT NULL,
    parent_denominator_id UUID NOT NULL,
    parent_denominator_item_id UUID NOT NULL,
    child_kind TEXT NOT NULL CHECK (BTRIM(child_kind)<>''),
    capability_contract_version TEXT NOT NULL CHECK (BTRIM(capability_contract_version)<>''),
    capability_contract_hash TEXT NOT NULL,
    expected_downstream_technique TEXT NOT NULL CHECK (BTRIM(expected_downstream_technique)<>''),
    expected_downstream_capability TEXT NOT NULL CHECK (BTRIM(expected_downstream_capability)<>''),
    manifest_hash TEXT NOT NULL,
    member_count BIGINT,
    member_set_hash TEXT,
    sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    sealed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_child_manifest_sha256_v1_check CHECK (
        parent_receipt_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        AND capability_contract_hash ~ '^sha256:[0-9a-f]{64}$'
        AND manifest_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')
    ),
    CONSTRAINT capability_child_manifest_seal_shape_check CHECK (
        (sealed_at IS NULL AND member_count IS NULL AND member_set_hash IS NULL)
        OR (sealed_at IS NOT NULL AND member_count IS NOT NULL AND member_count>=0
            AND member_set_hash IS NOT NULL AND sealed_empty=(member_count=0))
    ),
    UNIQUE(parent_receipt_id,child_kind),
    UNIQUE(id,execution_authority_id),
    FOREIGN KEY(parent_receipt_id,execution_authority_id,parent_receipt_authority_hash)
        REFERENCES capability_execution_receipts(id,execution_authority_id,receipt_authority_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(parent_denominator_item_id,parent_denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION tool_truth_validate_child_manifest_parent()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM capability_execution_receipts r
         WHERE r.id=NEW.parent_receipt_id
           AND r.denominator_id=NEW.parent_denominator_id
           AND r.execution_authority_id=NEW.execution_authority_id
    ) THEN
        RAISE EXCEPTION 'tool_truth_child_parent_splice' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER capability_child_manifest_parent_guard
BEFORE INSERT ON capability_discovered_child_manifests
FOR EACH ROW EXECUTE FUNCTION tool_truth_validate_child_manifest_parent();

CREATE TABLE capability_discovered_child_members (
    id UUID PRIMARY KEY,
    manifest_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    child_key TEXT NOT NULL CHECK (BTRIM(child_key)<>''),
    exact_child_asset TEXT NOT NULL CHECK (BTRIM(exact_child_asset)<>''),
    canonical_child_identity_hash TEXT NOT NULL,
    scope_classification TEXT NOT NULL CHECK (
        scope_classification IN ('in_scope','out_of_scope','not_applicable','blocked')
    ),
    expected_downstream_technique TEXT NOT NULL CHECK (BTRIM(expected_downstream_technique)<>''),
    expected_downstream_capability TEXT NOT NULL CHECK (BTRIM(expected_downstream_capability)<>''),
    member_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_child_member_sha256_v1_check CHECK (
        canonical_child_identity_hash ~ '^sha256:[0-9a-f]{64}$'
        AND member_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    UNIQUE(manifest_id,ordinal),
    UNIQUE(manifest_id,child_key),
    UNIQUE(id,manifest_id,execution_authority_id),
    FOREIGN KEY(manifest_id,execution_authority_id)
        REFERENCES capability_discovered_child_manifests(id,execution_authority_id)
        ON DELETE RESTRICT
);
CREATE TRIGGER capability_child_manifest_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_discovered_child_manifests
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'capability_discovered_child_members','manifest_id','member_hash','true');
CREATE TRIGGER capability_child_manifest_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON capability_discovered_child_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'capability_discovered_child_manifests','id','manifest_id');

CREATE TABLE capability_discovered_child_closures (
    id UUID PRIMARY KEY,
    child_member_id UUID NOT NULL UNIQUE,
    manifest_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    closure_kind TEXT NOT NULL CHECK (
        closure_kind IN ('derived_terminal','not_applicable','blocked','out_of_scope')
    ),
    derived_denominator_id UUID,
    derived_denominator_item_id UUID,
    residual JSONB,
    closure_hash TEXT NOT NULL CHECK (closure_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT capability_child_closure_shape_check CHECK (
        (closure_kind='derived_terminal' AND derived_denominator_id IS NOT NULL
            AND derived_denominator_item_id IS NOT NULL AND residual IS NULL)
        OR (closure_kind<>'derived_terminal' AND derived_denominator_id IS NULL
            AND derived_denominator_item_id IS NULL AND jsonb_typeof(residual)='object')
    ),
    FOREIGN KEY(child_member_id,manifest_id,execution_authority_id)
        REFERENCES capability_discovered_child_members(id,manifest_id,execution_authority_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(derived_denominator_item_id,derived_denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id)
        ON DELETE RESTRICT
);
CREATE TRIGGER capability_child_closure_append_only
BEFORE UPDATE OR DELETE ON capability_discovered_child_closures
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();

CREATE TABLE tool_truth_authority_set_seals (
    id UUID PRIMARY KEY,
    stable_consumer_request_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    denominator_hash TEXT NOT NULL,
    consumer_kind TEXT NOT NULL CHECK (BTRIM(consumer_kind)<>''),
    graph_hash TEXT NOT NULL,
    semantic_hash TEXT NOT NULL,
    freshness_hash TEXT NOT NULL,
    member_count BIGINT,
    member_set_hash TEXT,
    sealed_empty BOOLEAN NOT NULL DEFAULT FALSE,
    sealed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_authority_set_sha256_v1_check CHECK (
        denominator_hash ~ '^sha256:[0-9a-f]{64}$'
        AND graph_hash ~ '^sha256:[0-9a-f]{64}$'
        AND semantic_hash ~ '^sha256:[0-9a-f]{64}$'
        AND freshness_hash ~ '^sha256:[0-9a-f]{64}$'
        AND (member_set_hash IS NULL OR member_set_hash ~ '^sha256:[0-9a-f]{64}$')
    ),
    UNIQUE(execution_authority_id,stable_consumer_request_id),
    UNIQUE(id,execution_authority_id,denominator_id),
    FOREIGN KEY(denominator_id,execution_authority_id,denominator_hash)
        REFERENCES coverage_denominators(id,execution_authority_id,denominator_hash)
        ON DELETE RESTRICT
);

CREATE TABLE tool_truth_authority_set_members (
    id UUID PRIMARY KEY,
    authority_set_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    receipt_id UUID NOT NULL,
    reconciliation_id UUID NOT NULL,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version>0),
    semantic_hash TEXT NOT NULL,
    freshness_attestation_id UUID,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL,
    CONSTRAINT tool_truth_authority_set_member_sha256_v1_check CHECK (
        semantic_hash ~ '^sha256:[0-9a-f]{64}$'
        AND member_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    UNIQUE(authority_set_id,ordinal),
    UNIQUE(authority_set_id,receipt_id),
    FOREIGN KEY(authority_set_id,execution_authority_id,denominator_id)
        REFERENCES tool_truth_authority_set_seals(id,execution_authority_id,denominator_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(receipt_id,reconciliation_id,semantic_authority_version,semantic_hash,execution_authority_id)
        REFERENCES capability_execution_reconciliations(
            receipt_id,id,semantic_authority_version,semantic_reconciliation_hash,execution_authority_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(freshness_attestation_id,receipt_id)
        REFERENCES capability_execution_freshness_attestations(id,receipt_id) ON DELETE RESTRICT
);
CREATE TRIGGER tool_truth_authority_set_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON tool_truth_authority_set_seals
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_header(
    'tool_truth_authority_set_members','authority_set_id','member_hash','true');
CREATE TRIGGER tool_truth_authority_set_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON tool_truth_authority_set_members
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_set_member(
    'tool_truth_authority_set_seals','id','authority_set_id');

CREATE TABLE tool_truth_gate_assessments (
    id UUID PRIMARY KEY,
    stable_gate_request_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_id UUID NOT NULL,
    execution_authority_hash TEXT NOT NULL CHECK (
        execution_authority_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    assessment_basis_kind TEXT NOT NULL CHECK (
        assessment_basis_kind IN ('authority_set','missing_denominator')
    ),
    denominator_id UUID,
    authority_set_id UUID,
    legacy_allowed BOOLEAN NOT NULL,
    control_decision TEXT NOT NULL CHECK (control_decision IN ('allow','hold')),
    coverage_grade TEXT NOT NULL CHECK (coverage_grade IN ('complete','degraded','incomplete')),
    divergence BOOLEAN NOT NULL,
    expected_item_count BIGINT NOT NULL CHECK (expected_item_count>=0),
    terminal_item_count BIGINT NOT NULL CHECK (terminal_item_count>=0),
    degraded_item_count BIGINT NOT NULL CHECK (degraded_item_count>=0),
    residual JSONB NOT NULL CHECK (jsonb_typeof(residual)='object'),
    assessment_hash TEXT NOT NULL CHECK (assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tool_truth_gate_assessment_basis_shape_check CHECK (
        (assessment_basis_kind='authority_set' AND denominator_id IS NOT NULL
            AND authority_set_id IS NOT NULL)
        OR (assessment_basis_kind='missing_denominator' AND denominator_id IS NULL
            AND authority_set_id IS NULL AND control_decision='hold'
            AND coverage_grade='incomplete')
    ),
    CONSTRAINT tool_truth_gate_assessment_decision_check CHECK (
        divergence=(legacy_allowed<>(control_decision='allow'))
        AND NOT (coverage_grade='incomplete' AND control_decision='allow')
        AND NOT (coverage_grade='complete' AND control_decision='hold')
    ),
    UNIQUE(operation_id,stable_gate_request_id),
    CONSTRAINT tool_truth_gate_assessment_authority_fk
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
                execution_authority_hash)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id,stage_kind,authority_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(authority_set_id,execution_authority_id,denominator_id)
        REFERENCES tool_truth_authority_set_seals(id,execution_authority_id,denominator_id)
        ON DELETE RESTRICT
);
CREATE TRIGGER tool_truth_gate_assessment_append_only
BEFORE UPDATE OR DELETE ON tool_truth_gate_assessments
FOR EACH ROW EXECUTE FUNCTION tool_truth_reject_append_only();
