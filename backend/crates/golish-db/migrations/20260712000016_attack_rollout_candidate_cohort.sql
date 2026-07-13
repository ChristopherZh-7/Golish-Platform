-- Candidate admission cohort and database-owned attack rollout promotion gate.
--
-- A generation-zero WaveUnit is the first point at which an operation commits
-- to producing Candidate semantics.  That admission is serialized against the
-- rollout singleton and retained with a monotonic sequence.  Promotions freeze
-- the current sequence as a cutoff, then prove every materialized non-zero
-- WaveUnit has a sealed positive Candidate manifest, exactly one final-passed
-- Candidate Unit and an exact shadow sample.  Exact terminal follow-on
-- zero-input WaveUnits are excluded from that Candidate-domain denominator.

ALTER TABLE operation_state
    ADD CONSTRAINT operation_attack_v2_writer_requires_runtime_v2_writer
    CHECK (
        attack_execution_contract = 'legacy'
        OR runtime_memory_contract <> 'legacy_v1'
    );

CREATE TABLE attack_execution_candidate_admissions (
    admission_seq BIGSERIAL PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE,
    scope_snapshot_id UUID NOT NULL,
    initial_wave_run_id UUID NOT NULL UNIQUE,
    first_wave_unit_id UUID NOT NULL UNIQUE,
    first_organization_id UUID NOT NULL,
    attack_execution_contract TEXT NOT NULL CHECK (
        attack_execution_contract IN (
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback'
        )
    ),
    rollout_rank SMALLINT NOT NULL CHECK (rollout_rank IN (1, 2)),
    rollout_row_version BIGINT NOT NULL CHECK (rollout_row_version >= 0),
    admitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (admission_seq, attack_execution_contract, rollout_rank),
    FOREIGN KEY (operation_id, attack_execution_contract)
        REFERENCES operation_state(operation_id, attack_execution_contract)
        ON DELETE RESTRICT,
    FOREIGN KEY (initial_wave_run_id, operation_id, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id)
        ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        first_wave_unit_id,
        initial_wave_run_id,
        operation_id,
        scope_snapshot_id,
        first_organization_id
    ) REFERENCES attack_wave_units(
        id,
        wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT
      DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        rollout_rank = CASE attack_execution_contract
            WHEN 'dual_write_read_legacy' THEN 1
            WHEN 'dual_write_read_v2_fallback' THEN 2
        END
    )
);

CREATE INDEX attack_execution_candidate_admissions_cohort_idx
    ON attack_execution_candidate_admissions(
        attack_execution_contract,
        rollout_rank,
        admission_seq
    );

CREATE TABLE attack_execution_rollout_promotions (
    from_rank SMALLINT PRIMARY KEY CHECK (from_rank IN (1, 2)),
    to_rank SMALLINT NOT NULL CHECK (to_rank = from_rank + 1),
    from_contract TEXT NOT NULL,
    to_contract TEXT NOT NULL,
    from_row_version BIGINT NOT NULL CHECK (from_row_version >= 0),
    to_row_version BIGINT NOT NULL CHECK (to_row_version = from_row_version + 1),
    admission_cutoff BIGINT NOT NULL CHECK (admission_cutoff > 0),
    admission_count BIGINT NOT NULL CHECK (admission_count > 0),
    candidate_unit_count BIGINT NOT NULL CHECK (candidate_unit_count > 0),
    sample_count BIGINT NOT NULL CHECK (sample_count = candidate_unit_count),
    promoted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (from_rank = 1
            AND from_contract = 'dual_write_read_legacy'
            AND to_contract = 'dual_write_read_v2_fallback')
        OR
        (from_rank = 2
            AND from_contract = 'dual_write_read_v2_fallback'
            AND to_contract = 'v2_only')
    )
);

CREATE FUNCTION reject_direct_attack_execution_candidate_admission()
RETURNS trigger AS $$
DECLARE
    owner_operation_id UUID;
    owner_scope_snapshot_id UUID;
    owner_wave_run_id UUID;
    owner_wave_unit_id UUID;
    owner_organization_id UUID;
    owner_generation INTEGER;
    operation_contract TEXT;
    current_contract TEXT;
    current_rank SMALLINT;
    current_row_version BIGINT;
BEGIN
    -- UPDATE/DELETE are never lifecycle operations.  A direct INSERT is also
    -- forbidden; a nested INSERT is treated only as an owner-row reference and
    -- every retained field is rebuilt below from database authority.
    IF TG_OP IN ('UPDATE', 'DELETE') OR pg_trigger_depth() = 1 THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_INTERNAL_ONLY'
            USING ERRCODE = '42501';
    END IF;

    SELECT wave_unit.operation_id,
           wave_unit.scope_snapshot_id,
           wave_unit.wave_run_id,
           wave_unit.id,
           wave_unit.organization_id,
           wave.generation,
           operation.attack_execution_contract,
           rollout.contract,
           rollout.rank,
           rollout.row_version
      INTO owner_operation_id,
           owner_scope_snapshot_id,
           owner_wave_run_id,
           owner_wave_unit_id,
           owner_organization_id,
           owner_generation,
           operation_contract,
           current_contract,
           current_rank,
           current_row_version
      FROM attack_wave_units AS wave_unit
      JOIN attack_wave_runs AS wave
        ON wave.id = wave_unit.wave_run_id
       AND wave.operation_id = wave_unit.operation_id
       AND wave.scope_snapshot_id = wave_unit.scope_snapshot_id
      JOIN operation_state AS operation
        ON operation.operation_id = wave_unit.operation_id
       AND operation.superseded_by IS NULL
      JOIN attack_execution_rollout AS rollout
        ON rollout.singleton = TRUE
     WHERE wave_unit.id = NEW.first_wave_unit_id
     FOR SHARE OF wave_unit, wave, operation, rollout;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_OWNER_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF owner_generation <> 0 THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_INITIAL_UNIT_REQUIRED'
            USING ERRCODE = '23514';
    END IF;
    IF operation_contract IS DISTINCT FROM current_contract
       OR current_rank NOT IN (1, 2)
       OR operation_contract NOT IN (
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback'
       ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_STALE_CONTRACT'
            USING ERRCODE = '55000';
    END IF;

    NEW.admission_seq := nextval(
        pg_get_serial_sequence(
            'attack_execution_candidate_admissions',
            'admission_seq'
        )::regclass
    );
    NEW.operation_id := owner_operation_id;
    NEW.scope_snapshot_id := owner_scope_snapshot_id;
    NEW.initial_wave_run_id := owner_wave_run_id;
    NEW.first_wave_unit_id := owner_wave_unit_id;
    NEW.first_organization_id := owner_organization_id;
    NEW.attack_execution_contract := operation_contract;
    NEW.rollout_rank := current_rank;
    NEW.rollout_row_version := current_row_version;
    NEW.admitted_at := NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_candidate_admission_internal_only
BEFORE INSERT OR UPDATE OR DELETE ON attack_execution_candidate_admissions
FOR EACH ROW EXECUTE FUNCTION reject_direct_attack_execution_candidate_admission();

CREATE FUNCTION reject_direct_attack_execution_rollout_promotion_receipt()
RETURNS trigger AS $$
DECLARE
    rollout attack_execution_rollout%ROWTYPE;
    derived_from_rank SMALLINT;
    derived_from_contract TEXT;
    current_cutoff BIGINT;
    gate_admission_count BIGINT;
    gate_candidate_unit_count BIGINT;
    gate_sample_count BIGINT;
    gate_ready BOOLEAN;
    gate_reason TEXT;
BEGIN
    IF TG_OP IN ('UPDATE', 'DELETE') OR pg_trigger_depth() = 1 THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_PROMOTION_RECEIPT_INTERNAL_ONLY'
            USING ERRCODE = '42501';
    END IF;

    SELECT current_rollout.*
      INTO rollout
      FROM attack_execution_rollout AS current_rollout
     WHERE current_rollout.singleton = TRUE
     FOR SHARE;
    IF NOT FOUND OR rollout.rank NOT IN (2, 3) THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_PROMOTION_RECEIPT_STATE_MISMATCH'
            USING ERRCODE = '55000';
    END IF;

    derived_from_rank := rollout.rank - 1;
    derived_from_contract := CASE derived_from_rank
        WHEN 1 THEN 'dual_write_read_legacy'
        WHEN 2 THEN 'dual_write_read_v2_fallback'
    END;
    IF rollout.contract IS DISTINCT FROM (CASE rollout.rank
            WHEN 2 THEN 'dual_write_read_v2_fallback'
            WHEN 3 THEN 'v2_only'
       END) THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_PROMOTION_RECEIPT_STATE_MISMATCH'
            USING ERRCODE = '55000';
    END IF;

    SELECT MAX(admission.admission_seq)
      INTO current_cutoff
      FROM attack_execution_candidate_admissions AS admission
     WHERE admission.attack_execution_contract = derived_from_contract
       AND admission.rollout_rank = derived_from_rank;
    IF current_cutoff IS NULL THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_PROMOTION_RECEIPT_RECOMPUTE_FAILED: candidate_cohort_empty'
            USING ERRCODE = '55000';
    END IF;
    SELECT gate.admission_count,
           gate.candidate_unit_count,
           gate.sample_count,
           gate.ready,
           gate.reason
      INTO gate_admission_count,
           gate_candidate_unit_count,
           gate_sample_count,
           gate_ready,
           gate_reason
      FROM attack_execution_candidate_cohort_gate(
          derived_from_contract,
          derived_from_rank,
          current_cutoff
      ) AS gate;
    IF NOT COALESCE(gate_ready, FALSE) THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_PROMOTION_RECEIPT_RECOMPUTE_FAILED: %',
            COALESCE(gate_reason, 'missing_gate_result')
            USING ERRCODE = '55000';
    END IF;

    -- Every field, including the key, is rebuilt after the singleton has
    -- advanced, so nested trigger depth is never treated as owner authority.
    NEW.from_rank := derived_from_rank;
    NEW.to_rank := rollout.rank;
    NEW.from_contract := derived_from_contract;
    NEW.to_contract := rollout.contract;
    NEW.from_row_version := rollout.row_version - 1;
    NEW.to_row_version := rollout.row_version;
    NEW.admission_cutoff := current_cutoff;
    NEW.admission_count := gate_admission_count;
    NEW.candidate_unit_count := gate_candidate_unit_count;
    NEW.sample_count := gate_sample_count;
    NEW.promoted_at := NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_rollout_promotion_receipt_internal_only
BEFORE INSERT OR UPDATE OR DELETE ON attack_execution_rollout_promotions
FOR EACH ROW EXECUTE FUNCTION reject_direct_attack_execution_rollout_promotion_receipt();

-- This function is installed as the alphabetically first BEFORE INSERT trigger
-- on both Wave tables.  The lock order is therefore rollout -> operation/Wave
-- (the latter is acquired by the fuel trigger from migration 00015).
CREATE FUNCTION admit_attack_execution_candidate_operation()
RETURNS trigger AS $$
DECLARE
    wave_generation INTEGER;
    owner_operation_id UUID;
    owner_scope_snapshot_id UUID;
    owner_wave_run_id UUID;
    owner_wave_unit_id UUID;
    owner_organization_id UUID;
    operation_contract TEXT;
    current_contract TEXT;
    current_rank SMALLINT;
    current_row_version BIGINT;
    admitted attack_execution_candidate_admissions%ROWTYPE;
BEGIN
    IF TG_TABLE_NAME = 'attack_wave_runs' THEN
        wave_generation := NEW.generation;
        owner_operation_id := NEW.operation_id;
        owner_scope_snapshot_id := NEW.scope_snapshot_id;
        owner_wave_run_id := NEW.id;
    ELSIF TG_TABLE_NAME = 'attack_wave_units' THEN
        owner_operation_id := NEW.operation_id;
        owner_scope_snapshot_id := NEW.scope_snapshot_id;
        owner_wave_run_id := NEW.wave_run_id;
        owner_wave_unit_id := NEW.id;
        owner_organization_id := NEW.organization_id;
        SELECT wave.generation
          INTO wave_generation
          FROM attack_wave_runs AS wave
         WHERE wave.id = NEW.wave_run_id
           AND wave.operation_id = NEW.operation_id
           AND wave.scope_snapshot_id = NEW.scope_snapshot_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_WAVE_MISSING'
                USING ERRCODE = '23503';
        END IF;
    ELSE
        RAISE EXCEPTION 'unsupported Candidate admission table %', TG_TABLE_NAME;
    END IF;

    SELECT rollout.contract, rollout.rank, rollout.row_version
      INTO current_contract, current_rank, current_row_version
      FROM attack_execution_rollout AS rollout
     WHERE rollout.singleton = TRUE
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_ROLLOUT_SINGLETON_MISSING'
            USING ERRCODE = '55000';
    END IF;

    SELECT operation.attack_execution_contract
      INTO operation_contract
      FROM operation_state AS operation
     WHERE operation.operation_id = owner_operation_id
       AND operation.superseded_by IS NULL
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_OPERATION_MISSING'
            USING ERRCODE = '23503';
    END IF;

    SELECT admission.*
      INTO admitted
      FROM attack_execution_candidate_admissions AS admission
     WHERE admission.operation_id = owner_operation_id
     FOR SHARE;
    IF FOUND THEN
        IF wave_generation = 0
           AND admitted.initial_wave_run_id IS DISTINCT FROM owner_wave_run_id THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_INITIAL_WAVE_DRIFT'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    -- V2-only operations no longer participate in a dual-read promotion
    -- cohort, but retain the same rollout serialization for late Wave races.
    IF operation_contract = 'v2_only'
       AND current_contract = 'v2_only'
       AND current_rank = 3 THEN
        RETURN NEW;
    END IF;

    IF wave_generation <> 0 THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_FOLLOW_ON_REQUIRES_ADMISSION'
            USING ERRCODE = '55000';
    END IF;
    IF operation_contract IS DISTINCT FROM current_contract
       OR current_rank NOT IN (1, 2)
       OR operation_contract NOT IN (
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback'
       ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_STALE_CONTRACT'
            USING ERRCODE = '55000';
    END IF;

    -- BEFORE insertion establishes rollout -> operation/Wave lock order and
    -- validates stale/follow-on operations.  The persisted generation-zero
    -- WaveUnit is admitted by a separate AFTER trigger so the admission table
    -- can rebuild every retained field from an existing owner row.
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_attack_execution_candidate_wave_admission
BEFORE INSERT ON attack_wave_runs
FOR EACH ROW EXECUTE FUNCTION admit_attack_execution_candidate_operation();

CREATE TRIGGER aa_attack_execution_candidate_unit_validation
BEFORE INSERT ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION admit_attack_execution_candidate_operation();

CREATE FUNCTION record_attack_execution_candidate_admission()
RETURNS trigger AS $$
DECLARE
    wave_generation INTEGER;
    operation_contract TEXT;
    current_contract TEXT;
    current_rank SMALLINT;
BEGIN
    SELECT wave.generation,
           operation.attack_execution_contract,
           rollout.contract,
           rollout.rank
      INTO wave_generation,
           operation_contract,
           current_contract,
           current_rank
      FROM attack_wave_runs AS wave
      JOIN operation_state AS operation
        ON operation.operation_id = NEW.operation_id
       AND operation.superseded_by IS NULL
      JOIN attack_execution_rollout AS rollout
        ON rollout.singleton = TRUE
     WHERE wave.id = NEW.wave_run_id
       AND wave.operation_id = NEW.operation_id
       AND wave.scope_snapshot_id = NEW.scope_snapshot_id
     FOR SHARE OF wave, operation, rollout;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_OWNER_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF wave_generation = 0
       AND operation_contract = 'v2_only'
       AND current_contract = 'v2_only'
       AND current_rank = 3 THEN
        RETURN NEW;
    END IF;
    IF wave_generation = 0
       AND NOT EXISTS (
            SELECT 1
              FROM attack_execution_candidate_admissions AS admission
             WHERE admission.operation_id = NEW.operation_id
       ) THEN
        -- first_wave_unit_id is the sole semantic input.  The admission-table
        -- BEFORE trigger reconstructs sequence, owner tuple, frozen contract,
        -- rollout identity and chronology from database authority.
        -- Supplying a placeholder avoids consuming the BIGSERIAL default before
        -- the prepare trigger allocates the sole database-owned sequence value.
        INSERT INTO attack_execution_candidate_admissions(
            admission_seq,
            first_wave_unit_id
        )
        VALUES (0, NEW.id)
        ON CONFLICT (operation_id) DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_attack_execution_candidate_unit_admission
AFTER INSERT ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION record_attack_execution_candidate_admission();

-- A durable operation admission freezes that operation's writer contract, not
-- the deployment default forever.  Existing admitted operations may therefore
-- create later Candidate Units after the default advances; a never-admitted
-- stale operation must still be rejected on its first Candidate Unit.
CREATE FUNCTION reject_late_attack_candidate_stage_unit()
RETURNS trigger AS $$
DECLARE
    current_rank SMALLINT;
    current_contract TEXT;
    admitted_contract TEXT;
    operation_contract TEXT;
BEGIN
    IF NEW.stage_kind <> 'attack_candidate' THEN
        RETURN NEW;
    END IF;
    SELECT rollout.rank, rollout.contract
      INTO current_rank, current_contract
      FROM attack_execution_rollout AS rollout
     WHERE rollout.singleton = TRUE
     FOR SHARE;
    SELECT admission.attack_execution_contract
      INTO admitted_contract
      FROM attack_execution_candidate_admissions AS admission
     WHERE admission.operation_id = NEW.operation_id
     FOR SHARE;
    IF FOUND THEN
        SELECT operation.attack_execution_contract
          INTO operation_contract
          FROM operation_state AS operation
         WHERE operation.operation_id = NEW.operation_id
           AND operation.superseded_by IS NULL;
        IF operation_contract IS DISTINCT FROM admitted_contract THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_ADMISSION_CONTRACT_DRIFT'
                USING ERRCODE = '23514';
        END IF;
        -- Deployment defaults may advance while an immutable old-contract
        -- operation continues into a later Wave.  Its durable admission, not
        -- the current default rank, remains the authority for new Units.
        RETURN NEW;
    END IF;
    SELECT operation.attack_execution_contract
      INTO operation_contract
      FROM operation_state AS operation
     WHERE operation.operation_id = NEW.operation_id;
    IF NOT (
        operation_contract = current_contract
        AND (
            (current_rank IN (1, 2) AND operation_contract IN (
                'dual_write_read_legacy',
                'dual_write_read_v2_fallback'
            ))
            OR (current_rank = 3 AND operation_contract = 'v2_only')
        )
    ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_STAGE_UNIT_REQUIRES_ADMISSION'
            USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_attack_candidate_stage_unit_cutoff
BEFORE INSERT ON stage_run_units
FOR EACH ROW EXECUTE FUNCTION reject_late_attack_candidate_stage_unit();

CREATE FUNCTION reject_reopen_of_attack_rollout_authority()
RETURNS trigger AS $$
BEGIN
    IF TG_TABLE_NAME = 'attack_wave_runs' THEN
        IF OLD.status = 'terminal' AND (
            NEW.status IS DISTINCT FROM OLD.status
            OR NEW.terminal_at IS DISTINCT FROM OLD.terminal_at
        ) THEN
            RAISE EXCEPTION 'ATTACK_TERMINAL_WAVE_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'attack_wave_units' THEN
        IF OLD.status = 'terminal' AND (
            NEW.status IS DISTINCT FROM OLD.status
            OR NEW.terminal_at IS DISTINCT FROM OLD.terminal_at
            OR NEW.review_closed IS DISTINCT FROM OLD.review_closed
            OR NEW.verification_closed IS DISTINCT FROM OLD.verification_closed
        ) THEN
            RAISE EXCEPTION 'ATTACK_TERMINAL_WAVE_UNIT_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'stage_run_units' THEN
        IF OLD.stage_kind = 'attack_candidate'
           AND OLD.status = 'passed'
           AND (
                NEW.status IS DISTINCT FROM OLD.status
                OR NEW.terminal_at IS DISTINCT FROM OLD.terminal_at
                OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
                OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
                OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
                OR NEW.stage_kind IS DISTINCT FROM OLD.stage_kind
                OR NEW.generation IS DISTINCT FROM OLD.generation
           ) THEN
            RAISE EXCEPTION 'ATTACK_FINAL_PASSED_CANDIDATE_UNIT_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_terminal_wave_immutable
BEFORE UPDATE ON attack_wave_runs
FOR EACH ROW EXECUTE FUNCTION reject_reopen_of_attack_rollout_authority();

CREATE TRIGGER attack_terminal_wave_unit_immutable
BEFORE UPDATE ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION reject_reopen_of_attack_rollout_authority();

CREATE TRIGGER attack_final_passed_candidate_unit_immutable
BEFORE UPDATE ON stage_run_units
FOR EACH ROW EXECUTE FUNCTION reject_reopen_of_attack_rollout_authority();

-- Once a Candidate final seal has produced its retained shadow row, every
-- relational field used by the canonical rebuild becomes immutable. This
-- keeps the SQL gate/Rust rehydrate interval stable without holding a lock on
-- every evidence child for the lifetime of the deployment cohort.
CREATE FUNCTION reject_closed_attack_shadow_source_change()
RETURNS trigger AS $$
DECLARE
    owner_operation_id UUID;
    owner_stage_run_unit_id UUID;
    owner_work_item_id UUID;
BEGIN
    IF TG_TABLE_NAME = 'attack_candidates' THEN
        IF OLD.operation_uuid IS NOT NULL AND (
            NEW.hypothesis IS DISTINCT FROM OLD.hypothesis
            OR NEW.technique IS DISTINCT FROM OLD.technique
            OR NEW.rationale IS DISTINCT FROM OLD.rationale
            OR NEW.prior_refs IS DISTINCT FROM OLD.prior_refs
            OR NEW.suggested_approach IS DISTINCT FROM OLD.suggested_approach
            OR NEW.priority IS DISTINCT FROM OLD.priority
            OR NEW.execution_plan IS DISTINCT FROM OLD.execution_plan
            OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
            OR NEW.risk_class IS DISTINCT FROM OLD.risk_class
        ) AND EXISTS (
            SELECT 1
              FROM attack_execution_shadow_reads AS shadow
             WHERE shadow.operation_id = OLD.operation_uuid
               AND shadow.stage_run_unit_id = OLD.decision_stage_run_unit_id
        ) THEN
            RAISE EXCEPTION 'ATTACK_CLOSED_SHADOW_SOURCE_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    ELSIF TG_TABLE_NAME = 'attack_candidate_work_items' THEN
        owner_operation_id := CASE WHEN TG_OP = 'DELETE'
            THEN OLD.operation_id ELSE NEW.operation_id END;
        owner_work_item_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
        SELECT stage_unit.id
          INTO owner_stage_run_unit_id
          FROM attack_candidate_work_items AS item
          JOIN attack_wave_units AS wave_unit
            ON wave_unit.id = item.wave_unit_id
           AND wave_unit.operation_id = item.operation_id
           AND wave_unit.scope_snapshot_id = item.scope_snapshot_id
           AND wave_unit.organization_id = item.organization_id
          JOIN attack_wave_runs AS wave
            ON wave.id = wave_unit.wave_run_id
           AND wave.operation_id = wave_unit.operation_id
           AND wave.scope_snapshot_id = wave_unit.scope_snapshot_id
          JOIN stage_run_units AS stage_unit
            ON stage_unit.operation_id = wave.operation_id
           AND stage_unit.scope_snapshot_id = wave.scope_snapshot_id
           AND stage_unit.organization_id = wave_unit.organization_id
           AND stage_unit.generation = wave.generation
           AND stage_unit.stage_kind = 'attack_candidate'
          JOIN attack_execution_shadow_reads AS shadow
            ON shadow.operation_id = stage_unit.operation_id
           AND shadow.stage_run_unit_id = stage_unit.id
         WHERE item.id = owner_work_item_id
           AND item.operation_id = owner_operation_id;
        IF FOUND AND TG_OP = 'UPDATE' AND (
            NEW.decision_kind IS DISTINCT FROM OLD.decision_kind
            OR NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
            OR NEW.no_candidate_reason_code IS DISTINCT FROM OLD.no_candidate_reason_code
            OR NEW.no_candidate_detail IS DISTINCT FROM OLD.no_candidate_detail
            OR NEW.decided_at IS DISTINCT FROM OLD.decided_at
        ) THEN
            RAISE EXCEPTION 'ATTACK_CLOSED_SHADOW_SOURCE_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    ELSIF TG_TABLE_NAME = 'attack_candidate_evidence' THEN
        SELECT candidate.operation_uuid, candidate.decision_stage_run_unit_id
          INTO owner_operation_id, owner_stage_run_unit_id
          FROM attack_candidates AS candidate
         WHERE candidate.candidate_id = CASE WHEN TG_OP = 'DELETE'
             THEN OLD.candidate_id ELSE NEW.candidate_id END;
    ELSIF TG_TABLE_NAME = 'attack_candidate_work_item_evidence' THEN
        owner_work_item_id := CASE WHEN TG_OP = 'DELETE'
            THEN OLD.work_item_id ELSE NEW.work_item_id END;
        SELECT item.operation_id, stage_unit.id
          INTO owner_operation_id, owner_stage_run_unit_id
          FROM attack_candidate_work_items AS item
          JOIN attack_wave_units AS wave_unit
            ON wave_unit.id = item.wave_unit_id
           AND wave_unit.operation_id = item.operation_id
           AND wave_unit.scope_snapshot_id = item.scope_snapshot_id
           AND wave_unit.organization_id = item.organization_id
          JOIN attack_wave_runs AS wave
            ON wave.id = wave_unit.wave_run_id
           AND wave.operation_id = wave_unit.operation_id
           AND wave.scope_snapshot_id = wave_unit.scope_snapshot_id
          JOIN stage_run_units AS stage_unit
            ON stage_unit.operation_id = wave.operation_id
           AND stage_unit.scope_snapshot_id = wave.scope_snapshot_id
           AND stage_unit.organization_id = wave_unit.organization_id
           AND stage_unit.generation = wave.generation
           AND stage_unit.stage_kind = 'attack_candidate'
         WHERE item.id = owner_work_item_id;
    END IF;

    IF owner_operation_id IS NOT NULL
       AND owner_stage_run_unit_id IS NOT NULL
       AND EXISTS (
           SELECT 1
             FROM attack_execution_shadow_reads AS shadow
            WHERE shadow.operation_id = owner_operation_id
              AND shadow.stage_run_unit_id = owner_stage_run_unit_id
       ) THEN
        RAISE EXCEPTION 'ATTACK_CLOSED_SHADOW_SOURCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_attack_candidate_closed_shadow_source_immutable
BEFORE UPDATE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION reject_closed_attack_shadow_source_change();

CREATE TRIGGER aa_attack_work_item_closed_shadow_source_immutable
BEFORE UPDATE OF decision_kind,candidate_id,no_candidate_reason_code,
    no_candidate_detail,decided_at
ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION reject_closed_attack_shadow_source_change();

CREATE TRIGGER aa_attack_candidate_evidence_closed_shadow_source_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_evidence
FOR EACH ROW EXECUTE FUNCTION reject_closed_attack_shadow_source_change();

CREATE TRIGGER aa_attack_work_item_evidence_closed_shadow_source_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_work_item_evidence
FOR EACH ROW EXECUTE FUNCTION reject_closed_attack_shadow_source_change();

-- serde_json canonicalization sorts object keys recursively and emits no
-- structural whitespace.  Reproduce those bytes in SQL so a rollout UPDATE
-- can compare the retained legacy semantic record against canonical Candidate
-- rows without trusting caller-provided hashes or counts.
CREATE FUNCTION attack_execution_canonical_jsonb_text(value JSONB)
RETURNS TEXT AS $$
DECLARE
    rendered TEXT;
BEGIN
    CASE jsonb_typeof(value)
        WHEN 'object' THEN
            SELECT '{' || COALESCE(
                string_agg(
                    to_jsonb(entry.key)::TEXT || ':'
                        || attack_execution_canonical_jsonb_text(entry.value),
                    ',' ORDER BY entry.key COLLATE "C"
                ),
                ''
            ) || '}'
              INTO rendered
              FROM jsonb_each(value) AS entry;
        WHEN 'array' THEN
            SELECT '[' || COALESCE(
                string_agg(
                    attack_execution_canonical_jsonb_text(entry.value),
                    ',' ORDER BY entry.ordinality
                ),
                ''
            ) || ']'
              INTO rendered
              FROM jsonb_array_elements(value) WITH ORDINALITY AS entry(value, ordinality);
        ELSE
            rendered := value::TEXT;
    END CASE;
    RETURN rendered;
END;
$$ LANGUAGE plpgsql IMMUTABLE STRICT;

CREATE FUNCTION attack_execution_sha256_canonical_jsonb(value JSONB)
RETURNS TEXT AS $$
    SELECT encode(
        digest(
            convert_to(attack_execution_canonical_jsonb_text(value), 'UTF8'),
            'sha256'
        ),
        'hex'
    )
$$ LANGUAGE sql IMMUTABLE STRICT;

CREATE FUNCTION attack_execution_shadow_matches_candidate_rows(
    owner_operation_id UUID,
    candidate_stage_run_unit_id UUID
)
RETURNS BOOLEAN AS $$
DECLARE
    owner_wave_unit_id UUID;
    manifest_count INTEGER;
    persisted_count BIGINT;
    rows_complete BOOLEAN;
    expected_record JSONB;
    expected_record_hash TEXT;
    stored_record JSONB;
    stored_record_hash TEXT;
    stored_comparison TEXT;
    stored_source TEXT;
    stored_selected_hash TEXT;
    stored_contract TEXT;
BEGIN
    SELECT wave_unit.id, wave_unit.manifest_count
      INTO owner_wave_unit_id, manifest_count
      FROM stage_run_units AS stage_unit
      JOIN attack_wave_runs AS wave_run
        ON wave_run.operation_id = stage_unit.operation_id
       AND wave_run.scope_snapshot_id = stage_unit.scope_snapshot_id
       AND wave_run.generation = stage_unit.generation
      JOIN attack_wave_units AS wave_unit
        ON wave_unit.wave_run_id = wave_run.id
       AND wave_unit.operation_id = stage_unit.operation_id
       AND wave_unit.scope_snapshot_id = stage_unit.scope_snapshot_id
       AND wave_unit.organization_id = stage_unit.organization_id
      JOIN stage_handoffs AS handoff
        ON handoff.source_stage_run_unit_id = stage_unit.id
       AND handoff.operation_id = stage_unit.operation_id
       AND handoff.stage_execution_id = stage_unit.stage_execution_id
       AND handoff.organization_id = stage_unit.organization_id
       AND handoff.from_stage_kind = stage_unit.stage_kind
       AND handoff.scope_snapshot_id = stage_unit.scope_snapshot_id
       AND handoff.invalidated_at IS NULL
     WHERE stage_unit.id = candidate_stage_run_unit_id
       AND stage_unit.operation_id = owner_operation_id
       AND stage_unit.stage_kind = 'attack_candidate'
       AND stage_unit.status = 'passed'
       AND stage_unit.terminal_at IS NOT NULL
     FOR SHARE OF wave_unit;
    IF NOT FOUND OR manifest_count IS NULL OR manifest_count <= 0 THEN
        RETURN FALSE;
    END IF;

    SELECT COUNT(*), COALESCE(BOOL_AND(
        CASE item.decision_kind
            WHEN 'candidate' THEN
                candidate.candidate_id IS NOT NULL
                AND BTRIM(candidate.hypothesis) <> ''
                AND BTRIM(candidate.rationale) <> ''
                AND candidate.prior_refs IS NOT NULL
                AND BTRIM(candidate.suggested_approach) <> ''
                AND BTRIM(candidate.priority) <> ''
                AND candidate.execution_plan IS NOT NULL
                AND BTRIM(candidate.candidate_plan_hash) <> ''
                AND BTRIM(candidate.risk_class) <> ''
            WHEN 'no_candidate' THEN
                BTRIM(COALESCE(item.no_candidate_reason_code, '')) <> ''
                AND BTRIM(COALESCE(item.no_candidate_detail, '')) <> ''
            ELSE FALSE
        END
    ), FALSE)
      INTO persisted_count, rows_complete
      FROM attack_candidate_work_items AS item
 LEFT JOIN attack_candidates AS candidate
        ON candidate.candidate_id = item.candidate_id
       AND candidate.operation_uuid = item.operation_id
       AND candidate.source_work_item_id = item.id
     WHERE item.operation_id = owner_operation_id
       AND item.wave_unit_id = owner_wave_unit_id;
    IF persisted_count <> manifest_count OR NOT rows_complete THEN
        RETURN FALSE;
    END IF;

    SELECT jsonb_build_object(
        'decisions', COALESCE(
            jsonb_agg(
                jsonb_build_object(
                    'work_item_key', decision.work_item_key,
                    'kind', decision.kind,
                    'semantic_hash', decision.semantic_hash
                ) ORDER BY decision.work_item_key COLLATE "C",
                           decision.kind COLLATE "C",
                           decision.semantic_hash COLLATE "C"
            ),
            '[]'::JSONB
        ),
        'review_counts', jsonb_build_object(
            'wave_unit_count', 1,
            'review_closed_unit_count', 0,
            'candidate_decision_count', COUNT(*) FILTER (
                WHERE decision.kind = 'candidate'
            ),
            'no_candidate_decision_count', COUNT(*) FILTER (
                WHERE decision.kind = 'no_candidate'
            )
        )
    )
      INTO expected_record
      FROM (
          SELECT item.work_item_key,
                 item.decision_kind AS kind,
                 attack_execution_sha256_canonical_jsonb(
                     CASE item.decision_kind
                         WHEN 'candidate' THEN jsonb_build_object(
                             'work_item_id', item.id,
                             'candidate_id', candidate.candidate_id,
                             'hypothesis', candidate.hypothesis,
                             'technique', candidate.technique,
                             'rationale', candidate.rationale,
                             'prior_refs', candidate.prior_refs,
                             'suggested_approach', candidate.suggested_approach,
                             'priority', candidate.priority,
                             'execution_plan', candidate.execution_plan,
                             'candidate_plan_hash', candidate.candidate_plan_hash,
                             'risk_class', candidate.risk_class,
                             'evidence_ids', COALESCE((
                                 SELECT jsonb_agg(link.evidence_id ORDER BY link.evidence_id)
                                   FROM attack_candidate_evidence AS link
                                  WHERE link.candidate_id = candidate.candidate_id
                                    AND link.role = 'support'
                             ), '[]'::JSONB)
                         )
                         ELSE jsonb_build_object(
                             'work_item_id', item.id,
                             'reason_code', item.no_candidate_reason_code,
                             'detail', item.no_candidate_detail,
                             'evidence_ids', COALESCE((
                                 SELECT jsonb_agg(link.evidence_id ORDER BY link.evidence_id)
                                   FROM attack_candidate_work_item_evidence AS link
                                  WHERE link.work_item_id = item.id
                                    AND link.role = 'decision'
                             ), '[]'::JSONB)
                         )
                     END
                 ) AS semantic_hash
            FROM attack_candidate_work_items AS item
       LEFT JOIN attack_candidates AS candidate
              ON candidate.candidate_id = item.candidate_id
             AND candidate.operation_uuid = item.operation_id
             AND candidate.source_work_item_id = item.id
           WHERE item.operation_id = owner_operation_id
             AND item.wave_unit_id = owner_wave_unit_id
      ) AS decision;
    expected_record_hash := attack_execution_sha256_canonical_jsonb(expected_record);

    SELECT shadow.legacy_record,
           shadow.legacy_record_hash,
           shadow.comparison,
           shadow.selected_source,
           shadow.selected_record_hash,
           shadow.attack_execution_contract
      INTO stored_record,
           stored_record_hash,
           stored_comparison,
           stored_source,
           stored_selected_hash,
           stored_contract
      FROM attack_execution_shadow_reads AS shadow
     WHERE shadow.operation_id = owner_operation_id
       AND shadow.stage_run_unit_id = candidate_stage_run_unit_id
     FOR SHARE;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;

    RETURN stored_record = expected_record
       AND stored_record_hash = expected_record_hash
       AND stored_comparison = 'match'
       AND stored_selected_hash = expected_record_hash
       AND (
            (stored_contract = 'dual_write_read_legacy'
                AND stored_source = 'legacy')
            OR
            (stored_contract = 'dual_write_read_v2_fallback'
                AND stored_source = 'v2')
       );
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION attack_execution_candidate_cohort_gate(
    cohort_contract TEXT,
    cohort_rank SMALLINT,
    cohort_cutoff BIGINT
)
RETURNS TABLE (
    admission_count BIGINT,
    candidate_unit_count BIGINT,
    sample_count BIGINT,
    ready BOOLEAN,
    reason TEXT
) AS $$
DECLARE
    admissions BIGINT;
    candidate_units BIGINT;
    samples BIGINT;
BEGIN
    SELECT COUNT(*)
      INTO admissions
      FROM attack_execution_candidate_admissions AS admission
     WHERE admission.attack_execution_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff;
    IF admissions = 0 THEN
        RETURN QUERY SELECT 0::BIGINT, 0::BIGINT, 0::BIGINT, FALSE,
            'candidate_cohort_empty'::TEXT;
        RETURN;
    END IF;

    -- Promotion attests the Candidate shadow domain, not the later human
    -- review/verifier/consolidation lifecycle.  Every materialized WaveUnit
    -- must therefore be either an exact follow-on zero-input authority or a
    -- sealed, positive Candidate manifest.  An open/unsealed unit cannot
    -- disappear from the denominator while promotion holds the rollout lock.
    IF EXISTS (
        SELECT 1
          FROM attack_execution_candidate_admissions AS admission
          JOIN attack_wave_runs AS wave
            ON wave.operation_id = admission.operation_id
           AND wave.scope_snapshot_id = admission.scope_snapshot_id
          JOIN attack_wave_units AS wave_unit
            ON wave_unit.wave_run_id = wave.id
           AND wave_unit.operation_id = wave.operation_id
           AND wave_unit.scope_snapshot_id = wave.scope_snapshot_id
         WHERE admission.attack_execution_contract = cohort_contract
           AND admission.rollout_rank = cohort_rank
           AND admission.admission_seq <= cohort_cutoff
           AND NOT (
                wave_unit.entry_consolidation_id IS NOT NULL
                AND wave_unit.status = 'terminal'
                AND wave_unit.terminal_at IS NOT NULL
                AND wave_unit.review_closed
                AND wave_unit.verification_closed
                AND wave_unit.consolidation_status = 'terminal'
                AND wave_unit.manifest_hash IS NULL
                AND wave_unit.manifest_count IS NULL
                AND wave_unit.manifest_frozen_at IS NULL
                AND NOT EXISTS (
                    SELECT 1
                      FROM stage_run_units AS zero_stage_unit
                     WHERE zero_stage_unit.operation_id = wave.operation_id
                       AND zero_stage_unit.scope_snapshot_id = wave.scope_snapshot_id
                       AND zero_stage_unit.organization_id = wave_unit.organization_id
                       AND zero_stage_unit.generation = wave.generation
                       AND zero_stage_unit.stage_kind = 'attack_candidate'
                )
           )
           AND (
                BTRIM(COALESCE(wave_unit.manifest_hash, '')) = ''
                OR wave_unit.manifest_count IS NULL
                OR wave_unit.manifest_count <= 0
                OR wave_unit.manifest_frozen_at IS NULL
           )
    ) THEN
        RETURN QUERY SELECT admissions, 0::BIGINT, 0::BIGINT, FALSE,
            'candidate_manifest_not_sealed'::TEXT;
        RETURN;
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_execution_candidate_admissions AS admission
          JOIN attack_wave_runs AS wave
            ON wave.operation_id = admission.operation_id
           AND wave.scope_snapshot_id = admission.scope_snapshot_id
          JOIN attack_wave_units AS wave_unit
            ON wave_unit.wave_run_id = wave.id
           AND wave_unit.operation_id = wave.operation_id
           AND wave_unit.scope_snapshot_id = wave.scope_snapshot_id
     LEFT JOIN LATERAL (
            SELECT COUNT(*) AS total_count,
                   COUNT(*) FILTER (
                       WHERE stage_unit.status = 'passed'
                         AND stage_unit.terminal_at IS NOT NULL
                   ) AS passed_count
              FROM stage_run_units AS stage_unit
             WHERE stage_unit.operation_id = wave.operation_id
               AND stage_unit.scope_snapshot_id = wave.scope_snapshot_id
               AND stage_unit.organization_id = wave_unit.organization_id
               AND stage_unit.generation = wave.generation
               AND stage_unit.stage_kind = 'attack_candidate'
          ) AS candidate ON TRUE
         WHERE admission.attack_execution_contract = cohort_contract
           AND admission.rollout_rank = cohort_rank
           AND admission.admission_seq <= cohort_cutoff
           AND NOT (
                wave_unit.entry_consolidation_id IS NOT NULL
                AND wave_unit.status = 'terminal'
                AND wave_unit.terminal_at IS NOT NULL
                AND wave_unit.review_closed
                AND wave_unit.verification_closed
                AND wave_unit.consolidation_status = 'terminal'
                AND wave_unit.manifest_hash IS NULL
                AND wave_unit.manifest_count IS NULL
                AND wave_unit.manifest_frozen_at IS NULL
                AND candidate.total_count = 0
           )
           AND (candidate.total_count <> 1 OR candidate.passed_count <> 1)
    ) THEN
        RETURN QUERY SELECT admissions, 0::BIGINT, 0::BIGINT, FALSE,
            'candidate_final_unit_missing_or_ambiguous'::TEXT;
        RETURN;
    END IF;

    SELECT COUNT(*)
      INTO candidate_units
      FROM attack_execution_candidate_admissions AS admission
      JOIN attack_wave_runs AS wave
        ON wave.operation_id = admission.operation_id
       AND wave.scope_snapshot_id = admission.scope_snapshot_id
      JOIN attack_wave_units AS wave_unit
        ON wave_unit.wave_run_id = wave.id
       AND wave_unit.operation_id = wave.operation_id
       AND wave_unit.scope_snapshot_id = wave.scope_snapshot_id
     WHERE admission.attack_execution_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff
       AND NOT (
            wave_unit.entry_consolidation_id IS NOT NULL
            AND wave_unit.status = 'terminal'
            AND wave_unit.terminal_at IS NOT NULL
            AND wave_unit.review_closed
            AND wave_unit.verification_closed
            AND wave_unit.consolidation_status = 'terminal'
            AND wave_unit.manifest_hash IS NULL
            AND wave_unit.manifest_count IS NULL
            AND wave_unit.manifest_frozen_at IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM stage_run_units AS zero_stage_unit
                 WHERE zero_stage_unit.operation_id = wave.operation_id
                   AND zero_stage_unit.scope_snapshot_id = wave.scope_snapshot_id
                   AND zero_stage_unit.organization_id = wave_unit.organization_id
                   AND zero_stage_unit.generation = wave.generation
                   AND zero_stage_unit.stage_kind = 'attack_candidate'
            )
       );

    SELECT COUNT(*)
      INTO samples
      FROM attack_execution_candidate_admissions AS admission
      JOIN attack_wave_runs AS wave
        ON wave.operation_id = admission.operation_id
       AND wave.scope_snapshot_id = admission.scope_snapshot_id
      JOIN attack_wave_units AS wave_unit
        ON wave_unit.wave_run_id = wave.id
       AND wave_unit.operation_id = wave.operation_id
       AND wave_unit.scope_snapshot_id = wave.scope_snapshot_id
      JOIN stage_run_units AS stage_unit
        ON stage_unit.operation_id = wave.operation_id
       AND stage_unit.scope_snapshot_id = wave.scope_snapshot_id
       AND stage_unit.organization_id = wave_unit.organization_id
       AND stage_unit.generation = wave.generation
       AND stage_unit.stage_kind = 'attack_candidate'
       AND stage_unit.status = 'passed'
       AND stage_unit.terminal_at IS NOT NULL
      JOIN attack_execution_shadow_reads AS shadow
        ON shadow.operation_id = stage_unit.operation_id
       AND shadow.stage_run_unit_id = stage_unit.id
       AND shadow.attack_execution_contract = cohort_contract
     WHERE admission.attack_execution_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff
       AND NOT (
            wave_unit.entry_consolidation_id IS NOT NULL
            AND wave_unit.status = 'terminal'
            AND wave_unit.terminal_at IS NOT NULL
            AND wave_unit.review_closed
            AND wave_unit.verification_closed
            AND wave_unit.consolidation_status = 'terminal'
            AND wave_unit.manifest_hash IS NULL
            AND wave_unit.manifest_count IS NULL
            AND wave_unit.manifest_frozen_at IS NULL
       )
       AND attack_execution_shadow_matches_candidate_rows(
            stage_unit.operation_id,
            stage_unit.id
       );

    IF samples <> candidate_units THEN
        RETURN QUERY SELECT admissions, candidate_units, samples, FALSE,
            'candidate_shadow_missing_incomplete_or_mismatch'::TEXT;
        RETURN;
    END IF;
    RETURN QUERY SELECT admissions, candidate_units, samples, TRUE, 'ready'::TEXT;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION enforce_attack_execution_rollout_transition()
RETURNS trigger AS $$
DECLARE
    cutoff BIGINT;
    gate_ready BOOLEAN;
    gate_reason TEXT;
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

    IF OLD.rank IN (1, 2) THEN
        SELECT MAX(admission.admission_seq)
          INTO cutoff
          FROM attack_execution_candidate_admissions AS admission
         WHERE admission.attack_execution_contract = OLD.contract
           AND admission.rollout_rank = OLD.rank;
        IF cutoff IS NULL THEN
            RAISE EXCEPTION 'ATTACK_ROLLOUT_COHORT_NOT_READY: candidate_cohort_empty'
                USING ERRCODE = '55000';
        END IF;
        SELECT gate.ready,
               gate.reason
          INTO gate_ready,
               gate_reason
          FROM attack_execution_candidate_cohort_gate(
              OLD.contract,
              OLD.rank,
              cutoff
          ) AS gate;
        IF NOT gate_ready THEN
            RAISE EXCEPTION 'ATTACK_ROLLOUT_COHORT_NOT_READY: %', gate_reason
                USING ERRCODE = '55000';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION record_attack_execution_rollout_promotion_receipt()
RETURNS trigger AS $$
BEGIN
    IF OLD.rank IN (1, 2) THEN
        -- from_rank is the sole semantic input.  The receipt-table BEFORE
        -- trigger runs after the singleton update and reconstructs the entire
        -- receipt from the old-contract cohort plus the new singleton state.
        INSERT INTO attack_execution_rollout_promotions(from_rank)
        VALUES (OLD.rank);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_attack_execution_rollout_promotion_receipt
AFTER UPDATE ON attack_execution_rollout
FOR EACH ROW EXECUTE FUNCTION record_attack_execution_rollout_promotion_receipt();
