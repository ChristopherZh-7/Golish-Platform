-- Candidate attack-execution V2 additive foundation.
--
-- This migration freezes the per-operation attack rollout contract and adds a
-- retained, frozen-identity ownership spine from Wave -> Candidate -> Approval
-- -> Attempt -> Evidence -> FindingLineage. Live organization rows are not
-- referenced by the retained audit tables; live target references are nullable
-- and use ON DELETE SET NULL. Repository authority and runtime cutover land in
-- later tasks.

-- ---------------------------------------------------------------------------
-- Deployment default and immutable operation contract
-- ---------------------------------------------------------------------------

CREATE TABLE attack_execution_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    contract TEXT NOT NULL CHECK (
        contract IN (
            'legacy',
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback',
            'v2_only'
        )
    ),
    rank SMALLINT NOT NULL CHECK (rank BETWEEN 0 AND 3),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        rank = CASE contract
            WHEN 'legacy' THEN 0
            WHEN 'dual_write_read_legacy' THEN 1
            WHEN 'dual_write_read_v2_fallback' THEN 2
            WHEN 'v2_only' THEN 3
        END
    )
);

INSERT INTO attack_execution_rollout(singleton, contract, rank)
VALUES (TRUE, 'legacy', 0);

CREATE FUNCTION enforce_attack_execution_rollout_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'attack execution rollout singleton cannot be deleted';
    END IF;
    IF NEW.singleton IS DISTINCT FROM OLD.singleton
        OR NEW.rank <> OLD.rank + 1
        OR NEW.row_version <> OLD.row_version + 1
    THEN
        RAISE EXCEPTION 'attack execution rollout must advance one rank and one row version';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_rollout_forward_only
BEFORE UPDATE OR DELETE ON attack_execution_rollout
FOR EACH ROW EXECUTE FUNCTION enforce_attack_execution_rollout_transition();

ALTER TABLE operation_state
    ADD COLUMN attack_execution_contract TEXT NOT NULL DEFAULT 'legacy'
    CHECK (
        attack_execution_contract IN (
            'legacy',
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback',
            'v2_only'
        )
    );

ALTER TABLE operation_state
    ADD CONSTRAINT operation_v2_attack_requires_v2_runtime
    CHECK (
        attack_execution_contract <> 'v2_only'
        OR runtime_memory_contract = 'v2_only'
    );

CREATE FUNCTION reject_operation_attack_contract_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.attack_execution_contract IS DISTINCT FROM OLD.attack_execution_contract THEN
        RAISE EXCEPTION 'operation attack execution contract is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_attack_contract_immutable
BEFORE UPDATE OF attack_execution_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_operation_attack_contract_change();

-- ---------------------------------------------------------------------------
-- Wave and trusted per-organization units
-- ---------------------------------------------------------------------------

-- Candidate acceptance must be able to bind the exact current-stage final
-- handoff without weakening the immutable foundation migration.
ALTER TABLE stage_handoffs
    ADD CONSTRAINT stage_handoffs_attack_authority_unique UNIQUE (
        deliverable_submission_id,
        operation_id,
        stage_execution_id,
        source_stage_run_unit_id,
        organization_id,
        from_stage_kind,
        scope_snapshot_id
    );

CREATE TABLE attack_wave_runs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'review', 'verification', 'terminal')),
    policy_snapshot JSONB NOT NULL CHECK (jsonb_typeof(policy_snapshot) = 'object'),
    policy_hash TEXT NOT NULL CHECK (BTRIM(policy_hash) <> ''),
    max_waves INTEGER NOT NULL CHECK (max_waves > 0),
    max_candidates_total INTEGER NOT NULL CHECK (max_candidates_total > 0),
    max_chain_depth INTEGER NOT NULL CHECK (max_chain_depth >= 0),
    max_attempts_total INTEGER NOT NULL CHECK (max_attempts_total > 0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (operation_id, generation),
    UNIQUE (id, operation_id, scope_snapshot_id),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    CHECK (
        (status = 'terminal' AND terminal_at IS NOT NULL)
        OR (status <> 'terminal' AND terminal_at IS NULL)
    )
);

CREATE TABLE attack_wave_units (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    wave_run_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    entry_stage_execution_id UUID NOT NULL,
    entry_stage_run_unit_id UUID NOT NULL,
    entry_deliverable_submission_id UUID NOT NULL,
    entry_stage_kind TEXT NOT NULL CHECK (entry_stage_kind = 'vuln_triage'),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'reasoning', 'review', 'verification', 'terminal')),
    review_closed BOOLEAN NOT NULL DEFAULT FALSE,
    verification_closed BOOLEAN NOT NULL DEFAULT FALSE,
    consolidation_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (consolidation_status IN ('pending', 'ready', 'consumed', 'terminal')),
    manifest_hash TEXT,
    manifest_count INTEGER,
    manifest_frozen_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (wave_run_id, organization_id),
    UNIQUE (wave_run_id, ordinal),
    UNIQUE (entry_deliverable_submission_id),
    UNIQUE (id, operation_id, scope_snapshot_id, organization_id),
    UNIQUE (id, wave_run_id, operation_id, scope_snapshot_id, organization_id),
    UNIQUE (
        id,
        wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        entry_stage_run_unit_id,
        entry_deliverable_submission_id
    ),
    FOREIGN KEY (wave_run_id, operation_id, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        entry_stage_run_unit_id,
        operation_id,
        entry_stage_execution_id,
        organization_id,
        entry_stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        entry_deliverable_submission_id,
        operation_id,
        entry_stage_execution_id,
        entry_stage_run_unit_id,
        organization_id,
        entry_stage_kind
    ) REFERENCES stage_deliverable_submissions(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    CHECK (
        (status = 'terminal' AND terminal_at IS NOT NULL)
        OR (status <> 'terminal' AND terminal_at IS NULL)
    ),
    CHECK (
        (manifest_hash IS NULL AND manifest_count IS NULL AND manifest_frozen_at IS NULL)
        OR (
            BTRIM(COALESCE(manifest_hash, '')) <> ''
            AND manifest_count > 0
            AND manifest_frozen_at IS NOT NULL
        )
    )
);

CREATE FUNCTION enforce_attack_wave_entry_final_pass()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM stage_run_units AS source_unit
          JOIN stage_handoffs AS handoff
            ON handoff.operation_id = source_unit.operation_id
           AND handoff.scope_snapshot_id = source_unit.scope_snapshot_id
           AND handoff.organization_id = source_unit.organization_id
           AND handoff.source_stage_run_unit_id = source_unit.id
           AND handoff.deliverable_submission_id = NEW.entry_deliverable_submission_id
           AND handoff.invalidated_at IS NULL
         WHERE source_unit.id = NEW.entry_stage_run_unit_id
           AND source_unit.operation_id = NEW.operation_id
           AND source_unit.stage_execution_id = NEW.entry_stage_execution_id
           AND source_unit.scope_snapshot_id = NEW.scope_snapshot_id
           AND source_unit.organization_id = NEW.organization_id
           AND source_unit.stage_kind = NEW.entry_stage_kind
           AND source_unit.status = 'passed'
           AND source_unit.terminal_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'attack wave entry requires exact final-passed StageRunUnit and immutable handoff';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_units_require_final_pass_entry
BEFORE INSERT OR UPDATE OF operation_id, scope_snapshot_id, organization_id,
    entry_stage_execution_id, entry_stage_run_unit_id,
    entry_deliverable_submission_id, entry_stage_kind
ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION enforce_attack_wave_entry_final_pass();

CREATE FUNCTION validate_attack_live_target_owner()
RETURNS trigger AS $$
BEGIN
    IF NEW.target_live_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
          FROM targets AS target
         WHERE target.id = NEW.target_live_id
           AND target.organization_id = NEW.organization_id
    ) THEN
        RAISE EXCEPTION 'attack live target does not belong to frozen organization';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- Observation seeds and complete reasoning manifest
-- ---------------------------------------------------------------------------

CREATE TABLE attack_candidate_seeds (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    wave_unit_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    technique TEXT NOT NULL CHECK (
        BTRIM(technique) <> '' AND OCTET_LENGTH(technique) <= 128
    ),
    observation JSONB NOT NULL CHECK (
        jsonb_typeof(observation) = 'object' AND PG_COLUMN_SIZE(observation) <= 65536
    ),
    observation_hash TEXT NOT NULL CHECK (BTRIM(observation_hash) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (wave_unit_id, target_identity_hash, technique, observation_hash),
    UNIQUE (
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        target_identity_hash,
        target_type_at_time,
        target_value_at_time
    ),
    FOREIGN KEY (wave_unit_id, operation_id, scope_snapshot_id, organization_id)
        REFERENCES attack_wave_units(
            id,
            operation_id,
            scope_snapshot_id,
            organization_id
        ) ON DELETE RESTRICT
);

CREATE TRIGGER attack_candidate_seeds_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_candidate_seeds
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE TABLE attack_candidate_work_items (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    seed_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    work_item_key TEXT NOT NULL CHECK (
        BTRIM(work_item_key) <> '' AND OCTET_LENGTH(work_item_key) <= 256
    ),
    decision_kind TEXT CHECK (decision_kind IN ('candidate', 'no_candidate')),
    candidate_id UUID,
    no_candidate_reason_code TEXT,
    no_candidate_detail TEXT,
    decided_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (wave_unit_id, work_item_key),
    UNIQUE (
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        target_identity_hash,
        target_type_at_time,
        target_value_at_time
    ),
    FOREIGN KEY (wave_unit_id, operation_id, scope_snapshot_id, organization_id)
        REFERENCES attack_wave_units(
            id,
            operation_id,
            scope_snapshot_id,
            organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY (
        seed_id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        target_identity_hash,
        target_type_at_time,
        target_value_at_time
    ) REFERENCES attack_candidate_seeds(
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        target_identity_hash,
        target_type_at_time,
        target_value_at_time
    ) ON DELETE RESTRICT,
    CHECK (
        (
            decision_kind IS NULL
            AND candidate_id IS NULL
            AND no_candidate_reason_code IS NULL
            AND no_candidate_detail IS NULL
            AND decided_at IS NULL
        )
        OR
        (
            decision_kind = 'candidate'
            AND candidate_id IS NOT NULL
            AND no_candidate_reason_code IS NULL
            AND no_candidate_detail IS NULL
            AND decided_at IS NOT NULL
        )
        OR
        (
            decision_kind = 'no_candidate'
            AND candidate_id IS NULL
            AND no_candidate_reason_code ~ '^[a-z0-9_]{1,64}$'
            AND BTRIM(COALESCE(no_candidate_detail, '')) <> ''
            AND OCTET_LENGTH(no_candidate_detail) <= 8192
            AND decided_at IS NOT NULL
        )
    )
);

CREATE TRIGGER attack_candidate_work_items_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE FUNCTION reject_attack_manifest_attestation_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.manifest_frozen_at IS NOT NULL AND (
        NEW.manifest_count > 100
        OR NOT EXISTS (
            SELECT 1 FROM attack_wave_runs AS wave
             WHERE wave.id = NEW.wave_run_id
               AND wave.operation_id = NEW.operation_id
               AND wave.scope_snapshot_id = NEW.scope_snapshot_id
               AND NEW.manifest_count <= wave.max_candidates_total
        )
    ) THEN
        RAISE EXCEPTION 'attack manifest exceeds frozen Wave policy';
    END IF;
    IF OLD.manifest_frozen_at IS NOT NULL AND (
        NEW.manifest_hash IS DISTINCT FROM OLD.manifest_hash
        OR NEW.manifest_count IS DISTINCT FROM OLD.manifest_count
        OR NEW.manifest_frozen_at IS DISTINCT FROM OLD.manifest_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen attack manifest attestation is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_manifest_attestation_immutable
BEFORE UPDATE OF manifest_hash,manifest_count,manifest_frozen_at
ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION reject_attack_manifest_attestation_change();

CREATE FUNCTION reject_frozen_attack_manifest_row_change()
RETURNS trigger AS $$
DECLARE
    owner_wave_unit_id UUID;
BEGIN
    owner_wave_unit_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.wave_unit_id ELSE NEW.wave_unit_id END;
    IF EXISTS (
        SELECT 1 FROM attack_wave_units
         WHERE id = owner_wave_unit_id AND manifest_frozen_at IS NOT NULL
    ) THEN
        IF TG_OP = 'INSERT' THEN
            IF TG_TABLE_NAME = 'attack_candidate_seeds' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_seeds AS existing
                     WHERE existing.wave_unit_id = NEW.wave_unit_id
                       AND existing.target_identity_hash = NEW.target_identity_hash
                       AND existing.technique = NEW.technique
                       AND existing.observation_hash = NEW.observation_hash
                       AND existing.operation_id = NEW.operation_id
                       AND existing.scope_snapshot_id = NEW.scope_snapshot_id
                       AND existing.organization_id = NEW.organization_id
                       AND existing.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
                       AND existing.target_type_at_time = NEW.target_type_at_time
                       AND existing.target_value_at_time = NEW.target_value_at_time
                       AND existing.observation = NEW.observation
                ) THEN
                    RETURN NEW;
                END IF;
            ELSIF TG_TABLE_NAME = 'attack_candidate_work_items' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_work_items AS existing
                     WHERE existing.wave_unit_id = NEW.wave_unit_id
                       AND existing.work_item_key = NEW.work_item_key
                       AND existing.seed_id = NEW.seed_id
                       AND existing.operation_id = NEW.operation_id
                       AND existing.scope_snapshot_id = NEW.scope_snapshot_id
                       AND existing.organization_id = NEW.organization_id
                       AND existing.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
                       AND existing.target_type_at_time = NEW.target_type_at_time
                       AND existing.target_value_at_time = NEW.target_value_at_time
                       AND existing.target_identity_hash = NEW.target_identity_hash
                ) THEN
                    RETURN NEW;
                END IF;
            END IF;
        END IF;
        RAISE EXCEPTION 'frozen attack manifest rows are immutable';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_seeds_frozen_insert_delete
BEFORE INSERT OR DELETE ON attack_candidate_seeds
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_row_change();

CREATE TRIGGER attack_candidate_seeds_frozen_identity_update
BEFORE UPDATE OF wave_unit_id,operation_id,scope_snapshot_id,organization_id,
    target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
    technique,observation,observation_hash
ON attack_candidate_seeds
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_row_change();

CREATE TRIGGER attack_candidate_work_items_frozen_insert_delete
BEFORE INSERT OR DELETE ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_row_change();

CREATE TRIGGER attack_candidate_work_items_frozen_identity_update
BEFORE UPDATE OF seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
    target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,work_item_key
ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_row_change();

-- ---------------------------------------------------------------------------
-- Retained V2 Candidate identity and legacy index correction
-- ---------------------------------------------------------------------------

ALTER TABLE attack_candidates
    DROP CONSTRAINT IF EXISTS attack_candidates_organization_id_fkey;

ALTER TABLE attack_candidates
    ADD COLUMN operation_uuid UUID REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    ADD COLUMN scope_snapshot_id UUID,
    ADD COLUMN wave_run_id UUID,
    ADD COLUMN wave_unit_id UUID,
    ADD COLUMN source_work_item_id UUID,
    ADD COLUMN decision_stage_execution_id UUID,
    ADD COLUMN decision_stage_run_unit_id UUID,
    ADD COLUMN decision_deliverable_submission_id UUID,
    ADD COLUMN decision_stage_kind TEXT,
    ADD COLUMN target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    ADD COLUMN target_type_at_time TEXT,
    ADD COLUMN target_value_at_time TEXT,
    ADD COLUMN target_identity_hash TEXT,
    ADD COLUMN execution_plan JSONB,
    ADD COLUMN candidate_plan_hash TEXT,
    ADD COLUMN risk_class TEXT,
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    ADD COLUMN terminal_attempt_id UUID,
    ADD COLUMN terminal_finding_id UUID;

DROP INDEX IF EXISTS uq_attack_candidates_op_target_hash;

CREATE UNIQUE INDEX uq_attack_candidates_legacy_op_target_hash
    ON attack_candidates(operation_id, target, hypothesis_hash)
    WHERE operation_uuid IS NULL;

CREATE UNIQUE INDEX uq_attack_candidates_v2_identity
    ON attack_candidates(operation_uuid, organization_id, target_identity_hash, hypothesis_hash)
    WHERE operation_uuid IS NOT NULL;

CREATE UNIQUE INDEX uq_attack_candidates_v2_source_work_item
    ON attack_candidates(source_work_item_id)
    WHERE operation_uuid IS NOT NULL;

ALTER TABLE attack_candidates
    ADD CONSTRAINT attack_candidates_v2_shape_check CHECK (
        operation_uuid IS NULL
        OR (
            organization_id IS NOT NULL
            AND scope_snapshot_id IS NOT NULL
            AND wave_run_id IS NOT NULL
            AND wave_unit_id IS NOT NULL
            AND source_work_item_id IS NOT NULL
            AND decision_stage_execution_id IS NOT NULL
            AND decision_stage_run_unit_id IS NOT NULL
            AND decision_deliverable_submission_id IS NOT NULL
            AND decision_stage_kind = 'attack_candidate'
            AND BTRIM(COALESCE(target_type_at_time, '')) <> ''
            AND BTRIM(COALESCE(target_value_at_time, '')) <> ''
            AND BTRIM(COALESCE(target_identity_hash, '')) <> ''
            AND execution_plan IS NOT NULL
            AND jsonb_typeof(execution_plan) = 'object'
            AND BTRIM(COALESCE(candidate_plan_hash, '')) <> ''
            AND risk_class IN ('deterministic_safe', 'active_safe', 'exploit')
            AND operation_id = operation_uuid::TEXT
            AND BTRIM(hypothesis) <> ''
            AND OCTET_LENGTH(hypothesis) <= 4096
            AND BTRIM(rationale) <> ''
            AND OCTET_LENGTH(rationale) <= 8192
            AND (technique IS NULL OR (
                BTRIM(technique) <> '' AND OCTET_LENGTH(technique) <= 128
            ))
        )
    ),
    ADD CONSTRAINT attack_candidates_terminal_shape_check CHECK (
        terminal_finding_id IS NULL OR terminal_attempt_id IS NOT NULL
    ),
    ADD CONSTRAINT attack_candidates_v2_identity_unique UNIQUE (
        candidate_id,
        operation_uuid,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        source_work_item_id
    ),
    ADD CONSTRAINT attack_candidates_work_item_identity_unique UNIQUE (
        candidate_id,
        source_work_item_id,
        wave_unit_id,
        operation_uuid,
        scope_snapshot_id,
        organization_id,
        target_identity_hash
    ),
    ADD CONSTRAINT attack_candidates_scope_fk
        FOREIGN KEY (scope_snapshot_id, operation_uuid)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_wave_fk
        FOREIGN KEY (wave_run_id, operation_uuid, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_wave_unit_fk
        FOREIGN KEY (
            wave_unit_id,
            wave_run_id,
            operation_uuid,
            scope_snapshot_id,
            organization_id
        ) REFERENCES attack_wave_units(
            id,
            wave_run_id,
            operation_id,
            scope_snapshot_id,
            organization_id
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_decision_unit_fk
        FOREIGN KEY (
            decision_stage_run_unit_id,
            operation_uuid,
            decision_stage_execution_id,
            organization_id,
            decision_stage_kind
        ) REFERENCES stage_run_units(
            id,
            operation_id,
            stage_execution_id,
            organization_id,
            stage_kind
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_decision_submission_fk
        FOREIGN KEY (
            decision_deliverable_submission_id,
            operation_uuid,
            decision_stage_execution_id,
            decision_stage_run_unit_id,
            organization_id,
            decision_stage_kind
        ) REFERENCES stage_deliverable_submissions(
            id,
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id,
            stage_kind
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_decision_handoff_fk
        FOREIGN KEY (
            decision_deliverable_submission_id,
            operation_uuid,
            decision_stage_execution_id,
            decision_stage_run_unit_id,
            organization_id,
            decision_stage_kind,
            scope_snapshot_id
        ) REFERENCES stage_handoffs(
            deliverable_submission_id,
            operation_id,
            stage_execution_id,
            source_stage_run_unit_id,
            organization_id,
            from_stage_kind,
            scope_snapshot_id
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_work_item_fk
        FOREIGN KEY (
            source_work_item_id,
            wave_unit_id,
            operation_uuid,
            scope_snapshot_id,
            organization_id,
            target_identity_hash,
            target_type_at_time,
            target_value_at_time
        ) REFERENCES attack_candidate_work_items(
            id,
            wave_unit_id,
            operation_id,
            scope_snapshot_id,
            organization_id,
            target_identity_hash,
            target_type_at_time,
            target_value_at_time
        ) ON DELETE RESTRICT;

CREATE TRIGGER attack_candidates_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

ALTER TABLE attack_candidate_work_items
    ADD CONSTRAINT attack_candidate_work_items_candidate_fk
    FOREIGN KEY (
        candidate_id,
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        target_identity_hash
    ) REFERENCES attack_candidates(
        candidate_id,
        source_work_item_id,
        wave_unit_id,
        operation_uuid,
        scope_snapshot_id,
        organization_id,
        target_identity_hash
    ) ON DELETE RESTRICT;

CREATE FUNCTION enforce_attack_candidate_work_item_terminalization()
RETURNS trigger AS $$
DECLARE
    work_decision TEXT;
    work_candidate_id UUID;
BEGIN
    IF TG_TABLE_NAME = 'attack_candidates' THEN
        IF NEW.operation_uuid IS NULL THEN
            RETURN NEW;
        END IF;
        SELECT decision_kind, candidate_id
          INTO work_decision, work_candidate_id
          FROM attack_candidate_work_items
         WHERE id = NEW.source_work_item_id;
        IF work_decision IS DISTINCT FROM 'candidate'
            OR work_candidate_id IS DISTINCT FROM NEW.candidate_id
        THEN
            RAISE EXCEPTION 'V2 Candidate and source work item must terminalize together';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.decision_kind = 'candidate' THEN
        IF NOT EXISTS (
            SELECT 1 FROM attack_candidates AS candidate
             WHERE candidate.candidate_id = NEW.candidate_id
               AND candidate.operation_uuid = NEW.operation_id
               AND candidate.scope_snapshot_id = NEW.scope_snapshot_id
               AND candidate.wave_unit_id = NEW.wave_unit_id
               AND candidate.organization_id = NEW.organization_id
               AND candidate.source_work_item_id = NEW.id
        ) THEN
            RAISE EXCEPTION 'candidate work-item decision requires its exact V2 Candidate';
        END IF;
    ELSIF EXISTS (
        SELECT 1 FROM attack_candidates AS candidate
         WHERE candidate.operation_uuid IS NOT NULL
           AND candidate.source_work_item_id = NEW.id
    ) THEN
        RAISE EXCEPTION 'pending/no-candidate work item cannot own a V2 Candidate';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_candidates_terminalize_source_work_item
AFTER INSERT OR UPDATE OF source_work_item_id, candidate_id
ON attack_candidates
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_candidate_work_item_terminalization();

CREATE CONSTRAINT TRIGGER attack_candidate_work_items_terminalize_candidate
AFTER INSERT OR UPDATE OF decision_kind, candidate_id
ON attack_candidate_work_items
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_candidate_work_item_terminalization();

-- ---------------------------------------------------------------------------
-- Approval, Attempt, foreground action journal, durable review and exploit lane
-- ---------------------------------------------------------------------------

CREATE TABLE attack_candidate_approvals (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    source_work_item_id UUID NOT NULL,
    execution_plan JSONB NOT NULL CHECK (jsonb_typeof(execution_plan) = 'object'),
    allowed_capability_ids TEXT[] NOT NULL,
    allowed_action_kinds TEXT[] NOT NULL,
    budget JSONB NOT NULL CHECK (jsonb_typeof(budget) = 'object'),
    expires_at TIMESTAMPTZ NOT NULL,
    decision_version BIGINT NOT NULL CHECK (decision_version > 0),
    status TEXT NOT NULL CHECK (status IN ('approved', 'rejected', 'revoked', 'expired')),
    decided_by UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    UNIQUE (candidate_id, decision_version),
    UNIQUE (
        id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ),
    FOREIGN KEY (
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        source_work_item_id
    ) REFERENCES attack_candidates(
        candidate_id,
        operation_uuid,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        source_work_item_id
    ) ON DELETE RESTRICT
);

CREATE TRIGGER attack_candidate_approvals_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE UNIQUE INDEX attack_candidate_one_current_approval
    ON attack_candidate_approvals(candidate_id)
    WHERE status = 'approved';

ALTER TABLE stage_worker_runs
    ADD CONSTRAINT stage_worker_runs_id_operation_org_unique
        UNIQUE (id, operation_id, organization_id),
    ADD CONSTRAINT stage_worker_runs_id_lease_unique
        UNIQUE (id, lease_token);

CREATE TABLE candidate_attempts (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    candidate_id UUID NOT NULL,
    approval_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    status TEXT NOT NULL CHECK (
        status IN (
            'queued',
            'running',
            'submitted',
            'verified',
            'refuted',
            'blocked',
            'retryable_failed',
            'abandoned'
        )
    ),
    stage_worker_run_id UUID,
    result_json JSONB,
    result_hash TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (candidate_id, ordinal),
    UNIQUE (
        id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ),
    FOREIGN KEY (
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) REFERENCES attack_candidate_approvals(
        id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) ON DELETE RESTRICT,
    FOREIGN KEY (stage_worker_run_id, operation_id, organization_id)
        REFERENCES stage_worker_runs(id, operation_id, organization_id) ON DELETE RESTRICT,
    CHECK ((result_json IS NULL) = (result_hash IS NULL)),
    CHECK (
        (
            status IN ('verified', 'refuted', 'blocked', 'retryable_failed')
            AND result_json IS NOT NULL
            AND terminal_at IS NOT NULL
        )
        OR
        (status = 'abandoned' AND terminal_at IS NOT NULL)
        OR
        (
            status NOT IN ('verified', 'refuted', 'blocked', 'retryable_failed', 'abandoned')
            AND terminal_at IS NULL
        )
    )
);

CREATE UNIQUE INDEX candidate_attempts_one_live_per_candidate
    ON candidate_attempts(candidate_id)
    WHERE status IN ('queued', 'running', 'submitted');

CREATE FUNCTION enforce_candidate_attempt_authority()
RETURNS trigger AS $$
BEGIN
    IF NEW.status IN ('queued', 'running') AND NOT EXISTS (
        SELECT 1
          FROM attack_candidate_approvals AS approval
         WHERE approval.id = NEW.approval_id
           AND approval.candidate_id = NEW.candidate_id
           AND approval.operation_id = NEW.operation_id
           AND approval.scope_snapshot_id = NEW.scope_snapshot_id
           AND approval.wave_run_id = NEW.wave_run_id
           AND approval.wave_unit_id = NEW.wave_unit_id
           AND approval.organization_id = NEW.organization_id
           AND approval.target_identity_hash = NEW.target_identity_hash
           AND approval.candidate_plan_hash = NEW.candidate_plan_hash
           AND approval.status = 'approved'
           AND approval.expires_at > NOW()
    ) THEN
        RAISE EXCEPTION 'candidate attempt requires a current approved exact plan';
    END IF;

    IF NEW.stage_worker_run_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
          FROM stage_worker_runs AS worker
          JOIN stage_run_units AS unit
            ON unit.id = worker.stage_run_unit_id
           AND unit.operation_id = worker.operation_id
           AND unit.stage_execution_id = worker.stage_execution_id
           AND unit.organization_id = worker.organization_id
         WHERE worker.id = NEW.stage_worker_run_id
           AND worker.operation_id = NEW.operation_id
           AND worker.organization_id = NEW.organization_id
           AND worker.work_item_kind = 'candidate_attempt'
           AND worker.work_item_key = NEW.id::TEXT
           AND worker.specialist = 'candidate_verifier'
           AND unit.scope_snapshot_id = NEW.scope_snapshot_id
           AND unit.stage_kind = 'verification'
    ) THEN
        RAISE EXCEPTION 'candidate attempt worker must be exact verification Candidate WorkerRun';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempts_authority
BEFORE INSERT OR UPDATE OF candidate_id, approval_id, operation_id,
    scope_snapshot_id, wave_run_id, wave_unit_id, organization_id,
    target_identity_hash, candidate_plan_hash, status, stage_worker_run_id
ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_authority();

CREATE TRIGGER candidate_attempts_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE TABLE candidate_attempt_actions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    attempt_id UUID NOT NULL REFERENCES candidate_attempts(id) ON DELETE RESTRICT,
    action_ordinal INTEGER NOT NULL CHECK (action_ordinal >= 0),
    capability_id TEXT NOT NULL CHECK (BTRIM(capability_id) <> ''),
    action_kind TEXT NOT NULL CHECK (BTRIM(action_kind) <> ''),
    canonical_args JSONB NOT NULL CHECK (jsonb_typeof(canonical_args) = 'object'),
    status TEXT NOT NULL CHECK (
        status IN ('planned', 'started', 'completed', 'failed', 'outcome_unknown')
    ),
    outcome JSONB,
    outcome_hash TEXT,
    error_code TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (attempt_id, action_ordinal),
    CHECK ((outcome IS NULL) = (outcome_hash IS NULL)),
    CHECK (completed_at IS NULL OR started_at IS NOT NULL)
);

CREATE TABLE candidate_review_barriers (
    wave_run_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'resume_pending', 'dispatching', 'resumed', 'terminal')),
    resume_version BIGINT NOT NULL DEFAULT 0 CHECK (resume_version >= 0),
    last_error TEXT,
    dispatch_started_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (wave_run_id, operation_id, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id) ON DELETE RESTRICT,
    CHECK (status <> 'dispatching' OR dispatch_started_at IS NOT NULL)
);

CREATE TABLE attack_execution_lanes (
    lane_key TEXT PRIMARY KEY CHECK (BTRIM(lane_key) <> ''),
    stage_worker_run_id UUID,
    lease_token UUID,
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (stage_worker_run_id, lease_token)
        REFERENCES stage_worker_runs(id, lease_token) ON DELETE RESTRICT,
    CHECK (
        (
            stage_worker_run_id IS NULL
            AND lease_token IS NULL
            AND lease_owner IS NULL
            AND lease_expires_at IS NULL
        )
        OR
        (
            stage_worker_run_id IS NOT NULL
            AND lease_token IS NOT NULL
            AND BTRIM(COALESCE(lease_owner, '')) <> ''
            AND lease_expires_at IS NOT NULL
        )
    )
);

INSERT INTO attack_execution_lanes(lane_key) VALUES ('global:exploit');

-- ---------------------------------------------------------------------------
-- Finding lineage, FactDelta and reportable residual risk
-- ---------------------------------------------------------------------------

CREATE TABLE finding_lineage (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    finding_id UUID NOT NULL UNIQUE REFERENCES findings(id) ON DELETE RESTRICT,
    candidate_attempt_id UUID NOT NULL UNIQUE,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (
        finding_id,
        candidate_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ),
    FOREIGN KEY (
        candidate_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) REFERENCES candidate_attempts(
        id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_verified_finding_lineage()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM candidate_attempts AS attempt
         WHERE attempt.id = NEW.candidate_attempt_id
           AND attempt.candidate_id = NEW.candidate_id
           AND attempt.operation_id = NEW.operation_id
           AND attempt.scope_snapshot_id = NEW.scope_snapshot_id
           AND attempt.wave_run_id = NEW.wave_run_id
           AND attempt.wave_unit_id = NEW.wave_unit_id
           AND attempt.organization_id = NEW.organization_id
           AND attempt.target_identity_hash = NEW.target_identity_hash
           AND attempt.candidate_plan_hash = NEW.candidate_plan_hash
           AND attempt.status = 'verified'
           AND attempt.terminal_at IS NOT NULL
    ) OR NOT EXISTS (
        SELECT 1
          FROM findings AS finding
          JOIN operation_state AS operation
            ON operation.operation_id = NEW.operation_id
          JOIN project_scopes AS project_scope
            ON project_scope.project_scope_id = operation.project_scope_id
         WHERE finding.id = NEW.finding_id
           AND finding.target_id IS NOT DISTINCT FROM NEW.target_live_id
           AND finding.source = 'candidate_v2'
           AND finding.project_path = project_scope.canonical_project_path
           AND finding.evidence = COALESCE(
               (
                   SELECT jsonb_agg(link.evidence_id ORDER BY link.evidence_id)
                     FROM candidate_attempt_evidence AS link
                    WHERE link.attempt_id = NEW.candidate_attempt_id
                      AND link.role = 'proof'
               ),
               '[]'::JSONB
           )
    ) THEN
        RAISE EXCEPTION 'finding lineage requires exact verified Attempt and Candidate V2 Finding';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER finding_lineage_requires_verified_attempt
BEFORE INSERT OR UPDATE OF finding_id, candidate_attempt_id, candidate_id,
    operation_id, scope_snapshot_id, wave_run_id, wave_unit_id,
    organization_id, target_live_id, target_identity_hash, candidate_plan_hash
ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION enforce_verified_finding_lineage();

CREATE TRIGGER finding_lineage_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE FUNCTION protect_lineage_bound_finding()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM finding_lineage AS lineage WHERE lineage.finding_id = OLD.id
    ) THEN
        -- `findings.target_id` is only a nullable pointer to the live target;
        -- the immutable lineage row retains the frozen target identity. Preserve
        -- the existing ON DELETE SET NULL contract, but only for the nested FK
        -- trigger and only when no other Finding field changes. A direct client
        -- UPDATE still runs at trigger depth 1 and remains forbidden.
        IF TG_OP = 'UPDATE'
            AND pg_trigger_depth() > 1
            AND OLD.target_id IS NOT NULL
            AND NEW.target_id IS NULL
            AND (to_jsonb(NEW) - 'target_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_id')
        THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'lineage-bound Candidate Finding is immutable';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER findings_lineage_proof_immutable
BEFORE UPDATE OR DELETE ON findings
FOR EACH ROW EXECUTE FUNCTION protect_lineage_bound_finding();

ALTER TABLE attack_candidates
    ADD CONSTRAINT attack_candidates_terminal_attempt_fk
        FOREIGN KEY (
            terminal_attempt_id,
            candidate_id,
            operation_uuid,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            target_identity_hash,
            candidate_plan_hash
        ) REFERENCES candidate_attempts(
            id,
            candidate_id,
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            target_identity_hash,
            candidate_plan_hash
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_candidates_terminal_finding_fk
        FOREIGN KEY (
            terminal_finding_id,
            terminal_attempt_id,
            candidate_id,
            operation_uuid,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            target_identity_hash,
            candidate_plan_hash
        ) REFERENCES finding_lineage(
            finding_id,
            candidate_attempt_id,
            candidate_id,
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            target_identity_hash,
            candidate_plan_hash
        ) ON DELETE RESTRICT;

CREATE TABLE attack_fact_deltas (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    source_attempt_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time) <> ''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time) <> ''),
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    canonical_ref_kind TEXT NOT NULL CHECK (BTRIM(canonical_ref_kind) <> ''),
    canonical_ref_id UUID NOT NULL,
    canonical_ref_version BIGINT NOT NULL CHECK (canonical_ref_version >= 0),
    canonical_ref_hash TEXT NOT NULL CHECK (BTRIM(canonical_ref_hash) <> ''),
    delta_kind TEXT NOT NULL CHECK (BTRIM(delta_kind) <> ''),
    dedupe_hash TEXT NOT NULL CHECK (BTRIM(dedupe_hash) <> ''),
    status TEXT NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed', 'accepted', 'consumed', 'rejected')),
    consumed_by_wave_run_id UUID REFERENCES attack_wave_runs(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at TIMESTAMPTZ,
    UNIQUE (operation_id, organization_id, dedupe_hash),
    FOREIGN KEY (
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) REFERENCES candidate_attempts(
        id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash
    ) ON DELETE RESTRICT,
    CHECK (
        (status = 'consumed' AND consumed_by_wave_run_id IS NOT NULL AND consumed_at IS NOT NULL)
        OR (status <> 'consumed' AND consumed_by_wave_run_id IS NULL AND consumed_at IS NULL)
    )
);

CREATE FUNCTION enforce_fact_delta_consumed_wave_owner()
RETURNS trigger AS $$
BEGIN
    IF NEW.consumed_by_wave_run_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
          FROM attack_wave_runs AS consumed_wave
         WHERE consumed_wave.id = NEW.consumed_by_wave_run_id
           AND consumed_wave.operation_id = NEW.operation_id
           AND consumed_wave.scope_snapshot_id = NEW.scope_snapshot_id
    ) THEN
        RAISE EXCEPTION 'FactDelta consumed wave must share operation and frozen scope';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_deltas_consumed_wave_owner
BEFORE INSERT OR UPDATE OF consumed_by_wave_run_id, operation_id, scope_snapshot_id
ON attack_fact_deltas
FOR EACH ROW EXECUTE FUNCTION enforce_fact_delta_consumed_wave_owner();

CREATE TRIGGER attack_fact_deltas_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_fact_deltas
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

CREATE TABLE attack_residual_risks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT,
    target_value_at_time TEXT,
    target_identity_hash TEXT,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code) <> ''),
    reason_detail TEXT NOT NULL DEFAULT '',
    policy_hash TEXT NOT NULL CHECK (BTRIM(policy_hash) <> ''),
    wave_count INTEGER NOT NULL CHECK (wave_count >= 0),
    candidate_count INTEGER NOT NULL CHECK (candidate_count >= 0),
    chain_depth INTEGER NOT NULL CHECK (chain_depth >= 0),
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
    disclosure_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (disclosure_status IN ('pending', 'reported', 'acknowledged')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    disclosed_at TIMESTAMPTZ,
    FOREIGN KEY (wave_unit_id, wave_run_id, operation_id, scope_snapshot_id, organization_id)
        REFERENCES attack_wave_units(
            id,
            wave_run_id,
            operation_id,
            scope_snapshot_id,
            organization_id
        ) ON DELETE RESTRICT,
    CHECK (
        (
            target_identity_hash IS NULL
            AND target_type_at_time IS NULL
            AND target_value_at_time IS NULL
            AND target_live_id IS NULL
        )
        OR
        (
            BTRIM(COALESCE(target_identity_hash, '')) <> ''
            AND BTRIM(COALESCE(target_type_at_time, '')) <> ''
            AND BTRIM(COALESCE(target_value_at_time, '')) <> ''
        )
    ),
    CHECK (
        (disclosure_status = 'pending' AND disclosed_at IS NULL)
        OR (disclosure_status <> 'pending' AND disclosed_at IS NOT NULL)
    )
);

CREATE TRIGGER attack_residual_risks_target_owner
BEFORE INSERT OR UPDATE OF target_live_id, organization_id
ON attack_residual_risks
FOR EACH ROW EXECUTE FUNCTION validate_attack_live_target_owner();

-- ---------------------------------------------------------------------------
-- Evidence links: relational roles plus DB-enforced owner/run/org/target proof
-- ---------------------------------------------------------------------------

CREATE TABLE attack_candidate_seed_evidence (
    seed_id UUID NOT NULL REFERENCES attack_candidate_seeds(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('observation', 'support')),
    PRIMARY KEY (seed_id, evidence_id, role)
);

CREATE TABLE attack_candidate_work_item_evidence (
    work_item_id UUID NOT NULL REFERENCES attack_candidate_work_items(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('observation', 'support', 'decision')),
    PRIMARY KEY (work_item_id, evidence_id, role)
);

CREATE FUNCTION reject_frozen_attack_manifest_evidence_change()
RETURNS trigger AS $$
DECLARE
    owner_wave_unit_id UUID;
    manifest_role BOOLEAN;
BEGIN
    IF TG_TABLE_NAME = 'attack_candidate_seed_evidence' THEN
        SELECT wave_unit_id INTO owner_wave_unit_id
          FROM attack_candidate_seeds
         WHERE id = CASE WHEN TG_OP = 'DELETE' THEN OLD.seed_id ELSE NEW.seed_id END;
        manifest_role := TRUE;
    ELSE
        SELECT wave_unit_id INTO owner_wave_unit_id
          FROM attack_candidate_work_items
         WHERE id = CASE WHEN TG_OP = 'DELETE' THEN OLD.work_item_id ELSE NEW.work_item_id END;
        manifest_role := CASE
            WHEN TG_OP = 'INSERT' THEN NEW.role IN ('observation', 'support')
            WHEN TG_OP = 'DELETE' THEN OLD.role IN ('observation', 'support')
            ELSE OLD.role IN ('observation', 'support') OR NEW.role IN ('observation', 'support')
        END;
    END IF;
    IF manifest_role AND EXISTS (
        SELECT 1 FROM attack_wave_units
         WHERE id = owner_wave_unit_id AND manifest_frozen_at IS NOT NULL
    ) THEN
        IF TG_OP = 'INSERT' THEN
            IF TG_TABLE_NAME = 'attack_candidate_seed_evidence' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_seed_evidence
                     WHERE seed_id=NEW.seed_id AND evidence_id=NEW.evidence_id AND role=NEW.role
                ) THEN
                    RETURN NEW;
                END IF;
            ELSIF TG_TABLE_NAME = 'attack_candidate_work_item_evidence' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_work_item_evidence
                     WHERE work_item_id=NEW.work_item_id
                       AND evidence_id=NEW.evidence_id AND role=NEW.role
                ) THEN
                    RETURN NEW;
                END IF;
            END IF;
        END IF;
        RAISE EXCEPTION 'frozen attack manifest evidence membership is immutable';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_seed_evidence_manifest_frozen
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_seed_evidence
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_evidence_change();

CREATE TRIGGER attack_candidate_work_item_evidence_manifest_frozen
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_work_item_evidence
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_evidence_change();

CREATE TABLE attack_candidate_evidence (
    candidate_id UUID NOT NULL REFERENCES attack_candidates(candidate_id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('support', 'rationale')),
    PRIMARY KEY (candidate_id, evidence_id, role)
);

CREATE TABLE candidate_attempt_evidence (
    attempt_id UUID NOT NULL REFERENCES candidate_attempts(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('proof', 'refutation', 'blocker', 'fact_delta')),
    PRIMARY KEY (attempt_id, evidence_id, role)
);

CREATE FUNCTION protect_candidate_attempt_evidence_membership()
RETURNS trigger AS $$
DECLARE
    attempt_status TEXT;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        SELECT status INTO attempt_status
          FROM candidate_attempts
         WHERE id = OLD.attempt_id
         FOR UPDATE;
        IF attempt_status IS DISTINCT FROM 'running' THEN
            RAISE EXCEPTION 'submitted or terminal Candidate Attempt evidence is immutable';
        END IF;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        SELECT status INTO attempt_status
          FROM candidate_attempts
         WHERE id = NEW.attempt_id
         FOR UPDATE;
        IF attempt_status IS DISTINCT FROM 'running' THEN
            RAISE EXCEPTION 'submitted or terminal Candidate Attempt evidence is immutable';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempt_evidence_membership_immutable
BEFORE INSERT OR UPDATE OR DELETE ON candidate_attempt_evidence
FOR EACH ROW EXECUTE FUNCTION protect_candidate_attempt_evidence_membership();

CREATE TABLE attack_fact_delta_evidence (
    fact_delta_id UUID NOT NULL REFERENCES attack_fact_deltas(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role = 'fact_delta'),
    PRIMARY KEY (fact_delta_id, evidence_id, role)
);

CREATE TABLE attack_residual_risk_evidence (
    residual_risk_id UUID NOT NULL REFERENCES attack_residual_risks(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role = 'residual'),
    PRIMARY KEY (residual_risk_id, evidence_id, role)
);

CREATE FUNCTION enforce_attack_evidence_owner()
RETURNS trigger AS $$
DECLARE
    owner_id UUID;
    owner_operation_id UUID;
    owner_organization_id UUID;
    owner_target_live_id UUID;
    evidence_role TEXT;
    evidence_run_id UUID;
    evidence_organization_id UUID;
    evidence_target_id UUID;
BEGIN
    owner_id := (to_jsonb(NEW) ->> TG_ARGV[3])::UUID;
    EXECUTE format(
        'SELECT %I, organization_id, target_live_id FROM %I WHERE %I = $1',
        TG_ARGV[2],
        TG_ARGV[0],
        TG_ARGV[1]
    )
    INTO owner_operation_id, owner_organization_id, owner_target_live_id
    USING owner_id;
    IF owner_operation_id IS NULL THEN
        RAISE EXCEPTION 'attack evidence owner row is missing';
    END IF;

    SELECT
        audit_role,
        run_id,
        NULLIF(detail ->> 'organization_id', '')::UUID,
        target_id
    INTO
        evidence_role,
        evidence_run_id,
        evidence_organization_id,
        evidence_target_id
    FROM audit_log
    WHERE id = NEW.evidence_id;

    IF NOT FOUND
        OR evidence_role IS DISTINCT FROM 'evidence'
        OR evidence_run_id IS DISTINCT FROM owner_operation_id
        OR evidence_organization_id IS DISTINCT FROM owner_organization_id
        OR (
            evidence_target_id IS NOT NULL
            AND evidence_target_id IS DISTINCT FROM owner_target_live_id
        )
    THEN
        RAISE EXCEPTION 'audit evidence does not match attack owner operation, organization, or target';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_candidate_seed_evidence_owner
AFTER INSERT OR UPDATE ON attack_candidate_seed_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'attack_candidate_seeds', 'id', 'operation_id', 'seed_id'
);

CREATE CONSTRAINT TRIGGER attack_candidate_work_item_evidence_owner
AFTER INSERT OR UPDATE ON attack_candidate_work_item_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'attack_candidate_work_items', 'id', 'operation_id', 'work_item_id'
);

CREATE CONSTRAINT TRIGGER attack_candidate_evidence_owner
AFTER INSERT OR UPDATE ON attack_candidate_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'attack_candidates', 'candidate_id', 'operation_uuid', 'candidate_id'
);

CREATE CONSTRAINT TRIGGER candidate_attempt_evidence_owner
AFTER INSERT OR UPDATE ON candidate_attempt_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'candidate_attempts', 'id', 'operation_id', 'attempt_id'
);

CREATE CONSTRAINT TRIGGER attack_fact_delta_evidence_owner
AFTER INSERT OR UPDATE ON attack_fact_delta_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'attack_fact_deltas', 'id', 'operation_id', 'fact_delta_id'
);

CREATE CONSTRAINT TRIGGER attack_residual_risk_evidence_owner
AFTER INSERT OR UPDATE ON attack_residual_risk_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'attack_residual_risks', 'id', 'operation_id', 'residual_risk_id'
);

CREATE INDEX attack_candidate_seeds_owner_idx
    ON attack_candidate_seeds(operation_id, organization_id, wave_unit_id);
CREATE INDEX attack_candidate_work_items_owner_idx
    ON attack_candidate_work_items(operation_id, organization_id, wave_unit_id, decision_kind);
CREATE INDEX attack_candidate_approvals_review_idx
    ON attack_candidate_approvals(operation_id, wave_run_id, organization_id, status);
CREATE INDEX candidate_attempts_queue_idx
    ON candidate_attempts(operation_id, status, created_at);
CREATE INDEX attack_fact_deltas_status_idx
    ON attack_fact_deltas(operation_id, status, created_at);
CREATE INDEX attack_residual_risks_disclosure_idx
    ON attack_residual_risks(operation_id, disclosure_status, created_at);
