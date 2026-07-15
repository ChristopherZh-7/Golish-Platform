-- Candidate verification recovery protocol.
--
-- This is an additive bridge from the legacy submitted-Attempt terminalizer to
-- a recoverable intent -> finished tool -> checkpoint barrier -> terminal
-- receipt protocol. Legacy rows remain readable and legacy writers may still
-- omit action authorization receipts, but every new terminal intent is closed
-- over an exact receipt-backed action journal and an exact Worker checkpoint.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION candidate_recovery_sha256_text(value TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT 'sha256:' || encode(digest(convert_to(value, 'UTF8'), 'sha256'), 'hex')
$$;

CREATE FUNCTION candidate_recovery_sha256_json(value JSONB)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT candidate_recovery_sha256_text(value::TEXT)
$$;

-- -------------------------------------------------------------------------
-- Approval start authority and explicit pending-terminalization state
-- -------------------------------------------------------------------------

ALTER TABLE attack_candidate_approvals
    ADD COLUMN start_before TIMESTAMPTZ;

-- The pre-existing immutable-decision trigger correctly rejects ordinary
-- decision mutation. Disable only that trigger for this one migration-owned
-- compatibility backfill; every other authority/fuel trigger remains enabled.
ALTER TABLE attack_candidate_approvals
    DISABLE TRIGGER attack_candidate_approvals_decision_immutable;

UPDATE attack_candidate_approvals
   SET start_before = expires_at
 WHERE start_before IS NULL;

ALTER TABLE attack_candidate_approvals
    ENABLE TRIGGER attack_candidate_approvals_decision_immutable;

ALTER TABLE attack_candidate_approvals
    ALTER COLUMN start_before SET NOT NULL,
    ADD CONSTRAINT attack_candidate_approvals_start_before_shape
        CHECK (start_before <= expires_at);

CREATE FUNCTION fill_candidate_approval_start_before()
RETURNS trigger AS $$
BEGIN
    IF NEW.start_before IS NULL THEN
        NEW.start_before := NEW.expires_at;
    END IF;
    IF NEW.start_before > NEW.expires_at THEN
        RAISE EXCEPTION 'CANDIDATE_APPROVAL_START_AFTER_EXPIRY'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_approvals_start_before_default
BEFORE INSERT ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION fill_candidate_approval_start_before();

ALTER TABLE candidate_attempts
    DROP CONSTRAINT candidate_attempts_status_check,
    ADD CONSTRAINT candidate_attempts_status_check CHECK (
        status IN (
            'queued',
            'running',
            'submitted',
            'terminalization_pending',
            'verified',
            'refuted',
            'blocked',
            'retryable_failed',
            'abandoned'
        )
    );

DROP INDEX candidate_attempts_one_live_per_candidate;
CREATE UNIQUE INDEX candidate_attempts_one_live_per_candidate
    ON candidate_attempts(candidate_id)
    WHERE status IN ('queued', 'running', 'submitted', 'terminalization_pending');

CREATE OR REPLACE FUNCTION enforce_candidate_attempt_audit_transition()
RETURNS trigger AS $$
DECLARE
    old_is_terminal BOOLEAN;
    new_is_terminal BOOLEAN;
    target_pointer_change_allowed BOOLEAN;
BEGIN
    new_is_terminal := NEW.status IN (
        'verified','refuted','blocked','retryable_failed','abandoned'
    );
    IF TG_OP = 'INSERT' THEN
        IF new_is_terminal THEN
            NEW.terminal_at := NOW();
        END IF;
        RETURN NEW;
    END IF;
    old_is_terminal := OLD.status IN (
        'verified','refuted','blocked','retryable_failed','abandoned'
    );
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF NOT target_pointer_change_allowed
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR (OLD.result_json IS NOT NULL AND (
            NEW.result_json IS DISTINCT FROM OLD.result_json
            OR NEW.result_hash IS DISTINCT FROM OLD.result_hash
        ))
    THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_RESULT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF old_is_terminal THEN
        IF (to_jsonb(NEW) - ARRAY['target_live_id']::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY['target_live_id']::TEXT[])
        THEN
            RAISE EXCEPTION 'CANDIDATE_ATTEMPT_TERMINAL_AUDIT_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.status IS DISTINCT FROM OLD.status AND NOT (
        (OLD.status = 'queued' AND NEW.status IN ('running','abandoned'))
        OR (OLD.status = 'running' AND NEW.status IN (
            'running','submitted','terminalization_pending',
            'blocked','retryable_failed','abandoned'
        ))
        OR (OLD.status = 'submitted' AND NEW.status IN ('verified','refuted','blocked'))
        OR (
            OLD.status = 'terminalization_pending'
            AND NEW.status IN ('verified','refuted','blocked')
        )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_STATUS_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.result_json IS NOT NULL AND OLD.result_json IS NULL
        AND NOT (
            (OLD.status = 'running' AND NEW.status IN (
                'submitted','blocked','retryable_failed'
            ))
            OR (
                OLD.status = 'terminalization_pending'
                AND NEW.status IN ('verified','refuted','blocked')
            )
        )
    THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_RESULT_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.status = 'terminalization_pending'
        AND (NEW.result_json IS NOT NULL OR NEW.result_hash IS NOT NULL)
    THEN
        RAISE EXCEPTION 'CANDIDATE_PENDING_RESULT_MUST_REMAIN_IN_INTENT'
            USING ERRCODE = '23514';
    END IF;
    IF new_is_terminal AND NOT old_is_terminal THEN
        NEW.terminal_at := NOW();
    ELSIF NOT new_is_terminal AND NEW.terminal_at IS NOT NULL THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_TERMINAL_TIME_INVALID'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

ALTER TABLE candidate_attempt_actions
    ADD CONSTRAINT candidate_attempt_actions_id_attempt_unique
        UNIQUE (id, attempt_id);

ALTER TABLE candidate_attempts
    ADD CONSTRAINT candidate_attempts_recovery_identity_unique UNIQUE (
        id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        stage_worker_run_id
    );

-- -------------------------------------------------------------------------
-- Immutable action-start authorization receipt
-- -------------------------------------------------------------------------

CREATE TABLE candidate_action_authorization_receipts (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (request_id = BTRIM(request_id) AND request_id <> ''),
    attempt_id UUID NOT NULL,
    action_id UUID NOT NULL,
    approval_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    worker_run_id UUID NOT NULL,
    lease_token UUID NOT NULL,
    attempt_epoch BIGINT NOT NULL CHECK (attempt_epoch >= 0),
    decision_version BIGINT NOT NULL CHECK (decision_version > 0),
    scope_hash TEXT NOT NULL CHECK (BTRIM(scope_hash) <> ''),
    canonical_args_hash TEXT NOT NULL CHECK (
        canonical_args_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    budget_hash TEXT NOT NULL CHECK (budget_hash ~ '^sha256:[0-9a-f]{64}$'),
    authorized_at TIMESTAMPTZ NOT NULL,
    start_before TIMESTAMPTZ NOT NULL,
    execution_deadline TIMESTAMPTZ NOT NULL,
    receipt_hash TEXT NOT NULL UNIQUE CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (action_id),
    UNIQUE (id, action_id, attempt_id),
    FOREIGN KEY (action_id, attempt_id)
        REFERENCES candidate_attempt_actions(id, attempt_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        attempt_id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        worker_run_id
    ) REFERENCES candidate_attempts(
        id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        stage_worker_run_id
    ) ON DELETE RESTRICT,
    -- The insert trigger proves the exact live (worker, lease_token) fence.
    -- Keep the token immutable as audit evidence, but do not make it an
    -- ongoing FK: terminalization must be able to clear the Worker lease.
    FOREIGN KEY (worker_run_id)
        REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    CHECK (authorized_at <= start_before),
    CHECK (execution_deadline > authorized_at),
    CHECK (created_at = authorized_at)
);

CREATE FUNCTION derive_candidate_action_authorization_receipt()
RETURNS trigger AS $$
DECLARE
    attempt_row candidate_attempts%ROWTYPE;
    approval_row attack_candidate_approvals%ROWTYPE;
    action_row candidate_attempt_actions%ROWTYPE;
    worker_row stage_worker_runs%ROWTYPE;
    frozen_scope_hash TEXT;
BEGIN
    SELECT * INTO attempt_row
      FROM candidate_attempts
     WHERE id = NEW.attempt_id
     FOR SHARE;
    IF NOT FOUND OR attempt_row.status <> 'running'
        OR attempt_row.stage_worker_run_id IS NULL
    THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_AUTH_ATTEMPT_NOT_RUNNING'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO action_row
      FROM candidate_attempt_actions
     WHERE id = NEW.action_id AND attempt_id = NEW.attempt_id
     FOR SHARE;
    IF NOT FOUND OR action_row.status <> 'planned' THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_AUTH_ACTION_NOT_PLANNED'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO approval_row
      FROM attack_candidate_approvals
     WHERE id = attempt_row.approval_id
       AND candidate_id = attempt_row.candidate_id
       AND operation_id = attempt_row.operation_id
       AND scope_snapshot_id = attempt_row.scope_snapshot_id
       AND wave_run_id = attempt_row.wave_run_id
       AND wave_unit_id = attempt_row.wave_unit_id
       AND organization_id = attempt_row.organization_id
       AND target_identity_hash = attempt_row.target_identity_hash
       AND candidate_plan_hash = attempt_row.candidate_plan_hash
     FOR SHARE;
    IF NOT FOUND OR approval_row.status <> 'approved'
        OR clock_timestamp() >= approval_row.start_before
    THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_START_AUTHORITY_EXPIRED'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO worker_row
      FROM stage_worker_runs
     WHERE id = attempt_row.stage_worker_run_id
       AND lease_token = NEW.lease_token
       AND operation_id = attempt_row.operation_id
       AND organization_id = attempt_row.organization_id
     FOR SHARE;
    IF NOT FOUND OR worker_row.status <> 'running'
        OR worker_row.attempt_epoch < 0
        OR worker_row.lease_expires_at <= clock_timestamp()
    THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_START_WORKER_FENCE_INVALID'
            USING ERRCODE = '23514';
    END IF;

    SELECT scope_hash INTO frozen_scope_hash
      FROM operation_org_scope_snapshots
     WHERE id = attempt_row.scope_snapshot_id
       AND operation_id = attempt_row.operation_id
       AND sealed_at IS NOT NULL;
    IF frozen_scope_hash IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_START_SCOPE_NOT_FROZEN'
            USING ERRCODE = '23514';
    END IF;

    NEW.approval_id := approval_row.id;
    NEW.candidate_id := attempt_row.candidate_id;
    NEW.operation_id := attempt_row.operation_id;
    NEW.scope_snapshot_id := attempt_row.scope_snapshot_id;
    NEW.wave_run_id := attempt_row.wave_run_id;
    NEW.wave_unit_id := attempt_row.wave_unit_id;
    NEW.organization_id := attempt_row.organization_id;
    NEW.target_identity_hash := attempt_row.target_identity_hash;
    NEW.candidate_plan_hash := attempt_row.candidate_plan_hash;
    NEW.worker_run_id := worker_row.id;
    NEW.attempt_epoch := worker_row.attempt_epoch;
    NEW.decision_version := approval_row.decision_version;
    NEW.scope_hash := frozen_scope_hash;
    NEW.canonical_args_hash := candidate_recovery_sha256_json(action_row.canonical_args);
    NEW.budget_hash := candidate_recovery_sha256_json(approval_row.budget);
    NEW.authorized_at := clock_timestamp();
    NEW.start_before := approval_row.start_before;
    NEW.created_at := NEW.authorized_at;
    IF NEW.execution_deadline IS NULL
        OR NEW.execution_deadline <= NEW.authorized_at
    THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_EXECUTION_DEADLINE_INVALID'
            USING ERRCODE = '23514';
    END IF;
    NEW.receipt_hash := candidate_recovery_sha256_json(jsonb_build_object(
        'schema_version', 1,
        'request_id', NEW.request_id,
        'attempt_id', NEW.attempt_id,
        'action_id', NEW.action_id,
        'approval_id', NEW.approval_id,
        'candidate_id', NEW.candidate_id,
        'operation_id', NEW.operation_id,
        'scope_snapshot_id', NEW.scope_snapshot_id,
        'wave_run_id', NEW.wave_run_id,
        'wave_unit_id', NEW.wave_unit_id,
        'organization_id', NEW.organization_id,
        'target_identity_hash', NEW.target_identity_hash,
        'candidate_plan_hash', NEW.candidate_plan_hash,
        'worker_run_id', NEW.worker_run_id,
        'lease_token', NEW.lease_token,
        'attempt_epoch', NEW.attempt_epoch,
        'decision_version', NEW.decision_version,
        'scope_hash', NEW.scope_hash,
        'canonical_args_hash', NEW.canonical_args_hash,
        'budget_hash', NEW.budget_hash,
        'authorized_at', NEW.authorized_at,
        'start_before', NEW.start_before,
        'execution_deadline', NEW.execution_deadline
    ));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_action_authorization_receipts_derive
BEFORE INSERT ON candidate_action_authorization_receipts
FOR EACH ROW EXECUTE FUNCTION derive_candidate_action_authorization_receipt();

CREATE FUNCTION reject_candidate_recovery_immutable_row()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'CANDIDATE_RECOVERY_IMMUTABLE'
        USING ERRCODE = '23514';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_action_authorization_receipts_immutable
BEFORE UPDATE OR DELETE ON candidate_action_authorization_receipts
FOR EACH ROW EXECUTE FUNCTION reject_candidate_recovery_immutable_row();

ALTER TABLE candidate_attempt_actions
    ADD COLUMN authorization_receipt_id UUID,
    ADD CONSTRAINT candidate_attempt_actions_authorization_receipt_fk
        FOREIGN KEY (authorization_receipt_id, id, attempt_id)
        REFERENCES candidate_action_authorization_receipts(id, action_id, attempt_id)
        DEFERRABLE INITIALLY DEFERRED;

-- Once an intent exists the action journal is frozen. Until cutover, legacy
-- action rows without a receipt remain accepted; the new intent trigger below
-- requires every action in its journal to carry the exact receipt.
CREATE OR REPLACE FUNCTION enforce_candidate_action_journal_audit()
RETURNS trigger AS $$
DECLARE
    owner_attempt_id UUID;
    owner_attempt_status TEXT;
BEGIN
    owner_attempt_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.attempt_id ELSE NEW.attempt_id END;
    SELECT status INTO owner_attempt_status
      FROM candidate_attempts
     WHERE id = owner_attempt_id
     FOR UPDATE;
    IF owner_attempt_status IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_ATTEMPT_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF EXISTS (
        SELECT 1 FROM candidate_attempt_terminal_intents
         WHERE attempt_id = owner_attempt_id
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_AFTER_TERMINAL_INTENT'
            USING ERRCODE = '23514';
    END IF;
    IF owner_attempt_status <> 'running' THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_TERMINAL_AUDIT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' AND (
        NEW.attempt_id IS DISTINCT FROM OLD.attempt_id
        OR NEW.action_ordinal IS DISTINCT FROM OLD.action_ordinal
        OR NEW.capability_id IS DISTINCT FROM OLD.capability_id
        OR NEW.action_kind IS DISTINCT FROM OLD.action_kind
        OR NEW.canonical_args IS DISTINCT FROM OLD.canonical_args
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR (
            OLD.authorization_receipt_id IS NOT NULL
            AND NEW.authorization_receipt_id IS DISTINCT FROM OLD.authorization_receipt_id
        )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_IDENTITY_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' AND NEW.status IS DISTINCT FROM OLD.status AND NOT (
        (OLD.status = 'planned' AND NEW.status = 'started')
        OR (OLD.status = 'started' AND NEW.status IN (
            'completed','failed','outcome_unknown'
        ))
        OR (
            OLD.status = 'outcome_unknown'
            AND NEW.status = 'failed'
            AND EXISTS (
                SELECT 1 FROM candidate_recovery_cases
                 WHERE action_id = OLD.id
                   AND attempt_id = OLD.attempt_id
                   AND status = 'decision_recorded'
                   AND resolution_kind = 'terminalize_blocked_outcome_unknown'
            )
        )
        OR (
            OLD.status = 'outcome_unknown'
            AND NEW.status = 'completed'
            AND EXISTS (
                SELECT 1 FROM candidate_recovery_cases
                 WHERE action_id = OLD.id
                   AND attempt_id = OLD.attempt_id
                   AND status = 'decision_recorded'
                   AND resolution_kind = 'accept_external_result_with_exact_evidence'
            )
        )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_STATUS_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

-- -------------------------------------------------------------------------
-- Immutable terminal intent
-- -------------------------------------------------------------------------

CREATE TABLE candidate_attempt_terminal_intents (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (request_id = BTRIM(request_id) AND request_id <> ''),
    attempt_id UUID NOT NULL UNIQUE,
    approval_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    worker_run_id UUID NOT NULL,
    attempt_epoch BIGINT NOT NULL CHECK (attempt_epoch >= 0),
    lease_token UUID NOT NULL,
    tool_call_record_id UUID NOT NULL UNIQUE,
    disposition TEXT NOT NULL CHECK (disposition IN ('verified','refuted','blocked')),
    submitted_result JSONB NOT NULL CHECK (jsonb_typeof(submitted_result) = 'object'),
    result_hash TEXT NOT NULL CHECK (result_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_manifest_hash TEXT NOT NULL CHECK (
        evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    evidence_count INTEGER NOT NULL CHECK (evidence_count >= 0),
    tool_result_text TEXT NOT NULL CHECK (tool_result_text <> ''),
    tool_result_hash TEXT NOT NULL CHECK (tool_result_hash ~ '^sha256:[0-9a-f]{64}$'),
    intent_hash TEXT NOT NULL UNIQUE CHECK (intent_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, attempt_id),
    UNIQUE (id, attempt_id, worker_run_id, tool_call_record_id),
    UNIQUE (
        id,
        attempt_id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        worker_run_id,
        tool_call_record_id
    ),
    FOREIGN KEY (
        attempt_id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        worker_run_id
    ) REFERENCES candidate_attempts(
        id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        stage_worker_run_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (tool_call_record_id, worker_run_id, attempt_epoch, lease_token)
        REFERENCES tool_calls(id, worker_run_id, attempt_epoch, lease_token)
        ON DELETE RESTRICT
);

CREATE FUNCTION derive_candidate_attempt_terminal_intent()
RETURNS trigger AS $$
DECLARE
    attempt_row candidate_attempts%ROWTYPE;
    worker_row stage_worker_runs%ROWTYPE;
    tool_row tool_calls%ROWTYPE;
    evidence_manifest JSONB;
    nonterminal_action_count BIGINT;
    unreceipted_action_count BIGINT;
BEGIN
    SELECT * INTO attempt_row
      FROM candidate_attempts
     WHERE id = NEW.attempt_id
     FOR SHARE;
    IF NOT FOUND OR attempt_row.status <> 'running'
        OR attempt_row.stage_worker_run_id IS NULL
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_ATTEMPT_NOT_RUNNING'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO worker_row
      FROM stage_worker_runs
     WHERE id = attempt_row.stage_worker_run_id
       AND lease_token = NEW.lease_token
       AND operation_id = attempt_row.operation_id
       AND organization_id = attempt_row.organization_id
     FOR SHARE;
    IF NOT FOUND OR worker_row.status <> 'running'
        OR worker_row.lease_expires_at <= clock_timestamp()
        OR worker_row.active_tool_call_id IS DISTINCT FROM NEW.tool_call_record_id
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_WORKER_FENCE_INVALID'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO tool_row
      FROM tool_calls
     WHERE id = NEW.tool_call_record_id
       AND worker_run_id = worker_row.id
       AND operation_id = worker_row.operation_id
       AND stage_execution_id = worker_row.stage_execution_id
       AND stage_run_unit_id = worker_row.stage_run_unit_id
       AND organization_id = worker_row.organization_id
       AND attempt_epoch = worker_row.attempt_epoch
       AND lease_token = NEW.lease_token
     FOR SHARE;
    IF NOT FOUND OR tool_row.name <> 'submit_candidate_attempt'
        OR tool_row.status NOT IN ('received','running')
        OR tool_row.result IS NOT NULL
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_TOOL_FENCE_INVALID'
            USING ERRCODE = '23514';
    END IF;

    SELECT
        COUNT(*) FILTER (WHERE status IN ('planned','started','outcome_unknown')),
        COUNT(*) FILTER (WHERE authorization_receipt_id IS NULL)
      INTO nonterminal_action_count, unreceipted_action_count
      FROM candidate_attempt_actions
     WHERE attempt_id = NEW.attempt_id;
    IF nonterminal_action_count <> 0 OR unreceipted_action_count <> 0 THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_ACTION_JOURNAL_OPEN'
            USING ERRCODE = '23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM candidate_attempt_actions WHERE attempt_id = NEW.attempt_id
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_ACTION_JOURNAL_EMPTY'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.submitted_result ->> 'disposition' IS DISTINCT FROM NEW.disposition THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_DISPOSITION_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM attack_candidates
         WHERE candidate_id = attempt_row.candidate_id
           AND operation_uuid = attempt_row.operation_id
           AND scope_snapshot_id = attempt_row.scope_snapshot_id
           AND wave_run_id = attempt_row.wave_run_id
           AND wave_unit_id = attempt_row.wave_unit_id
           AND organization_id = attempt_row.organization_id
           AND target_identity_hash = attempt_row.target_identity_hash
           AND candidate_plan_hash = attempt_row.candidate_plan_hash
           AND disposition = 'approved'
           AND terminal_attempt_id IS NULL
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_CANDIDATE_NOT_APPROVED'
            USING ERRCODE = '23514';
    END IF;

    SELECT COALESCE(jsonb_agg(
               jsonb_build_object('evidence_id',evidence_id,'role',role)
               ORDER BY evidence_id,role
           ), '[]'::JSONB)
      INTO evidence_manifest
      FROM candidate_attempt_evidence
     WHERE attempt_id = NEW.attempt_id;

    NEW.approval_id := attempt_row.approval_id;
    NEW.candidate_id := attempt_row.candidate_id;
    NEW.operation_id := attempt_row.operation_id;
    NEW.scope_snapshot_id := attempt_row.scope_snapshot_id;
    NEW.wave_run_id := attempt_row.wave_run_id;
    NEW.wave_unit_id := attempt_row.wave_unit_id;
    NEW.organization_id := attempt_row.organization_id;
    NEW.target_identity_hash := attempt_row.target_identity_hash;
    NEW.candidate_plan_hash := attempt_row.candidate_plan_hash;
    NEW.worker_run_id := worker_row.id;
    NEW.attempt_epoch := worker_row.attempt_epoch;
    NEW.result_hash := candidate_recovery_sha256_json(NEW.submitted_result);
    NEW.evidence_manifest_hash := candidate_recovery_sha256_json(evidence_manifest);
    NEW.evidence_count := jsonb_array_length(evidence_manifest);
    NEW.tool_result_hash := candidate_recovery_sha256_text(NEW.tool_result_text);
    NEW.created_at := clock_timestamp();
    NEW.intent_hash := candidate_recovery_sha256_json(jsonb_build_object(
        'schema_version', 1,
        'request_id', NEW.request_id,
        'attempt_id', NEW.attempt_id,
        'approval_id', NEW.approval_id,
        'candidate_id', NEW.candidate_id,
        'operation_id', NEW.operation_id,
        'scope_snapshot_id', NEW.scope_snapshot_id,
        'wave_run_id', NEW.wave_run_id,
        'wave_unit_id', NEW.wave_unit_id,
        'organization_id', NEW.organization_id,
        'target_identity_hash', NEW.target_identity_hash,
        'candidate_plan_hash', NEW.candidate_plan_hash,
        'worker_run_id', NEW.worker_run_id,
        'attempt_epoch', NEW.attempt_epoch,
        'tool_call_record_id', NEW.tool_call_record_id,
        'disposition', NEW.disposition,
        'result_hash', NEW.result_hash,
        'evidence_manifest_hash', NEW.evidence_manifest_hash,
        'tool_result_hash', NEW.tool_result_hash
    ));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempt_terminal_intents_derive
BEFORE INSERT ON candidate_attempt_terminal_intents
FOR EACH ROW EXECUTE FUNCTION derive_candidate_attempt_terminal_intent();

CREATE TRIGGER candidate_attempt_terminal_intents_immutable
BEFORE UPDATE OR DELETE ON candidate_attempt_terminal_intents
FOR EACH ROW EXECUTE FUNCTION reject_candidate_recovery_immutable_row();

-- -------------------------------------------------------------------------
-- Exact finished-tool/checkpoint barrier
-- -------------------------------------------------------------------------

CREATE TABLE candidate_attempt_terminal_barriers (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (request_id = BTRIM(request_id) AND request_id <> ''),
    intent_id UUID NOT NULL UNIQUE,
    attempt_id UUID NOT NULL UNIQUE,
    worker_run_id UUID NOT NULL,
    tool_call_record_id UUID NOT NULL UNIQUE,
    message_chain_id UUID NOT NULL,
    attempt_epoch BIGINT NOT NULL CHECK (attempt_epoch >= 0),
    checkpoint_version BIGINT NOT NULL CHECK (checkpoint_version > 0),
    checkpoint_hash TEXT NOT NULL CHECK (checkpoint_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_result_hash TEXT NOT NULL CHECK (tool_result_hash ~ '^sha256:[0-9a-f]{64}$'),
    barrier_hash TEXT NOT NULL UNIQUE CHECK (barrier_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, intent_id, attempt_id, worker_run_id),
    UNIQUE (id, intent_id, attempt_id, worker_run_id, tool_call_record_id),
    FOREIGN KEY (
        intent_id,
        attempt_id,
        worker_run_id,
        tool_call_record_id
    ) REFERENCES candidate_attempt_terminal_intents(
        id,
        attempt_id,
        worker_run_id,
        tool_call_record_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (tool_call_record_id, worker_run_id)
        REFERENCES tool_calls(id, worker_run_id) ON DELETE RESTRICT,
    FOREIGN KEY (message_chain_id)
        REFERENCES message_chains(id) ON DELETE RESTRICT
);

CREATE FUNCTION derive_candidate_attempt_terminal_barrier()
RETURNS trigger AS $$
DECLARE
    intent_row candidate_attempt_terminal_intents%ROWTYPE;
    worker_row stage_worker_runs%ROWTYPE;
    tool_row tool_calls%ROWTYPE;
BEGIN
    SELECT * INTO intent_row
      FROM candidate_attempt_terminal_intents
     WHERE id = NEW.intent_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_BARRIER_INTENT_MISSING'
            USING ERRCODE = '23503';
    END IF;
    SELECT * INTO worker_row
      FROM stage_worker_runs
     WHERE id = intent_row.worker_run_id
       AND operation_id = intent_row.operation_id
       AND organization_id = intent_row.organization_id
     FOR SHARE;
    IF NOT FOUND OR worker_row.active_tool_call_id IS NOT NULL
        OR worker_row.message_chain_id IS NULL
        OR worker_row.checkpoint_version IS DISTINCT FROM NEW.checkpoint_version
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_BARRIER_WORKER_NOT_CHECKPOINTED'
            USING ERRCODE = '23514';
    END IF;
    SELECT * INTO tool_row
      FROM tool_calls
     WHERE id = intent_row.tool_call_record_id
       AND worker_run_id = intent_row.worker_run_id
       AND attempt_epoch = intent_row.attempt_epoch
       AND lease_token = intent_row.lease_token
     FOR SHARE;
    IF NOT FOUND OR tool_row.status <> 'finished'
        OR tool_row.result IS DISTINCT FROM intent_row.tool_result_text
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_BARRIER_TOOL_RESULT_NOT_DURABLE'
            USING ERRCODE = '23514';
    END IF;
    NEW.attempt_id := intent_row.attempt_id;
    NEW.worker_run_id := intent_row.worker_run_id;
    NEW.tool_call_record_id := intent_row.tool_call_record_id;
    NEW.message_chain_id := worker_row.message_chain_id;
    NEW.attempt_epoch := intent_row.attempt_epoch;
    NEW.checkpoint_hash := candidate_recovery_sha256_json(worker_row.checkpoint);
    NEW.tool_result_hash := intent_row.tool_result_hash;
    NEW.created_at := clock_timestamp();
    NEW.barrier_hash := candidate_recovery_sha256_json(jsonb_build_object(
        'schema_version', 1,
        'request_id', NEW.request_id,
        'intent_id', NEW.intent_id,
        'attempt_id', NEW.attempt_id,
        'worker_run_id', NEW.worker_run_id,
        'tool_call_record_id', NEW.tool_call_record_id,
        'message_chain_id', NEW.message_chain_id,
        'attempt_epoch', NEW.attempt_epoch,
        'checkpoint_version', NEW.checkpoint_version,
        'checkpoint_hash', NEW.checkpoint_hash,
        'tool_result_hash', NEW.tool_result_hash
    ));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempt_terminal_barriers_derive
BEFORE INSERT ON candidate_attempt_terminal_barriers
FOR EACH ROW EXECUTE FUNCTION derive_candidate_attempt_terminal_barrier();

CREATE TRIGGER candidate_attempt_terminal_barriers_immutable
BEFORE UPDATE OR DELETE ON candidate_attempt_terminal_barriers
FOR EACH ROW EXECUTE FUNCTION reject_candidate_recovery_immutable_row();

-- -------------------------------------------------------------------------
-- Server-authority terminal receipt. This row may only be inserted after the
-- existing compound terminalizer has committed all canonical truth in its
-- caller-owned transaction: Attempt/Candidate/Finding/FactDelta/outbox and
-- Worker/lane release.
-- -------------------------------------------------------------------------

CREATE TABLE candidate_attempt_terminal_receipts (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (request_id = BTRIM(request_id) AND request_id <> ''),
    intent_id UUID NOT NULL UNIQUE,
    barrier_id UUID NOT NULL UNIQUE,
    attempt_id UUID NOT NULL UNIQUE,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    worker_run_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('verified','refuted','blocked')),
    result_hash TEXT NOT NULL CHECK (result_hash ~ '^sha256:[0-9a-f]{64}$'),
    terminal_attempt_row_version BIGINT NOT NULL CHECK (terminal_attempt_row_version > 0),
    finding_id UUID REFERENCES findings(id) ON DELETE RESTRICT,
    fact_delta_count INTEGER NOT NULL CHECK (fact_delta_count >= 0),
    terminal_event_id UUID NOT NULL
        REFERENCES knowledge_outbox_events(event_id) ON DELETE RESTRICT,
    receipt_payload JSONB NOT NULL CHECK (jsonb_typeof(receipt_payload) = 'object'),
    receipt_hash TEXT NOT NULL UNIQUE CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, intent_id, barrier_id, attempt_id),
    FOREIGN KEY (barrier_id, intent_id, attempt_id, worker_run_id)
        REFERENCES candidate_attempt_terminal_barriers(
            id, intent_id, attempt_id, worker_run_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, attempt_id)
        REFERENCES candidate_attempt_terminal_intents(id, attempt_id)
        ON DELETE RESTRICT,
    CHECK ((disposition = 'verified') = (finding_id IS NOT NULL))
);

CREATE FUNCTION derive_candidate_attempt_terminal_receipt()
RETURNS trigger AS $$
DECLARE
    intent_row candidate_attempt_terminal_intents%ROWTYPE;
    barrier_row candidate_attempt_terminal_barriers%ROWTYPE;
    attempt_row candidate_attempts%ROWTYPE;
    candidate_disposition TEXT;
    candidate_terminal_attempt_id UUID;
    candidate_terminal_finding_id UUID;
    worker_row stage_worker_runs%ROWTYPE;
    event_row knowledge_outbox_events%ROWTYPE;
    actual_fact_delta_count BIGINT;
    delivery_count BIGINT;
BEGIN
    SELECT * INTO intent_row
      FROM candidate_attempt_terminal_intents
     WHERE id = NEW.intent_id
     FOR SHARE;
    SELECT * INTO barrier_row
      FROM candidate_attempt_terminal_barriers
     WHERE id = NEW.barrier_id AND intent_id = NEW.intent_id
     FOR SHARE;
    IF intent_row.id IS NULL OR barrier_row.id IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_BARRIER_MISSING'
            USING ERRCODE = '23503';
    END IF;
    SELECT * INTO attempt_row
      FROM candidate_attempts
     WHERE id = intent_row.attempt_id
     FOR SHARE;
    IF NOT FOUND OR attempt_row.status IS DISTINCT FROM intent_row.disposition
        OR attempt_row.result_hash IS DISTINCT FROM intent_row.result_hash
        OR attempt_row.result_json IS DISTINCT FROM intent_row.submitted_result
        OR attempt_row.terminal_at IS NULL
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_ATTEMPT_NOT_TERMINAL'
            USING ERRCODE = '23514';
    END IF;
    SELECT disposition,terminal_attempt_id,terminal_finding_id
      INTO candidate_disposition,candidate_terminal_attempt_id,candidate_terminal_finding_id
      FROM attack_candidates
     WHERE candidate_id = intent_row.candidate_id
       AND operation_uuid = intent_row.operation_id
       AND scope_snapshot_id = intent_row.scope_snapshot_id
       AND wave_run_id = intent_row.wave_run_id
       AND wave_unit_id = intent_row.wave_unit_id
       AND organization_id = intent_row.organization_id
       AND target_identity_hash = intent_row.target_identity_hash
       AND candidate_plan_hash = intent_row.candidate_plan_hash
     FOR SHARE;
    IF candidate_disposition IS DISTINCT FROM intent_row.disposition
        OR candidate_terminal_attempt_id IS DISTINCT FROM intent_row.attempt_id
        OR (
            intent_row.disposition = 'verified'
            AND candidate_terminal_finding_id IS NULL
        )
        OR (
            intent_row.disposition <> 'verified'
            AND candidate_terminal_finding_id IS NOT NULL
        )
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_CANDIDATE_NOT_TERMINAL'
            USING ERRCODE = '23514';
    END IF;
    SELECT * INTO worker_row
      FROM stage_worker_runs
     WHERE id = intent_row.worker_run_id
     FOR SHARE;
    IF NOT FOUND OR worker_row.status <> 'passed'
        OR worker_row.lease_token IS NOT NULL
        OR worker_row.active_tool_call_id IS NOT NULL
        OR worker_row.terminal_at IS NULL
        OR EXISTS (
            SELECT 1 FROM attack_execution_lanes
             WHERE stage_worker_run_id = worker_row.id
        )
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_WORKER_NOT_RELEASED'
            USING ERRCODE = '23514';
    END IF;
    SELECT * INTO event_row
      FROM knowledge_outbox_events
     WHERE event_name = 'CandidateAttemptTerminal.v1'
       AND source_operation_id = intent_row.operation_id
       AND source_kind = 'candidate_attempt'
       AND source_id_kind = 'uuid'
       AND source_id_value = intent_row.attempt_id::TEXT
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_EVENT_MISSING'
            USING ERRCODE = '23514';
    END IF;
    SELECT COUNT(*) INTO delivery_count
      FROM knowledge_projection_deliveries
     WHERE event_id = event_row.event_id;
    IF delivery_count <> 4 THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_DELIVERY_BUNDLE_INCOMPLETE'
            USING ERRCODE = '23514';
    END IF;
    SELECT COUNT(*) INTO actual_fact_delta_count
      FROM attack_fact_deltas
     WHERE source_attempt_id = intent_row.attempt_id;

    NEW.attempt_id := intent_row.attempt_id;
    NEW.candidate_id := intent_row.candidate_id;
    NEW.operation_id := intent_row.operation_id;
    NEW.scope_snapshot_id := intent_row.scope_snapshot_id;
    NEW.wave_run_id := intent_row.wave_run_id;
    NEW.wave_unit_id := intent_row.wave_unit_id;
    NEW.organization_id := intent_row.organization_id;
    NEW.worker_run_id := intent_row.worker_run_id;
    NEW.disposition := intent_row.disposition;
    NEW.result_hash := intent_row.result_hash;
    NEW.terminal_attempt_row_version := attempt_row.row_version;
    NEW.finding_id := candidate_terminal_finding_id;
    NEW.fact_delta_count := actual_fact_delta_count::INTEGER;
    NEW.terminal_event_id := event_row.event_id;
    NEW.created_at := clock_timestamp();
    NEW.receipt_payload := jsonb_build_object(
        'schema_version', 1,
        'request_id', NEW.request_id,
        'intent_id', NEW.intent_id,
        'barrier_id', NEW.barrier_id,
        'attempt_id', NEW.attempt_id,
        'candidate_id', NEW.candidate_id,
        'operation_id', NEW.operation_id,
        'scope_snapshot_id', NEW.scope_snapshot_id,
        'wave_run_id', NEW.wave_run_id,
        'wave_unit_id', NEW.wave_unit_id,
        'organization_id', NEW.organization_id,
        'worker_run_id', NEW.worker_run_id,
        'disposition', NEW.disposition,
        'result_hash', NEW.result_hash,
        'terminal_attempt_row_version', NEW.terminal_attempt_row_version,
        'finding_id', NEW.finding_id,
        'fact_delta_count', NEW.fact_delta_count,
        'terminal_event_id', NEW.terminal_event_id
    );
    NEW.receipt_hash := candidate_recovery_sha256_json(NEW.receipt_payload);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempt_terminal_receipts_derive
BEFORE INSERT ON candidate_attempt_terminal_receipts
FOR EACH ROW EXECUTE FUNCTION derive_candidate_attempt_terminal_receipt();

CREATE TRIGGER candidate_attempt_terminal_receipts_immutable
BEFORE UPDATE OR DELETE ON candidate_attempt_terminal_receipts
FOR EACH ROW EXECUTE FUNCTION reject_candidate_recovery_immutable_row();

CREATE FUNCTION enforce_candidate_attempt_terminal_protocol()
RETURNS trigger AS $$
DECLARE
    owner_attempt_id UUID;
    attempt_status TEXT;
    intent_disposition TEXT;
    intent_result_hash TEXT;
    receipt_disposition TEXT;
    receipt_result_hash TEXT;
BEGIN
    owner_attempt_id := CASE
        WHEN TG_TABLE_NAME = 'candidate_attempts'
            THEN (to_jsonb(NEW) ->> 'id')::UUID
        ELSE (to_jsonb(NEW) ->> 'attempt_id')::UUID
    END;
    SELECT status INTO attempt_status
      FROM candidate_attempts
     WHERE id = owner_attempt_id;
    SELECT disposition,result_hash
      INTO intent_disposition,intent_result_hash
      FROM candidate_attempt_terminal_intents
     WHERE attempt_id = owner_attempt_id;
    IF intent_disposition IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT disposition,result_hash
      INTO receipt_disposition,receipt_result_hash
      FROM candidate_attempt_terminal_receipts
     WHERE attempt_id = owner_attempt_id;
    IF receipt_disposition IS NULL THEN
        IF attempt_status IS DISTINCT FROM 'terminalization_pending' THEN
            RAISE EXCEPTION 'CANDIDATE_TERMINAL_INTENT_REQUIRES_PENDING_ATTEMPT'
                USING ERRCODE = '23514';
        END IF;
    ELSIF attempt_status IS DISTINCT FROM receipt_disposition
        OR receipt_disposition IS DISTINCT FROM intent_disposition
        OR receipt_result_hash IS DISTINCT FROM intent_result_hash
    THEN
        RAISE EXCEPTION 'CANDIDATE_TERMINAL_RECEIPT_TRUTH_MISMATCH'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER candidate_terminal_protocol_from_intent
AFTER INSERT ON candidate_attempt_terminal_intents
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_terminal_protocol();

CREATE CONSTRAINT TRIGGER candidate_terminal_protocol_from_attempt
AFTER INSERT OR UPDATE ON candidate_attempts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_terminal_protocol();

CREATE CONSTRAINT TRIGGER candidate_terminal_protocol_from_receipt
AFTER INSERT ON candidate_attempt_terminal_receipts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_terminal_protocol();

-- -------------------------------------------------------------------------
-- Restricted operator recovery CAS. Frozen identity is DB-derived; operators
-- may record exactly one of the three decisions below and may never rewrite
-- target, plan, action arguments, budget, or evidence owner.
-- -------------------------------------------------------------------------

CREATE TABLE candidate_recovery_cases (
    id UUID PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE CHECK (request_id = BTRIM(request_id) AND request_id <> ''),
    attempt_id UUID NOT NULL,
    action_id UUID,
    intent_id UUID,
    approval_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_identity_hash TEXT NOT NULL CHECK (BTRIM(target_identity_hash) <> ''),
    candidate_plan_hash TEXT NOT NULL CHECK (BTRIM(candidate_plan_hash) <> ''),
    worker_run_id UUID NOT NULL,
    evidence_owner_attempt_id UUID NOT NULL,
    attempt_row_version BIGINT NOT NULL CHECK (attempt_row_version >= 0),
    expected_action_args_hash TEXT CHECK (
        expected_action_args_hash IS NULL
        OR expected_action_args_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    expected_budget_hash TEXT NOT NULL CHECK (
        expected_budget_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    case_kind TEXT NOT NULL CHECK (case_kind IN (
        'outcome_unknown',
        'terminalization_stalled',
        'approval_start_expired',
        'response_loss'
    )),
    reason_code TEXT NOT NULL CHECK (reason_code = BTRIM(reason_code) AND reason_code <> ''),
    status TEXT NOT NULL DEFAULT 'open' CHECK (
        status IN ('open','decision_recorded','resolved')
    ),
    resolution_kind TEXT CHECK (
        resolution_kind IS NULL OR resolution_kind IN (
            'terminalize_blocked_outcome_unknown',
            'abandon_before_side_effect',
            'accept_external_result_with_exact_evidence'
        )
    ),
    resolution_request_id TEXT UNIQUE,
    resolution_payload JSONB CHECK (
        resolution_payload IS NULL OR jsonb_typeof(resolution_payload) = 'object'
    ),
    resolved_by UUID REFERENCES operator_principals(id) ON DELETE RESTRICT,
    decided_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (id, attempt_id),
    FOREIGN KEY (action_id, attempt_id)
        REFERENCES candidate_attempt_actions(id, attempt_id) ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, attempt_id)
        REFERENCES candidate_attempt_terminal_intents(id, attempt_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        attempt_id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        worker_run_id
    ) REFERENCES candidate_attempts(
        id,
        approval_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        target_identity_hash,
        candidate_plan_hash,
        stage_worker_run_id
    ) ON DELETE RESTRICT,
    CHECK (evidence_owner_attempt_id = attempt_id),
    CHECK (
        (
            status = 'open'
            AND resolution_kind IS NULL
            AND resolution_request_id IS NULL
            AND resolution_payload IS NULL
            AND resolved_by IS NULL
            AND decided_at IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'decision_recorded'
            AND resolution_kind IS NOT NULL
            AND BTRIM(COALESCE(resolution_request_id, '')) <> ''
            AND resolved_by IS NOT NULL
            AND decided_at IS NOT NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'resolved'
            AND resolution_kind IS NOT NULL
            AND BTRIM(COALESCE(resolution_request_id, '')) <> ''
            AND resolved_by IS NOT NULL
            AND decided_at IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

CREATE UNIQUE INDEX candidate_recovery_one_active_case
    ON candidate_recovery_cases(
        attempt_id,
        COALESCE(action_id, '00000000-0000-0000-0000-000000000000'::UUID),
        case_kind
    )
    WHERE status IN ('open','decision_recorded');

CREATE FUNCTION derive_candidate_recovery_case()
RETURNS trigger AS $$
DECLARE
    attempt_row candidate_attempts%ROWTYPE;
    approval_row attack_candidate_approvals%ROWTYPE;
    action_row candidate_attempt_actions%ROWTYPE;
    frozen_scope_hash TEXT;
BEGIN
    SELECT * INTO attempt_row
      FROM candidate_attempts
     WHERE id = NEW.attempt_id
     FOR SHARE;
    IF NOT FOUND OR attempt_row.status NOT IN ('running','terminalization_pending')
        OR attempt_row.stage_worker_run_id IS NULL
    THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_ATTEMPT_NOT_RECOVERABLE'
            USING ERRCODE = '23514';
    END IF;
    SELECT * INTO approval_row
      FROM attack_candidate_approvals
     WHERE id = attempt_row.approval_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_APPROVAL_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF NEW.action_id IS NOT NULL THEN
        SELECT * INTO action_row
          FROM candidate_attempt_actions
         WHERE id = NEW.action_id AND attempt_id = NEW.attempt_id
         FOR SHARE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_ACTION_MISSING'
                USING ERRCODE = '23503';
        END IF;
    END IF;
    IF NEW.case_kind = 'outcome_unknown'
        AND (NEW.action_id IS NULL OR action_row.status <> 'outcome_unknown')
    THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_OUTCOME_UNKNOWN_ACTION_REQUIRED'
            USING ERRCODE = '23514';
    ELSIF NEW.case_kind = 'terminalization_stalled' AND NOT EXISTS (
        SELECT 1 FROM candidate_attempt_terminal_intents
         WHERE attempt_id = NEW.attempt_id
           AND NOT EXISTS (
               SELECT 1 FROM candidate_attempt_terminal_receipts
                WHERE attempt_id = NEW.attempt_id
           )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_TERMINAL_INTENT_NOT_STALLED'
            USING ERRCODE = '23514';
    ELSIF NEW.case_kind = 'approval_start_expired' AND (
        clock_timestamp() < approval_row.start_before
        OR EXISTS (
            SELECT 1 FROM candidate_attempt_actions
             WHERE attempt_id = NEW.attempt_id AND started_at IS NOT NULL
        )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_APPROVAL_NOT_ABANDONABLE'
            USING ERRCODE = '23514';
    ELSIF NEW.case_kind = 'response_loss' AND NOT EXISTS (
        SELECT 1 FROM candidate_attempt_terminal_barriers
         WHERE attempt_id = NEW.attempt_id
           AND NOT EXISTS (
               SELECT 1 FROM candidate_attempt_terminal_receipts
                WHERE attempt_id = NEW.attempt_id
           )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_RESPONSE_LOSS_NOT_PROVEN'
            USING ERRCODE = '23514';
    END IF;

    SELECT scope_hash INTO frozen_scope_hash
      FROM operation_org_scope_snapshots
     WHERE id = attempt_row.scope_snapshot_id
       AND operation_id = attempt_row.operation_id;
    IF frozen_scope_hash IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_SCOPE_MISSING'
            USING ERRCODE = '23503';
    END IF;
    NEW.intent_id := (
        SELECT id FROM candidate_attempt_terminal_intents
         WHERE attempt_id = NEW.attempt_id
    );
    NEW.approval_id := attempt_row.approval_id;
    NEW.candidate_id := attempt_row.candidate_id;
    NEW.operation_id := attempt_row.operation_id;
    NEW.scope_snapshot_id := attempt_row.scope_snapshot_id;
    NEW.wave_run_id := attempt_row.wave_run_id;
    NEW.wave_unit_id := attempt_row.wave_unit_id;
    NEW.organization_id := attempt_row.organization_id;
    NEW.target_live_id := attempt_row.target_live_id;
    NEW.target_identity_hash := attempt_row.target_identity_hash;
    NEW.candidate_plan_hash := attempt_row.candidate_plan_hash;
    NEW.worker_run_id := attempt_row.stage_worker_run_id;
    NEW.evidence_owner_attempt_id := attempt_row.id;
    NEW.attempt_row_version := attempt_row.row_version;
    NEW.expected_action_args_hash := CASE
        WHEN NEW.action_id IS NULL THEN NULL
        ELSE candidate_recovery_sha256_json(action_row.canonical_args)
    END;
    NEW.expected_budget_hash := candidate_recovery_sha256_json(approval_row.budget);
    NEW.status := 'open';
    NEW.resolution_kind := NULL;
    NEW.resolution_request_id := NULL;
    NEW.resolution_payload := NULL;
    NEW.resolved_by := NULL;
    NEW.decided_at := NULL;
    NEW.completed_at := NULL;
    NEW.row_version := 0;
    NEW.created_at := clock_timestamp();
    NEW.updated_at := NEW.created_at;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_recovery_cases_derive
BEFORE INSERT ON candidate_recovery_cases
FOR EACH ROW EXECUTE FUNCTION derive_candidate_recovery_case();

CREATE FUNCTION guard_candidate_recovery_case_transition()
RETURNS trigger AS $$
DECLARE
    active_operator BOOLEAN;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_CASE_DELETE_REJECTED'
            USING ERRCODE = '23514';
    END IF;
    IF (to_jsonb(NEW) - ARRAY[
            'status','resolution_kind','resolution_request_id',
            'resolution_payload','resolved_by','decided_at','completed_at',
            'row_version','updated_at'
        ]::TEXT[])
       IS DISTINCT FROM
       (to_jsonb(OLD) - ARRAY[
            'status','resolution_kind','resolution_request_id',
            'resolution_payload','resolved_by','decided_at','completed_at',
            'row_version','updated_at'
        ]::TEXT[])
    THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_IDENTITY_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.status = 'open' AND NEW.status = 'decision_recorded' THEN
        IF NEW.row_version <> OLD.row_version + 1
            OR NEW.resolution_kind NOT IN (
                'terminalize_blocked_outcome_unknown',
                'abandon_before_side_effect',
                'accept_external_result_with_exact_evidence'
            )
            OR BTRIM(COALESCE(NEW.resolution_request_id, '')) = ''
            OR NEW.resolved_by IS NULL
        THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_DECISION_CAS_INVALID'
                USING ERRCODE = '23514';
        END IF;
        SELECT EXISTS(
            SELECT 1 FROM operator_principals
             WHERE id = NEW.resolved_by AND active
        ) INTO active_operator;
        IF NOT active_operator THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_OPERATOR_INVALID'
                USING ERRCODE = '23514';
        END IF;
        IF COALESCE(NEW.resolution_payload ?| ARRAY[
            'target','target_id','target_identity_hash',
            'candidate_plan_hash','plan','args','canonical_args',
            'budget','evidence_owner_attempt_id'
        ]::TEXT[], FALSE) THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_FROZEN_AUTHORITY_IN_PAYLOAD'
                USING ERRCODE = '23514';
        END IF;
        IF NEW.resolution_kind = 'abandon_before_side_effect' AND (
            EXISTS (
                SELECT 1 FROM candidate_attempt_actions
                 WHERE attempt_id = NEW.attempt_id AND started_at IS NOT NULL
            )
            OR EXISTS (
                SELECT 1 FROM candidate_attempt_terminal_intents
                 WHERE attempt_id = NEW.attempt_id
            )
        ) THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_ABANDON_AFTER_SIDE_EFFECT'
                USING ERRCODE = '23514';
        ELSIF NEW.resolution_kind = 'terminalize_blocked_outcome_unknown'
            AND NOT EXISTS (
                SELECT 1 FROM candidate_attempt_actions
                 WHERE id = NEW.action_id AND attempt_id = NEW.attempt_id
                   AND status = 'outcome_unknown'
            )
        THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_BLOCKED_REQUIRES_UNKNOWN_OUTCOME'
                USING ERRCODE = '23514';
        ELSIF NEW.resolution_kind = 'accept_external_result_with_exact_evidence'
            AND NOT EXISTS (
                SELECT 1 FROM candidate_recovery_evidence
                 WHERE recovery_case_id = NEW.id AND role = 'external_result'
            )
        THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_EXTERNAL_RESULT_EVIDENCE_REQUIRED'
                USING ERRCODE = '23514';
        END IF;
        NEW.decided_at := clock_timestamp();
        NEW.completed_at := NULL;
        NEW.updated_at := NEW.decided_at;
        RETURN NEW;
    END IF;
    IF OLD.status = 'decision_recorded' AND NEW.status = 'resolved' THEN
        IF NEW.row_version <> OLD.row_version + 1
            OR NEW.resolution_kind IS DISTINCT FROM OLD.resolution_kind
            OR NEW.resolution_request_id IS DISTINCT FROM OLD.resolution_request_id
            OR NEW.resolution_payload IS DISTINCT FROM OLD.resolution_payload
            OR NEW.resolved_by IS DISTINCT FROM OLD.resolved_by
            OR NEW.decided_at IS DISTINCT FROM OLD.decided_at
        THEN
            RAISE EXCEPTION 'CANDIDATE_RECOVERY_COMPLETION_CAS_INVALID'
                USING ERRCODE = '23514';
        END IF;
        NEW.completed_at := clock_timestamp();
        NEW.updated_at := NEW.completed_at;
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'CANDIDATE_RECOVERY_CASE_IMMUTABLE'
        USING ERRCODE = '23514';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_recovery_case_transition_guard
BEFORE UPDATE OR DELETE ON candidate_recovery_cases
FOR EACH ROW EXECUTE FUNCTION guard_candidate_recovery_case_transition();

CREATE TABLE candidate_recovery_evidence (
    recovery_case_id UUID NOT NULL
        REFERENCES candidate_recovery_cases(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('external_result','operator_basis')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (recovery_case_id, evidence_id, role)
);

CREATE FUNCTION guard_candidate_recovery_evidence()
RETURNS trigger AS $$
DECLARE
    case_status TEXT;
    owner_case_id UUID;
BEGIN
    owner_case_id := CASE
        WHEN TG_OP = 'DELETE' THEN OLD.recovery_case_id
        ELSE NEW.recovery_case_id
    END;
    SELECT status INTO case_status
      FROM candidate_recovery_cases
     WHERE id = owner_case_id
     FOR UPDATE;
    IF case_status IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_CASE_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF TG_OP <> 'INSERT' OR case_status <> 'open' THEN
        RAISE EXCEPTION 'CANDIDATE_RECOVERY_EVIDENCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_recovery_evidence_immutable
BEFORE INSERT OR UPDATE OR DELETE ON candidate_recovery_evidence
FOR EACH ROW EXECUTE FUNCTION guard_candidate_recovery_evidence();

CREATE CONSTRAINT TRIGGER candidate_recovery_evidence_owner
AFTER INSERT OR UPDATE ON candidate_recovery_evidence
DEFERRABLE INITIALLY IMMEDIATE
FOR EACH ROW EXECUTE FUNCTION enforce_attack_evidence_owner(
    'candidate_recovery_cases', 'id', 'operation_id', 'recovery_case_id'
);

-- `start_before` authorizes starting new external work; it is not a deadline
-- for persisting or recovering work that already crossed the side-effect
-- boundary.  The legacy trigger used `expires_at` for every queued/running
-- Attempt update, which made a completed action impossible to reclaim for the
-- submit-only continuation once the approval clock elapsed.
--
-- Keep INSERT and ordinary queued -> running claims fail-closed on the current
-- approval window.  After that window, only an existing exact Attempt whose
-- action journal is already terminal and contains no in-flight/unknown action
-- may remain queued/running while its original Worker binding is reclaimed.
CREATE OR REPLACE FUNCTION enforce_candidate_attempt_authority()
RETURNS trigger AS $$
DECLARE
    approval_is_current BOOLEAN;
    approval_is_recoverable BOOLEAN;
    action_journal_is_submit_only BOOLEAN;
BEGIN
    IF NEW.status IN ('queued', 'running') THEN
        SELECT
            approval.status = 'approved'
                AND approval.start_before > clock_timestamp(),
            approval.status IN ('approved', 'expired')
          INTO approval_is_current, approval_is_recoverable
          FROM attack_candidate_approvals AS approval
         WHERE approval.id = NEW.approval_id
           AND approval.candidate_id = NEW.candidate_id
           AND approval.operation_id = NEW.operation_id
           AND approval.scope_snapshot_id = NEW.scope_snapshot_id
           AND approval.wave_run_id = NEW.wave_run_id
           AND approval.wave_unit_id = NEW.wave_unit_id
           AND approval.organization_id = NEW.organization_id
           AND approval.target_identity_hash = NEW.target_identity_hash
           AND approval.candidate_plan_hash = NEW.candidate_plan_hash;

        approval_is_current := COALESCE(approval_is_current, FALSE);
        approval_is_recoverable := COALESCE(approval_is_recoverable, FALSE);

        IF TG_OP = 'UPDATE' THEN
            SELECT
                EXISTS (
                    SELECT 1
                      FROM candidate_attempt_actions AS action
                     WHERE action.attempt_id = NEW.id
                       AND action.status IN ('completed', 'failed')
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM candidate_attempt_actions AS action
                     WHERE action.attempt_id = NEW.id
                       AND action.status IN ('started', 'outcome_unknown')
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM candidate_attempt_terminal_intents AS intent
                     WHERE intent.attempt_id = NEW.id
                )
              INTO action_journal_is_submit_only;
        ELSE
            action_journal_is_submit_only := FALSE;
        END IF;

        IF NOT approval_is_current AND NOT (
            TG_OP = 'UPDATE'
            AND approval_is_recoverable
            AND action_journal_is_submit_only
        ) THEN
            RAISE EXCEPTION 'candidate attempt requires a current approved exact plan';
        END IF;
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

CREATE INDEX candidate_terminal_intents_pending_idx
    ON candidate_attempt_terminal_intents(operation_id, created_at);

CREATE INDEX candidate_recovery_cases_queue_idx
    ON candidate_recovery_cases(operation_id, status, created_at);
