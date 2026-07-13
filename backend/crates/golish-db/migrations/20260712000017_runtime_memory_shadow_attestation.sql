-- Runtime-memory retained whole-record shadow attestation and guarded rollout.
--
-- Migration 00002 only enables rank-one dual writes.  Every later deployment
-- default transition is decided from immutable database-owned admissions and
-- observations. Existing operations keep their frozen contract.

CREATE FUNCTION runtime_memory_state_blob_has_legacy_checkpoint(candidate JSONB)
RETURNS BOOLEAN AS $$
    SELECT COALESCE(candidate ?| ARRAY[
        'graph_flow',
        'profile',
        'current_stage',
        'current_stage_run_id',
        'queue_titles',
        'completed_count',
        'continuity_adoption',
        'schema_v',
        'stage_run_workers',
        'stage_run_handoffs',
        'agent_run'
    ], FALSE);
$$ LANGUAGE sql IMMUTABLE;

CREATE FUNCTION assert_runtime_memory_shadow_attestation_installable()
RETURNS VOID AS $$
BEGIN
    -- The retained ledgers introduced below cannot safely attest a promotion
    -- that happened before they existed. Never silently bless missing history.
    IF NOT EXISTS (
        SELECT 1
          FROM runtime_memory_rollout AS rollout
         WHERE rollout.singleton_id = 1
           AND rollout.contract = 'dual_write_legacy_read'
           AND rollout.contract_rank = 1
           AND rollout.row_version = 1
    ) THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ATTESTATION_REQUIRES_RANK_ONE'
            USING ERRCODE = '55000';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM runtime_memory_rollout AS runtime
         CROSS JOIN attack_execution_rollout AS attack
         WHERE runtime.singleton_id = 1
           AND attack.singleton = TRUE
           AND (
                attack.rank = 0
                OR (attack.rank IN (1, 2) AND runtime.contract_rank >= 1)
                OR (attack.rank = 3 AND runtime.contract_rank = 3)
           )
    ) THEN
        RAISE EXCEPTION 'EXECUTION_ROLLOUT_PAIR_INCOMPATIBLE_EXISTING'
            USING ERRCODE = '55000';
    END IF;

    -- A V2-only operation resumes exclusively from relational execution truth.
    -- Installing the boundary over a legacy checkpoint would silently retain a
    -- second authority, so fail before adding the DML guard.
    IF EXISTS (
        SELECT 1
          FROM operation_state
         WHERE runtime_memory_contract = 'v2_only'
           AND runtime_memory_state_blob_has_legacy_checkpoint(state_blob)
    ) THEN
        RAISE EXCEPTION 'V2_ONLY_LEGACY_CHECKPOINT_EXISTING'
            USING ERRCODE = '55000';
    END IF;
END;
$$ LANGUAGE plpgsql;

SELECT assert_runtime_memory_shadow_attestation_installable();

ALTER TABLE operation_state
    ADD CONSTRAINT operation_rollout_contract_pair_compatible
    CHECK (
        attack_execution_contract = 'legacy'
        OR (
            attack_execution_contract IN (
                'dual_write_read_legacy',
                'dual_write_read_v2_fallback'
            )
            AND runtime_memory_contract <> 'legacy_v1'
        )
        OR (
            attack_execution_contract = 'v2_only'
            AND runtime_memory_contract = 'v2_only'
        )
    ) NOT VALID;

DO $existing_operation_contract_pair_scan$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM operation_state
         WHERE NOT (
             attack_execution_contract = 'legacy'
             OR (
                 attack_execution_contract IN (
                     'dual_write_read_legacy',
                     'dual_write_read_v2_fallback'
                 )
                 AND runtime_memory_contract <> 'legacy_v1'
             )
             OR (
                 attack_execution_contract = 'v2_only'
                 AND runtime_memory_contract = 'v2_only'
             )
         )
    ) THEN
        RAISE EXCEPTION 'OPERATION_ROLLOUT_CONTRACT_PAIR_INCOMPATIBLE_EXISTING'
            USING ERRCODE = '23514';
    END IF;
END;
$existing_operation_contract_pair_scan$;

ALTER TABLE operation_state
    VALIDATE CONSTRAINT operation_rollout_contract_pair_compatible;

CREATE FUNCTION reject_v2_only_legacy_checkpoint_state_blob()
RETURNS trigger AS $$
BEGIN
    IF (
           (TG_OP = 'INSERT' AND NEW.runtime_memory_contract = 'v2_only')
           OR (TG_OP = 'UPDATE' AND OLD.runtime_memory_contract = 'v2_only')
       )
       AND runtime_memory_state_blob_has_legacy_checkpoint(NEW.state_blob)
    THEN
        RAISE EXCEPTION 'V2_ONLY_LEGACY_CHECKPOINT_FORBIDDEN'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_v2_only_legacy_checkpoint_guard
BEFORE INSERT OR UPDATE OF state_blob ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_v2_only_legacy_checkpoint_state_blob();

ALTER TABLE operation_state
    ADD CONSTRAINT operation_state_runtime_contract_identity_unique
    UNIQUE (operation_id, runtime_memory_contract);

CREATE TABLE runtime_memory_rollout_admissions (
    admission_seq BIGSERIAL PRIMARY KEY,
    worker_run_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    runtime_memory_contract TEXT NOT NULL CHECK (
        runtime_memory_contract IN (
            'dual_write_legacy_read',
            'dual_write_v2_preferred'
        )
    ),
    rollout_rank SMALLINT NOT NULL CHECK (rollout_rank IN (1, 2)),
    rollout_row_version BIGINT NOT NULL CHECK (rollout_row_version >= 0),
    admitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (admission_seq, runtime_memory_contract, rollout_rank),
    FOREIGN KEY (operation_id, runtime_memory_contract)
        REFERENCES operation_state(operation_id, runtime_memory_contract)
        ON DELETE RESTRICT,
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
    ) ON DELETE RESTRICT
      DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        rollout_rank = CASE runtime_memory_contract
            WHEN 'dual_write_legacy_read' THEN 1
            WHEN 'dual_write_v2_preferred' THEN 2
        END
    )
);

CREATE INDEX runtime_memory_rollout_admissions_cohort_idx
    ON runtime_memory_rollout_admissions(
        runtime_memory_contract,
        rollout_rank,
        admission_seq
    );

CREATE TABLE runtime_memory_shadow_samples (
    sample_seq BIGSERIAL PRIMARY KEY,
    admission_seq BIGINT NOT NULL,
    worker_run_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    runtime_memory_contract TEXT NOT NULL,
    rollout_rank SMALLINT NOT NULL,
    mutation_kind TEXT NOT NULL CHECK (
        mutation_kind <> '' AND length(mutation_kind) <= 96
    ),
    legacy_record JSONB,
    v2_record JSONB,
    legacy_record_hash TEXT,
    v2_record_hash TEXT,
    comparison TEXT NOT NULL CHECK (
        comparison IN ('match', 'mismatch', 'legacy_missing', 'v2_missing')
    ),
    selected_source TEXT NOT NULL CHECK (
        selected_source IN ('legacy', 'v2')
    ),
    selected_record JSONB,
    selected_record_hash TEXT,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (admission_seq, runtime_memory_contract, rollout_rank)
        REFERENCES runtime_memory_rollout_admissions(
            admission_seq,
            runtime_memory_contract,
            rollout_rank
        ) ON DELETE RESTRICT,
    FOREIGN KEY (worker_run_id)
        REFERENCES runtime_memory_rollout_admissions(worker_run_id)
        ON DELETE RESTRICT,
    CHECK (legacy_record_hash IS NULL OR length(legacy_record_hash) = 64),
    CHECK (v2_record_hash IS NULL OR length(v2_record_hash) = 64),
    CHECK (selected_record_hash IS NULL OR length(selected_record_hash) = 64)
);

CREATE INDEX runtime_memory_shadow_samples_admission_idx
    ON runtime_memory_shadow_samples(admission_seq, sample_seq);

CREATE TABLE runtime_memory_rollout_promotions (
    from_rank SMALLINT PRIMARY KEY CHECK (from_rank IN (1, 2)),
    to_rank SMALLINT NOT NULL CHECK (to_rank = from_rank + 1),
    from_contract TEXT NOT NULL,
    to_contract TEXT NOT NULL,
    from_row_version BIGINT NOT NULL CHECK (from_row_version >= 0),
    to_row_version BIGINT NOT NULL CHECK (to_row_version = from_row_version + 1),
    admission_cutoff BIGINT NOT NULL CHECK (admission_cutoff > 0),
    admission_count BIGINT NOT NULL CHECK (admission_count > 0),
    sample_count BIGINT NOT NULL CHECK (sample_count >= admission_count),
    aggregate_digest TEXT NOT NULL CHECK (length(aggregate_digest) = 64),
    promoted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (from_rank = 1
            AND from_contract = 'dual_write_legacy_read'
            AND to_contract = 'dual_write_v2_preferred')
        OR
        (from_rank = 2
            AND from_contract = 'dual_write_v2_preferred'
            AND to_contract = 'v2_only')
    )
);

CREATE FUNCTION runtime_memory_timestamp_micros(value JSONB)
RETURNS BIGINT AS $$
BEGIN
    IF value IS NULL OR jsonb_typeof(value) = 'null' THEN
        RETURN NULL;
    END IF;
    IF jsonb_typeof(value) = 'number' THEN
        RETURN (value #>> '{}')::BIGINT;
    END IF;
    IF jsonb_typeof(value) = 'string' THEN
        RETURN ROUND(
            EXTRACT(EPOCH FROM ((value #>> '{}')::TIMESTAMPTZ)) * 1000000
        )::BIGINT;
    END IF;
    RETURN NULL;
EXCEPTION WHEN OTHERS THEN
    RETURN NULL;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE FUNCTION runtime_memory_normalize_worker_record(record JSONB)
RETURNS JSONB AS $$
BEGIN
    IF record IS NULL OR jsonb_typeof(record) <> 'object' THEN
        RETURN NULL;
    END IF;
    RETURN jsonb_build_object(
        'schema_v', (record->>'schema_v')::INTEGER,
        'id', to_jsonb((record->>'id')::UUID),
        'operation_id', to_jsonb((record->>'operation_id')::UUID),
        'stage_execution_id', to_jsonb((record->>'stage_execution_id')::UUID),
        'stage_run_unit_id', to_jsonb((record->>'stage_run_unit_id')::UUID),
        'worker_run_id', to_jsonb((record->>'worker_run_id')::UUID),
        'organization_id', to_jsonb((record->>'organization_id')::UUID),
        'org_name', record->>'org_name',
        'worker_generation', (record->>'worker_generation')::INTEGER,
        'specialist', record->>'specialist',
        'work_item_kind', record->>'work_item_kind',
        'work_item_key', record->>'work_item_key',
        'agent_path', record->>'agent_path',
        'parent_request_id', record->>'parent_request_id',
        'chain_id', CASE
            WHEN record->'chain_id' IS NULL OR jsonb_typeof(record->'chain_id') = 'null'
                THEN NULL
            ELSE to_jsonb((record->>'chain_id')::UUID)
        END,
        'message_chain_id', CASE
            WHEN record->'message_chain_id' IS NULL
              OR jsonb_typeof(record->'message_chain_id') = 'null'
                THEN NULL
            ELSE to_jsonb((record->>'message_chain_id')::UUID)
        END,
        'status', record->>'status',
        'gate_attempt', (record->>'gate_attempt')::INTEGER,
        'checkpoint', record->'checkpoint',
        'checkpoint_version', (record->>'checkpoint_version')::BIGINT,
        'lease_token', CASE
            WHEN record->'lease_token' IS NULL OR jsonb_typeof(record->'lease_token') = 'null'
                THEN NULL
            ELSE to_jsonb((record->>'lease_token')::UUID)
        END,
        'lease_owner', record->>'lease_owner',
        'lease_acquired_at', runtime_memory_timestamp_micros(record->'lease_acquired_at'),
        'lease_expires_at', runtime_memory_timestamp_micros(record->'lease_expires_at'),
        'heartbeat_at', runtime_memory_timestamp_micros(record->'heartbeat_at'),
        'attempt_epoch', (record->>'attempt_epoch')::BIGINT,
        'active_tool_call_id', CASE
            WHEN record->'active_tool_call_id' IS NULL
              OR jsonb_typeof(record->'active_tool_call_id') = 'null'
                THEN NULL
            ELSE to_jsonb((record->>'active_tool_call_id')::UUID)
        END,
        'active_tool_started_at', runtime_memory_timestamp_micros(record->'active_tool_started_at'),
        'evidence_watermark', CASE
            WHEN record->'evidence_watermark' IS NULL
              OR jsonb_typeof(record->'evidence_watermark') = 'null'
                THEN NULL
            ELSE (record->>'evidence_watermark')::BIGINT
        END,
        'started_at', runtime_memory_timestamp_micros(record->'started_at'),
        'updated_at', runtime_memory_timestamp_micros(record->'updated_at'),
        'terminal_at', runtime_memory_timestamp_micros(record->'terminal_at')
    );
EXCEPTION WHEN OTHERS THEN
    RETURN NULL;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE FUNCTION runtime_memory_v2_worker_record(requested_worker_id UUID)
RETURNS JSONB AS $$
    SELECT jsonb_build_object(
        'schema_v', 2,
        'id', to_jsonb(worker.id),
        'operation_id', to_jsonb(worker.operation_id),
        'stage_execution_id', to_jsonb(worker.stage_execution_id),
        'stage_run_unit_id', to_jsonb(worker.stage_run_unit_id),
        'worker_run_id', to_jsonb(worker.id),
        'organization_id', to_jsonb(worker.organization_id),
        'org_name', member.organization_name_at_freeze,
        'worker_generation', worker.worker_generation,
        'specialist', worker.specialist,
        'work_item_kind', worker.work_item_kind,
        'work_item_key', worker.work_item_key,
        'agent_path', worker.agent_path,
        'parent_request_id', worker.parent_request_id,
        'chain_id', to_jsonb(worker.message_chain_id),
        'message_chain_id', to_jsonb(worker.message_chain_id),
        'status', worker.status,
        'gate_attempt', worker.gate_attempt,
        'checkpoint', worker.checkpoint,
        'checkpoint_version', worker.checkpoint_version,
        'lease_token', to_jsonb(worker.lease_token),
        'lease_owner', worker.lease_owner,
        'lease_acquired_at', runtime_memory_timestamp_micros(to_jsonb(worker.lease_acquired_at)),
        'lease_expires_at', runtime_memory_timestamp_micros(to_jsonb(worker.lease_expires_at)),
        'heartbeat_at', runtime_memory_timestamp_micros(to_jsonb(worker.heartbeat_at)),
        'attempt_epoch', worker.attempt_epoch,
        'active_tool_call_id', to_jsonb(worker.active_tool_call_id),
        'active_tool_started_at', runtime_memory_timestamp_micros(to_jsonb(worker.active_tool_started_at)),
        'evidence_watermark', worker.evidence_watermark,
        'started_at', runtime_memory_timestamp_micros(to_jsonb(worker.started_at)),
        'updated_at', runtime_memory_timestamp_micros(to_jsonb(worker.updated_at)),
        'terminal_at', runtime_memory_timestamp_micros(to_jsonb(worker.terminal_at))
    )
      FROM stage_worker_runs AS worker
      JOIN stage_run_units AS unit
        ON unit.id = worker.stage_run_unit_id
       AND unit.operation_id = worker.operation_id
       AND unit.stage_execution_id = worker.stage_execution_id
       AND unit.organization_id = worker.organization_id
      JOIN operation_org_scope_units AS member
        ON member.snapshot_id = unit.scope_snapshot_id
       AND member.organization_id = unit.organization_id
     WHERE worker.id = requested_worker_id;
$$ LANGUAGE sql STABLE;

CREATE FUNCTION runtime_memory_legacy_worker_record(requested_worker_id UUID)
RETURNS JSONB AS $$
    SELECT runtime_memory_normalize_worker_record(
        operation.state_blob #> ARRAY[
            'stage_run_workers',
            unit.stage_kind,
            worker.organization_id::TEXT,
            'worker_records',
            worker.id::TEXT
        ]
    )
      FROM stage_worker_runs AS worker
      JOIN stage_run_units AS unit
        ON unit.id = worker.stage_run_unit_id
       AND unit.operation_id = worker.operation_id
       AND unit.stage_execution_id = worker.stage_execution_id
       AND unit.organization_id = worker.organization_id
      JOIN operation_state AS operation
        ON operation.operation_id = worker.operation_id
     WHERE worker.id = requested_worker_id;
$$ LANGUAGE sql STABLE;

CREATE FUNCTION runtime_memory_json_sha256(value JSONB)
RETURNS TEXT AS $$
    SELECT CASE WHEN value IS NULL THEN NULL ELSE
        encode(digest(convert_to(value::TEXT, 'UTF8'), 'sha256'), 'hex')
    END;
$$ LANGUAGE sql IMMUTABLE;

CREATE FUNCTION lock_execution_rollout_pair()
RETURNS VOID AS $$
BEGIN
    -- Admission/promotion serialization relies on each command observing the
    -- latest committed cohort after lock waits. Fixed transaction snapshots
    -- cannot provide that guarantee, so raw callers must fail closed.
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'EXECUTION_ROLLOUT_REQUIRES_READ_COMMITTED'
            USING ERRCODE = '25001';
    END IF;
    PERFORM pg_advisory_xact_lock(7142026, 120017);
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION lock_execution_rollout_pair_trigger()
RETURNS trigger AS $$
BEGIN
    PERFORM lock_execution_rollout_pair();
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_runtime_memory_rollout_pair_lock
BEFORE UPDATE ON runtime_memory_rollout
FOR EACH STATEMENT EXECUTE FUNCTION lock_execution_rollout_pair_trigger();

CREATE TRIGGER aa_attack_execution_rollout_pair_lock
BEFORE UPDATE ON attack_execution_rollout
FOR EACH STATEMENT EXECUTE FUNCTION lock_execution_rollout_pair_trigger();

CREATE FUNCTION reject_direct_runtime_memory_rollout_admission()
RETURNS trigger AS $$
BEGIN
    IF TG_OP IN ('UPDATE', 'DELETE') OR pg_trigger_depth() = 1 THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_ADMISSION_IMMUTABLE'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION admit_runtime_memory_worker()
RETURNS trigger AS $$
DECLARE
    frozen_contract TEXT;
BEGIN
    SELECT operation.runtime_memory_contract
      INTO frozen_contract
      FROM operation_state AS operation
     WHERE operation.operation_id = NEW.operation_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ADMISSION_OPERATION_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF frozen_contract NOT IN (
        'dual_write_legacy_read',
        'dual_write_v2_preferred'
    ) THEN
        RETURN NEW;
    END IF;
    -- The admission-table prepare trigger reconstructs every other field from
    -- the now-persisted WorkerRun, frozen operation and rollout singleton.
    -- Supplying a placeholder avoids consuming the BIGSERIAL default before
    -- the prepare trigger allocates the sole database-owned sequence value.
    INSERT INTO runtime_memory_rollout_admissions(admission_seq,worker_run_id)
    VALUES (0,NEW.id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER runtime_memory_worker_admission
AFTER INSERT ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION admit_runtime_memory_worker();

INSERT INTO runtime_memory_rollout_admissions(
    worker_run_id,
    operation_id,
    stage_execution_id,
    stage_run_unit_id,
    organization_id,
    runtime_memory_contract,
    rollout_rank,
    rollout_row_version,
    admitted_at
)
SELECT worker.id,
       worker.operation_id,
       worker.stage_execution_id,
       worker.stage_run_unit_id,
       worker.organization_id,
       operation.runtime_memory_contract,
       CASE operation.runtime_memory_contract
           WHEN 'dual_write_legacy_read' THEN 1
           WHEN 'dual_write_v2_preferred' THEN 2
       END,
       rollout.row_version,
       worker.updated_at
  FROM stage_worker_runs AS worker
  JOIN operation_state AS operation
    ON operation.operation_id = worker.operation_id
 CROSS JOIN runtime_memory_rollout AS rollout
 WHERE rollout.singleton_id = 1
   AND operation.runtime_memory_contract IN (
       'dual_write_legacy_read',
       'dual_write_v2_preferred'
   )
 ORDER BY worker.operation_id,worker.stage_execution_id,
          worker.stage_run_unit_id,worker.id;

CREATE FUNCTION prepare_runtime_memory_rollout_admission()
RETURNS trigger AS $$
DECLARE
    worker stage_worker_runs%ROWTYPE;
    frozen_contract TEXT;
    frozen_rank SMALLINT;
    current_rank SMALLINT;
    current_row_version BIGINT;
BEGIN
    SELECT persisted.*
      INTO worker
      FROM stage_worker_runs AS persisted
     WHERE persisted.id = NEW.worker_run_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ADMISSION_WORKER_MISSING'
            USING ERRCODE = '23503';
    END IF;

    SELECT operation.runtime_memory_contract
      INTO frozen_contract
      FROM operation_state AS operation
     WHERE operation.operation_id = worker.operation_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ADMISSION_OPERATION_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF frozen_contract NOT IN (
        'dual_write_legacy_read',
        'dual_write_v2_preferred'
    ) THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ADMISSION_CONTRACT_NOT_DUAL'
            USING ERRCODE = '23514';
    END IF;
    frozen_rank := CASE frozen_contract
        WHEN 'dual_write_legacy_read' THEN 1
        WHEN 'dual_write_v2_preferred' THEN 2
    END;

    SELECT rollout.contract_rank,rollout.row_version
      INTO current_rank,current_row_version
      FROM runtime_memory_rollout AS rollout
     WHERE rollout.singleton_id = 1
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_SINGLETON_MISSING'
            USING ERRCODE = '55000';
    END IF;
    IF frozen_rank > current_rank THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ADMISSION_FUTURE_CONTRACT'
            USING ERRCODE = '55000';
    END IF;

    NEW.admission_seq := nextval(
        pg_get_serial_sequence(
            'runtime_memory_rollout_admissions',
            'admission_seq'
        )::regclass
    );
    NEW.worker_run_id := worker.id;
    NEW.operation_id := worker.operation_id;
    NEW.stage_execution_id := worker.stage_execution_id;
    NEW.stage_run_unit_id := worker.stage_run_unit_id;
    NEW.organization_id := worker.organization_id;
    NEW.runtime_memory_contract := frozen_contract;
    NEW.rollout_rank := frozen_rank;
    NEW.rollout_row_version := current_row_version;
    NEW.admitted_at := NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_runtime_memory_rollout_admission_prepare
BEFORE INSERT ON runtime_memory_rollout_admissions
FOR EACH ROW EXECUTE FUNCTION prepare_runtime_memory_rollout_admission();

CREATE TRIGGER runtime_memory_rollout_admission_immutable
BEFORE INSERT OR UPDATE OR DELETE ON runtime_memory_rollout_admissions
FOR EACH ROW EXECUTE FUNCTION reject_direct_runtime_memory_rollout_admission();

CREATE FUNCTION prepare_runtime_memory_shadow_sample()
RETURNS trigger AS $$
DECLARE
    admission runtime_memory_rollout_admissions%ROWTYPE;
BEGIN
    SELECT cohort.*
      INTO admission
      FROM runtime_memory_rollout_admissions AS cohort
     WHERE cohort.worker_run_id = NEW.worker_run_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_SHADOW_ADMISSION_MISSING'
            USING ERRCODE = '23503';
    END IF;

    NEW.sample_seq := nextval(
        pg_get_serial_sequence(
            'runtime_memory_shadow_samples',
            'sample_seq'
        )::regclass
    );
    NEW.admission_seq := admission.admission_seq;
    NEW.operation_id := admission.operation_id;
    NEW.stage_execution_id := admission.stage_execution_id;
    NEW.stage_run_unit_id := admission.stage_run_unit_id;
    NEW.organization_id := admission.organization_id;
    NEW.runtime_memory_contract := admission.runtime_memory_contract;
    NEW.rollout_rank := admission.rollout_rank;
    NEW.legacy_record := runtime_memory_legacy_worker_record(NEW.worker_run_id);
    NEW.v2_record := runtime_memory_v2_worker_record(NEW.worker_run_id);
    NEW.legacy_record_hash := runtime_memory_json_sha256(NEW.legacy_record);
    NEW.v2_record_hash := runtime_memory_json_sha256(NEW.v2_record);
    NEW.comparison := CASE
        WHEN NEW.legacy_record IS NULL THEN 'legacy_missing'
        WHEN NEW.v2_record IS NULL THEN 'v2_missing'
        WHEN NEW.legacy_record = NEW.v2_record THEN 'match'
        ELSE 'mismatch'
    END;
    NEW.selected_source := CASE admission.runtime_memory_contract
        WHEN 'dual_write_legacy_read' THEN 'legacy'
        WHEN 'dual_write_v2_preferred' THEN 'v2'
    END;
    NEW.selected_record := CASE NEW.selected_source
        WHEN 'legacy' THEN NEW.legacy_record
        WHEN 'v2' THEN NEW.v2_record
    END;
    NEW.selected_record_hash := runtime_memory_json_sha256(NEW.selected_record);
    NEW.observed_at := NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION reject_runtime_memory_shadow_sample_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'RUNTIME_MEMORY_SHADOW_SAMPLE_IMMUTABLE'
        USING ERRCODE = '42501';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER runtime_memory_shadow_sample_prepare
BEFORE INSERT ON runtime_memory_shadow_samples
FOR EACH ROW EXECUTE FUNCTION prepare_runtime_memory_shadow_sample();

CREATE TRIGGER runtime_memory_shadow_sample_immutable
BEFORE UPDATE OR DELETE ON runtime_memory_shadow_samples
FOR EACH ROW EXECUTE FUNCTION reject_runtime_memory_shadow_sample_change();

CREATE FUNCTION backfill_runtime_memory_shadow_samples()
RETURNS BIGINT AS $$
DECLARE
    inserted_rows BIGINT;
BEGIN
    -- This is a database-rehydrated upgrade observation, not a caller payload.
    -- Serialize explicit replays with admissions and sample writers so every
    -- pre-existing admitted WorkerRun receives exactly one retained snapshot.
    PERFORM lock_execution_rollout_pair();
    LOCK TABLE runtime_memory_rollout_admissions IN SHARE ROW EXCLUSIVE MODE;
    LOCK TABLE runtime_memory_shadow_samples IN SHARE ROW EXCLUSIVE MODE;
    INSERT INTO runtime_memory_shadow_samples(
        sample_seq,
        worker_run_id,
        mutation_kind
    )
    SELECT 0,
           admission.worker_run_id,
           'migration_backfill'
      FROM runtime_memory_rollout_admissions AS admission
     WHERE NOT EXISTS (
            SELECT 1
              FROM runtime_memory_shadow_samples AS sample
             WHERE sample.admission_seq = admission.admission_seq
       )
     ORDER BY admission.admission_seq;
    GET DIAGNOSTICS inserted_rows = ROW_COUNT;
    RETURN inserted_rows;
END;
$$ LANGUAGE plpgsql;

SELECT backfill_runtime_memory_shadow_samples();

CREATE FUNCTION runtime_memory_shadow_sample_is_valid(requested_sample_seq BIGINT)
RETURNS BOOLEAN AS $$
    SELECT sample.admission_seq = admission.admission_seq
       AND sample.worker_run_id = admission.worker_run_id
       AND sample.operation_id = admission.operation_id
       AND sample.stage_execution_id = admission.stage_execution_id
       AND sample.stage_run_unit_id = admission.stage_run_unit_id
       AND sample.organization_id = admission.organization_id
       AND sample.runtime_memory_contract = admission.runtime_memory_contract
       AND sample.rollout_rank = admission.rollout_rank
       AND sample.legacy_record_hash IS NOT DISTINCT FROM
           runtime_memory_json_sha256(sample.legacy_record)
       AND sample.v2_record_hash IS NOT DISTINCT FROM
           runtime_memory_json_sha256(sample.v2_record)
       AND sample.comparison = CASE
           WHEN sample.legacy_record IS NULL THEN 'legacy_missing'
           WHEN sample.v2_record IS NULL THEN 'v2_missing'
           WHEN sample.legacy_record = sample.v2_record THEN 'match'
           ELSE 'mismatch'
       END
       AND sample.selected_source = CASE sample.runtime_memory_contract
           WHEN 'dual_write_legacy_read' THEN 'legacy'
           WHEN 'dual_write_v2_preferred' THEN 'v2'
       END
       AND sample.selected_record IS NOT DISTINCT FROM CASE sample.runtime_memory_contract
           WHEN 'dual_write_legacy_read' THEN sample.legacy_record
           WHEN 'dual_write_v2_preferred' THEN sample.v2_record
       END
       AND sample.selected_record_hash IS NOT DISTINCT FROM
           runtime_memory_json_sha256(sample.selected_record)
      FROM runtime_memory_shadow_samples AS sample
      JOIN runtime_memory_rollout_admissions AS admission
        ON admission.admission_seq = sample.admission_seq
     WHERE sample.sample_seq = requested_sample_seq;
$$ LANGUAGE sql STABLE;

CREATE FUNCTION runtime_memory_rollout_cohort_gate(
    cohort_contract TEXT,
    cohort_rank SMALLINT,
    cohort_cutoff BIGINT
)
RETURNS TABLE(
    admission_count BIGINT,
    sample_count BIGINT,
    ready BOOLEAN,
    reason TEXT,
    aggregate_digest TEXT
) AS $$
DECLARE
    admissions BIGINT;
    samples BIGINT;
    exact_identities BIGINT;
    sampled_admissions BIGINT;
    invalid_samples BIGINT;
    retained_mismatches BIGINT;
    current_latest BIGINT;
    digest_value TEXT;
BEGIN
    SELECT COUNT(*)
      INTO admissions
      FROM runtime_memory_rollout_admissions AS admission
     WHERE admission.runtime_memory_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff;
    IF admissions = 0 THEN
        RETURN QUERY SELECT 0::BIGINT,0::BIGINT,FALSE,
            'runtime_shadow_cohort_empty'::TEXT,NULL::TEXT;
        RETURN;
    END IF;

    SELECT COUNT(*)
      INTO exact_identities
      FROM runtime_memory_rollout_admissions AS admission
      JOIN operation_state AS operation
        ON operation.operation_id = admission.operation_id
       AND operation.runtime_memory_contract = admission.runtime_memory_contract
      JOIN stage_worker_runs AS worker
        ON worker.id = admission.worker_run_id
       AND worker.operation_id = admission.operation_id
       AND worker.stage_execution_id = admission.stage_execution_id
       AND worker.stage_run_unit_id = admission.stage_run_unit_id
       AND worker.organization_id = admission.organization_id
     WHERE admission.runtime_memory_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff;
    IF exact_identities <> admissions THEN
        RETURN QUERY SELECT admissions,0::BIGINT,FALSE,
            'runtime_shadow_identity_drift'::TEXT,NULL::TEXT;
        RETURN;
    END IF;

    SELECT COUNT(*),COUNT(DISTINCT sample.admission_seq),
           COUNT(*) FILTER (
               WHERE NOT runtime_memory_shadow_sample_is_valid(sample.sample_seq)
           ),
           COUNT(*) FILTER (WHERE sample.comparison <> 'match')
      INTO samples,sampled_admissions,invalid_samples,retained_mismatches
      FROM runtime_memory_rollout_admissions AS admission
 LEFT JOIN runtime_memory_shadow_samples AS sample
        ON sample.admission_seq = admission.admission_seq
     WHERE admission.runtime_memory_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff;
    -- LEFT JOIN contributes one null row per missing admission.
    samples := COALESCE((
        SELECT COUNT(*)
          FROM runtime_memory_shadow_samples AS sample
          JOIN runtime_memory_rollout_admissions AS admission
            ON admission.admission_seq = sample.admission_seq
         WHERE admission.runtime_memory_contract = cohort_contract
           AND admission.rollout_rank = cohort_rank
           AND admission.admission_seq <= cohort_cutoff
    ),0);
    IF sampled_admissions <> admissions THEN
        RETURN QUERY SELECT admissions,samples,FALSE,
            'runtime_shadow_sample_missing'::TEXT,NULL::TEXT;
        RETURN;
    END IF;
    IF invalid_samples <> 0 THEN
        RETURN QUERY SELECT admissions,samples,FALSE,
            'runtime_shadow_sample_invalid'::TEXT,NULL::TEXT;
        RETURN;
    END IF;
    IF retained_mismatches <> 0 THEN
        RETURN QUERY SELECT admissions,samples,FALSE,
            'runtime_shadow_retained_mismatch'::TEXT,NULL::TEXT;
        RETURN;
    END IF;

    SELECT COUNT(*)
      INTO current_latest
      FROM runtime_memory_rollout_admissions AS admission
      JOIN LATERAL (
          SELECT sample.*
            FROM runtime_memory_shadow_samples AS sample
           WHERE sample.admission_seq = admission.admission_seq
           ORDER BY sample.sample_seq DESC
           LIMIT 1
      ) AS latest ON TRUE
     WHERE admission.runtime_memory_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff
       AND latest.comparison = 'match'
       AND latest.v2_record IS NOT DISTINCT FROM
           runtime_memory_v2_worker_record(admission.worker_run_id)
       AND latest.legacy_record IS NOT DISTINCT FROM
           runtime_memory_legacy_worker_record(admission.worker_run_id);
    IF current_latest <> admissions THEN
        RETURN QUERY SELECT admissions,samples,FALSE,
            'runtime_shadow_latest_sample_stale'::TEXT,NULL::TEXT;
        RETURN;
    END IF;

    SELECT runtime_memory_json_sha256(
        jsonb_agg(
            jsonb_build_object(
                'admission_seq', admission.admission_seq,
                'worker_run_id', admission.worker_run_id,
                'sample_seq', latest.sample_seq,
                'legacy_record_hash', latest.legacy_record_hash,
                'v2_record_hash', latest.v2_record_hash,
                'selected_source', latest.selected_source,
                'selected_record_hash', latest.selected_record_hash
            ) ORDER BY admission.admission_seq
        )
    )
      INTO digest_value
      FROM runtime_memory_rollout_admissions AS admission
      JOIN LATERAL (
          SELECT sample.*
            FROM runtime_memory_shadow_samples AS sample
           WHERE sample.admission_seq = admission.admission_seq
           ORDER BY sample.sample_seq DESC
           LIMIT 1
      ) AS latest ON TRUE
     WHERE admission.runtime_memory_contract = cohort_contract
       AND admission.rollout_rank = cohort_rank
       AND admission.admission_seq <= cohort_cutoff;
    RETURN QUERY SELECT admissions,samples,TRUE,'ready'::TEXT,digest_value;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION execution_rollout_pair_is_compatible(
    runtime_rank SMALLINT,
    attack_rank SMALLINT
)
RETURNS BOOLEAN AS $$
    SELECT attack_rank = 0
        OR (attack_rank IN (1,2) AND runtime_rank >= 1)
        OR (attack_rank = 3 AND runtime_rank = 3);
$$ LANGUAGE sql IMMUTABLE;

CREATE FUNCTION enforce_runtime_memory_attested_promotion()
RETURNS trigger AS $$
DECLARE
    cutoff BIGINT;
    gate_admission_count BIGINT;
    gate_sample_count BIGINT;
    gate_ready BOOLEAN;
    gate_reason TEXT;
    gate_digest TEXT;
    current_attack_rank SMALLINT;
BEGIN
    SELECT attack.rank
      INTO current_attack_rank
      FROM attack_execution_rollout AS attack
     WHERE attack.singleton = TRUE
     FOR SHARE;
    IF NOT execution_rollout_pair_is_compatible(NEW.contract_rank,current_attack_rank) THEN
        RAISE EXCEPTION 'RUNTIME_ATTACK_ROLLOUT_INCOMPATIBLE'
            USING ERRCODE = '55000';
    END IF;

    IF OLD.contract_rank IN (1,2) THEN
        SELECT MAX(admission.admission_seq)
          INTO cutoff
          FROM runtime_memory_rollout_admissions AS admission
         WHERE admission.runtime_memory_contract = OLD.contract
           AND admission.rollout_rank = OLD.contract_rank;
        IF cutoff IS NULL THEN
            RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_NOT_READY: runtime_shadow_cohort_empty'
                USING ERRCODE = '55000';
        END IF;
        SELECT gate.admission_count,
               gate.sample_count,
               gate.ready,
               gate.reason,
               gate.aggregate_digest
          INTO gate_admission_count,
               gate_sample_count,
               gate_ready,
               gate_reason,
               gate_digest
          FROM runtime_memory_rollout_cohort_gate(
              OLD.contract,
              OLD.contract_rank,
              cutoff
          ) AS gate;
        IF NOT gate_ready THEN
            RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_NOT_READY: %', gate_reason
                USING ERRCODE = '55000';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_runtime_memory_rollout_attestation_gate
BEFORE UPDATE ON runtime_memory_rollout
FOR EACH ROW EXECUTE FUNCTION enforce_runtime_memory_attested_promotion();

CREATE FUNCTION prepare_runtime_memory_rollout_promotion_receipt()
RETURNS trigger AS $$
DECLARE
    rollout runtime_memory_rollout%ROWTYPE;
    derived_from_rank SMALLINT;
    derived_from_contract TEXT;
    cutoff BIGINT;
    gate_admission_count BIGINT;
    gate_sample_count BIGINT;
    gate_ready BOOLEAN;
    gate_reason TEXT;
    gate_digest TEXT;
BEGIN
    SELECT current_rollout.*
      INTO rollout
      FROM runtime_memory_rollout AS current_rollout
     WHERE current_rollout.singleton_id = 1
     FOR SHARE;
    IF NOT FOUND OR rollout.contract_rank NOT IN (2,3) THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_RECEIPT_TRANSITION_MISSING'
            USING ERRCODE = '55000';
    END IF;

    derived_from_rank := rollout.contract_rank - 1;
    derived_from_contract := CASE derived_from_rank
        WHEN 1 THEN 'dual_write_legacy_read'
        WHEN 2 THEN 'dual_write_v2_preferred'
    END;
    IF rollout.contract IS DISTINCT FROM (CASE rollout.contract_rank
        WHEN 2 THEN 'dual_write_v2_preferred'
        WHEN 3 THEN 'v2_only'
    END) THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_RECEIPT_TRANSITION_MISSING'
            USING ERRCODE = '55000';
    END IF;

    SELECT MAX(admission.admission_seq)
      INTO cutoff
      FROM runtime_memory_rollout_admissions AS admission
     WHERE admission.runtime_memory_contract = derived_from_contract
       AND admission.rollout_rank = derived_from_rank;
    IF cutoff IS NULL THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_RECEIPT_COHORT_EMPTY'
            USING ERRCODE = '55000';
    END IF;
    SELECT gate.admission_count,
           gate.sample_count,
           gate.ready,
           gate.reason,
           gate.aggregate_digest
      INTO gate_admission_count,
           gate_sample_count,
           gate_ready,
           gate_reason,
           gate_digest
      FROM runtime_memory_rollout_cohort_gate(
          derived_from_contract,
          derived_from_rank,
          cutoff
      ) AS gate;
    IF NOT gate_ready THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_RECEIPT_NOT_READY: %', gate_reason
            USING ERRCODE = '55000';
    END IF;

    NEW.from_rank := derived_from_rank;
    NEW.to_rank := rollout.contract_rank;
    NEW.from_contract := derived_from_contract;
    NEW.to_contract := rollout.contract;
    NEW.from_row_version := rollout.row_version - 1;
    NEW.to_row_version := rollout.row_version;
    NEW.admission_cutoff := cutoff;
    NEW.admission_count := gate_admission_count;
    NEW.sample_count := gate_sample_count;
    NEW.aggregate_digest := gate_digest;
    NEW.promoted_at := NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER aa_runtime_memory_rollout_promotion_receipt_prepare
BEFORE INSERT ON runtime_memory_rollout_promotions
FOR EACH ROW EXECUTE FUNCTION prepare_runtime_memory_rollout_promotion_receipt();

CREATE FUNCTION record_runtime_memory_rollout_promotion_receipt()
RETURNS trigger AS $$
BEGIN
    IF OLD.contract_rank IN (1,2) THEN
        -- The receipt prepare trigger reconstructs every field from the
        -- already-updated singleton and retained old-contract cohort.
        INSERT INTO runtime_memory_rollout_promotions(from_rank)
        VALUES (OLD.contract_rank);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_runtime_memory_rollout_promotion_receipt
AFTER UPDATE ON runtime_memory_rollout
FOR EACH ROW EXECUTE FUNCTION record_runtime_memory_rollout_promotion_receipt();

CREATE FUNCTION enforce_attack_runtime_rollout_compatibility()
RETURNS trigger AS $$
DECLARE
    current_runtime_rank SMALLINT;
BEGIN
    SELECT runtime.contract_rank
      INTO current_runtime_rank
      FROM runtime_memory_rollout AS runtime
     WHERE runtime.singleton_id = 1
     FOR SHARE;
    IF NOT execution_rollout_pair_is_compatible(current_runtime_rank,NEW.rank) THEN
        RAISE EXCEPTION 'ATTACK_RUNTIME_ROLLOUT_INCOMPATIBLE'
            USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_attack_runtime_rollout_compatibility
BEFORE UPDATE ON attack_execution_rollout
FOR EACH ROW EXECUTE FUNCTION enforce_attack_runtime_rollout_compatibility();

CREATE FUNCTION reject_direct_runtime_memory_rollout_promotion_receipt()
RETURNS trigger AS $$
BEGIN
    IF TG_OP IN ('UPDATE','DELETE') OR pg_trigger_depth() = 1 THEN
        RAISE EXCEPTION 'RUNTIME_MEMORY_ROLLOUT_RECEIPT_IMMUTABLE'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER runtime_memory_rollout_promotion_receipt_immutable
BEFORE INSERT OR UPDATE OR DELETE ON runtime_memory_rollout_promotions
FOR EACH ROW EXECUTE FUNCTION reject_direct_runtime_memory_rollout_promotion_receipt();
