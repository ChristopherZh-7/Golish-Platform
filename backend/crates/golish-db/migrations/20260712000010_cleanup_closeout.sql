-- Cleanup P7b: retained target identity, DB-global closeout work and the
-- durable first half of two-phase organization deletion.
--
-- The deletion request transaction owns every DB precondition and creates all
-- projection invalidation deliveries before filesystem cleanup can be
-- claimed.  The worker only receives frozen path/target snapshots after that
-- transaction commits; the final live-row hard delete is a separate commit.

-- ---------------------------------------------------------------------------
-- Historical target identity: immutable at-time id/snapshot + nullable live FK
-- ---------------------------------------------------------------------------

ALTER TABLE attack_candidates
    ADD COLUMN target_id_at_time UUID,
    ADD COLUMN live_target_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    ADD COLUMN canonical_target_snapshot JSONB;

UPDATE attack_candidates
   SET target_id_at_time = COALESCE(
           target_live_id,
           uuid_generate_v5(uuid_ns_url(), 'attack-candidate:' || candidate_id::text)
       ),
       live_target_id = target_live_id,
       canonical_target_snapshot = jsonb_build_object(
           'targetIdAtTime', COALESCE(
               target_live_id,
               uuid_generate_v5(uuid_ns_url(), 'attack-candidate:' || candidate_id::text)
           ),
           'targetTypeAtTime', COALESCE(target_type_at_time, 'legacy'),
           'targetValueAtTime', COALESCE(target_value_at_time, target),
           'targetIdentityHash', COALESCE(target_identity_hash, hypothesis_hash)
       );

ALTER TABLE attack_candidates
    ALTER COLUMN target_id_at_time SET NOT NULL,
    ALTER COLUMN canonical_target_snapshot SET NOT NULL,
    ADD CONSTRAINT attack_candidates_canonical_target_snapshot_object
        CHECK (jsonb_typeof(canonical_target_snapshot) = 'object');

ALTER TABLE attack_candidate_approvals
    ADD COLUMN target_id_at_time UUID,
    ADD COLUMN live_target_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    ADD COLUMN canonical_target_snapshot JSONB;

UPDATE attack_candidate_approvals
   SET target_id_at_time = COALESCE(
           target_live_id,
           uuid_generate_v5(uuid_ns_url(), 'attack-approval:' || id::text)
       ),
       live_target_id = target_live_id,
       canonical_target_snapshot = jsonb_build_object(
           'targetIdAtTime', COALESCE(
               target_live_id,
               uuid_generate_v5(uuid_ns_url(), 'attack-approval:' || id::text)
           ),
           'targetTypeAtTime', target_type_at_time,
           'targetValueAtTime', target_value_at_time,
           'targetIdentityHash', target_identity_hash
       );

ALTER TABLE attack_candidate_approvals
    ALTER COLUMN target_id_at_time SET NOT NULL,
    ALTER COLUMN canonical_target_snapshot SET NOT NULL,
    ADD CONSTRAINT attack_candidate_approvals_canonical_target_snapshot_object
        CHECK (jsonb_typeof(canonical_target_snapshot) = 'object');

ALTER TABLE finding_lineage
    ADD COLUMN target_id_at_time UUID,
    ADD COLUMN live_target_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    ADD COLUMN canonical_target_snapshot JSONB;

UPDATE finding_lineage
   SET target_id_at_time = COALESCE(
           target_live_id,
           uuid_generate_v5(uuid_ns_url(), 'finding-lineage:' || id::text)
       ),
       live_target_id = target_live_id,
       canonical_target_snapshot = jsonb_build_object(
           'targetIdAtTime', COALESCE(
               target_live_id,
               uuid_generate_v5(uuid_ns_url(), 'finding-lineage:' || id::text)
           ),
           'targetTypeAtTime', target_type_at_time,
           'targetValueAtTime', target_value_at_time,
           'targetIdentityHash', target_identity_hash
       );

ALTER TABLE finding_lineage
    ALTER COLUMN target_id_at_time SET NOT NULL,
    ALTER COLUMN canonical_target_snapshot SET NOT NULL,
    ADD CONSTRAINT finding_lineage_canonical_target_snapshot_object
        CHECK (jsonb_typeof(canonical_target_snapshot) = 'object');

-- P2 keeps its compatibility-facing `target_live_id` API.  C8 makes the new
-- dual-field storage authoritative and requires both nullable live aliases to
-- identify the exact retained at-time target whenever they are present.
ALTER TABLE attack_candidates
    ADD CONSTRAINT attack_candidates_live_target_alias_exact CHECK (
        target_live_id IS NOT DISTINCT FROM live_target_id
        AND (live_target_id IS NULL OR live_target_id = target_id_at_time)
    );
ALTER TABLE attack_candidate_approvals
    ADD CONSTRAINT attack_candidate_approvals_live_target_alias_exact CHECK (
        target_live_id IS NOT DISTINCT FROM live_target_id
        AND (live_target_id IS NULL OR live_target_id = target_id_at_time)
    );
ALTER TABLE finding_lineage
    ADD CONSTRAINT finding_lineage_live_target_alias_exact CHECK (
        target_live_id IS NOT DISTINCT FROM live_target_id
        AND (live_target_id IS NULL OR live_target_id = target_id_at_time)
    );

CREATE FUNCTION populate_retained_target_identity()
RETURNS trigger AS $$
DECLARE
    fallback_prefix TEXT;
BEGIN
    fallback_prefix := CASE TG_TABLE_NAME
        WHEN 'attack_candidates' THEN 'attack-candidate:'
        WHEN 'attack_candidate_approvals' THEN 'attack-approval:'
        ELSE 'finding-lineage:'
    END;
    IF TG_OP = 'UPDATE' AND (
        (OLD.target_live_id IS NOT NULL AND NEW.target_live_id IS NULL)
        OR (OLD.live_target_id IS NOT NULL AND NEW.live_target_id IS NULL)
    ) THEN
        NEW.live_target_id := NULL;
        NEW.target_live_id := NULL;
    ELSE
        NEW.live_target_id := COALESCE(NEW.live_target_id, NEW.target_live_id);
        NEW.target_live_id := NEW.live_target_id;
    END IF;
    NEW.target_id_at_time := COALESCE(
        NEW.target_id_at_time,
        NEW.live_target_id,
        uuid_generate_v5(
            uuid_ns_url(),
            fallback_prefix || COALESCE(
                to_jsonb(NEW)->>'candidate_id',
                to_jsonb(NEW)->>'id'
            )
        )
    );
    NEW.canonical_target_snapshot := COALESCE(
        NEW.canonical_target_snapshot,
        jsonb_build_object(
            'targetIdAtTime', NEW.target_id_at_time,
            'targetTypeAtTime', COALESCE(NEW.target_type_at_time, 'legacy'),
            'targetValueAtTime', COALESCE(
                NEW.target_value_at_time,
                to_jsonb(NEW)->>'target'
            ),
            'targetIdentityHash', COALESCE(
                NEW.target_identity_hash,
                to_jsonb(NEW)->>'hypothesis_hash'
            )
        )
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_retained_target_identity
BEFORE INSERT OR UPDATE OF target_live_id,live_target_id,target_id_at_time,
    target_type_at_time,target_value_at_time,target_identity_hash
ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION populate_retained_target_identity();

CREATE TRIGGER attack_candidate_approvals_retained_target_identity
BEFORE INSERT OR UPDATE OF target_live_id,live_target_id,target_id_at_time,
    target_type_at_time,target_value_at_time,target_identity_hash
ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION populate_retained_target_identity();

CREATE TRIGGER finding_lineage_retained_target_identity
BEFORE INSERT OR UPDATE OF target_live_id,live_target_id,target_id_at_time,
    target_type_at_time,target_value_at_time,target_identity_hash
ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION populate_retained_target_identity();

CREATE FUNCTION reject_retained_target_snapshot_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.target_id_at_time IS DISTINCT FROM OLD.target_id_at_time
        OR NEW.canonical_target_snapshot IS DISTINCT FROM OLD.canonical_target_snapshot
    THEN
        RAISE EXCEPTION 'retained target identity is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_retain_target_snapshot
BEFORE UPDATE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION reject_retained_target_snapshot_change();

CREATE TRIGGER attack_candidate_approvals_retain_target_snapshot
BEFORE UPDATE ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION reject_retained_target_snapshot_change();

CREATE TRIGGER finding_lineage_retain_target_snapshot
BEFORE UPDATE ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION reject_retained_target_snapshot_change();

-- ---------------------------------------------------------------------------
-- Two-phase organization deletion job and immutable request snapshots
-- ---------------------------------------------------------------------------

-- P7a allowed a residual payload only for a waiver. P7b also treats a blocked
-- obligation as terminal, but only when its unresolved residual is disclosed.
DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT conname INTO constraint_name
      FROM pg_constraint
     WHERE conrelid='cleanup_obligations'::regclass
       AND contype='c'
       AND pg_get_constraintdef(oid) LIKE '%waived_by_user%residual_risk%'
     ORDER BY conname
     LIMIT 1;
    IF constraint_name IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE cleanup_obligations DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;
END $$;

ALTER TABLE cleanup_obligations
    ADD CONSTRAINT cleanup_obligations_terminal_residual_disclosure CHECK (
        (status IN ('blocked','waived_by_user')) = (residual_risk IS NOT NULL)
    );

-- A terminal label is not terminal truth. `verified_absent` must be backed by
-- one exact cleanup attempt plus an independently-owned absence proof;
-- `waived_by_user` must be backed by one retained local-operator decision;
-- `blocked` receives its own retained decision/evidence relation rather than
-- reusing obligation-creation evidence. These relations are checked again by
-- Gate/deletion reads, but a deferred trigger also prevents ordinary writers
-- from committing a forged or only-partially-written terminal state.
CREATE TABLE cleanup_blocked_decisions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    obligation_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    decided_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (BTRIM(reason) <> '' AND LENGTH(reason) <= 4096),
    residual_risk JSONB NOT NULL CHECK (jsonb_typeof(residual_risk) = 'object'),
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (
        obligation_id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time
    ) REFERENCES cleanup_obligations(
        id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time
    ) ON DELETE RESTRICT
);

CREATE TABLE cleanup_blocked_decision_evidence (
    blocked_decision_id UUID NOT NULL
        REFERENCES cleanup_blocked_decisions(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('decision','residual','support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (blocked_decision_id,evidence_id,role)
);

-- Cleanup terminal events derive their immutable evidence union from these
-- retained child relations.  Serialize membership inserts with the parent
-- status transition so evidence may be written before terminalization in the
-- same transaction, but can never grow after the canonical event is emitted.
CREATE FUNCTION guard_cleanup_terminal_evidence_insert()
RETURNS trigger AS $$
DECLARE
    obligation_uuid UUID;
    obligation_status TEXT;
BEGIN
    obligation_uuid := CASE TG_TABLE_NAME
        WHEN 'cleanup_obligation_evidence' THEN
            (to_jsonb(NEW)->>'obligation_id')::UUID
        WHEN 'cleanup_attempt_evidence' THEN (
            SELECT obligation_id FROM cleanup_attempts
             WHERE id=(to_jsonb(NEW)->>'attempt_id')::UUID
        )
        WHEN 'cleanup_absence_check_evidence' THEN (
            SELECT obligation_id FROM cleanup_absence_checks
             WHERE id=(to_jsonb(NEW)->>'absence_check_id')::UUID
        )
        WHEN 'cleanup_waiver_evidence' THEN (
            SELECT obligation_id FROM cleanup_waivers
             WHERE id=(to_jsonb(NEW)->>'waiver_id')::UUID
        )
        WHEN 'cleanup_blocked_decision_evidence' THEN (
            SELECT obligation_id FROM cleanup_blocked_decisions
             WHERE id=(to_jsonb(NEW)->>'blocked_decision_id')::UUID
        )
        ELSE NULL
    END;

    SELECT status
      INTO obligation_status
      FROM cleanup_obligations
     WHERE id=obligation_uuid
     FOR SHARE;
    IF obligation_status IN ('verified_absent','waived_by_user','blocked') THEN
        RAISE EXCEPTION 'CLEANUP_TERMINAL_EVIDENCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER cleanup_obligation_evidence_terminal_insert_guard
BEFORE INSERT ON cleanup_obligation_evidence
FOR EACH ROW EXECUTE FUNCTION guard_cleanup_terminal_evidence_insert();

CREATE TRIGGER cleanup_attempt_evidence_terminal_insert_guard
BEFORE INSERT ON cleanup_attempt_evidence
FOR EACH ROW EXECUTE FUNCTION guard_cleanup_terminal_evidence_insert();

CREATE TRIGGER cleanup_absence_evidence_terminal_insert_guard
BEFORE INSERT ON cleanup_absence_check_evidence
FOR EACH ROW EXECUTE FUNCTION guard_cleanup_terminal_evidence_insert();

CREATE TRIGGER cleanup_waiver_evidence_terminal_insert_guard
BEFORE INSERT ON cleanup_waiver_evidence
FOR EACH ROW EXECUTE FUNCTION guard_cleanup_terminal_evidence_insert();

CREATE TRIGGER cleanup_blocked_evidence_terminal_insert_guard
BEFORE INSERT ON cleanup_blocked_decision_evidence
FOR EACH ROW EXECUTE FUNCTION guard_cleanup_terminal_evidence_insert();

-- One live attempt may make exactly one transition into a terminal status.
-- Once OLD is terminal, even a no-op update would drift the canonical
-- terminal source relation and is rejected together with deletion.
CREATE FUNCTION reject_terminal_cleanup_attempt_change()
RETURNS trigger AS $$
BEGIN
    IF OLD.status IN ('verified_absent','verification_failed','execution_failed') THEN
        RAISE EXCEPTION 'CLEANUP_TERMINAL_ATTEMPT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger names are executed alphabetically.  The 00 prefix ensures terminal
-- DELETE receives the terminal-source error before the older generic retained
-- identity trigger rejects deletion.
CREATE TRIGGER cleanup_attempts_00_terminal_immutable
BEFORE UPDATE OR DELETE ON cleanup_attempts
FOR EACH ROW EXECUTE FUNCTION reject_terminal_cleanup_attempt_change();

CREATE TRIGGER cleanup_blocked_decisions_immutable
BEFORE UPDATE OR DELETE ON cleanup_blocked_decisions
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_blocked_decision_evidence_immutable
BEFORE UPDATE OR DELETE ON cleanup_blocked_decision_evidence
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_obligation_evidence_immutable
BEFORE UPDATE OR DELETE ON cleanup_obligation_evidence
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_attempt_evidence_immutable
BEFORE UPDATE OR DELETE ON cleanup_attempt_evidence
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_absence_check_evidence_immutable
BEFORE UPDATE OR DELETE ON cleanup_absence_check_evidence
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_waiver_evidence_immutable
BEFORE UPDATE OR DELETE ON cleanup_waiver_evidence
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE FUNCTION cleanup_obligation_state_truth_is_exact(obligation_uuid UUID)
RETURNS BOOLEAN AS $$
    SELECT CASE obligation.status
        WHEN 'open' THEN
            obligation.terminal_at IS NULL
            AND obligation.residual_risk IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status IN (
                       'claimed','executing','cleaned_pending_verification','verified_absent'
                   )
            )
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_waivers AS waiver
                 WHERE waiver.obligation_id=obligation.id
            )
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_blocked_decisions AS decision
                 WHERE decision.obligation_id=obligation.id
            )
        WHEN 'in_progress' THEN
            obligation.terminal_at IS NULL
            AND obligation.residual_risk IS NULL
            AND (
                SELECT COUNT(*) FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status IN (
                       'claimed','executing','cleaned_pending_verification'
                   )
            ) = 1
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status='verified_absent'
            )
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_waivers AS waiver
                 WHERE waiver.obligation_id=obligation.id
            )
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_blocked_decisions AS decision
                 WHERE decision.obligation_id=obligation.id
            )
        WHEN 'verified_absent' THEN
            obligation.terminal_at IS NOT NULL
            AND obligation.residual_risk IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status IN (
                       'claimed','executing','cleaned_pending_verification'
                   )
            )
            AND (
                SELECT COUNT(*) FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status='verified_absent'
            ) = 1
            AND (
                SELECT COUNT(*)
                  FROM cleanup_attempts AS attempt
                  JOIN cleanup_absence_checks AS absence
                    ON absence.cleanup_attempt_id=attempt.id
                   AND absence.obligation_id=attempt.obligation_id
                  JOIN stage_worker_runs AS executor
                    ON executor.id=attempt.worker_run_id
                   AND executor.operation_id=attempt.operation_id
                   AND executor.organization_id=attempt.organization_id_at_time
                  JOIN stage_worker_runs AS verifier
                    ON verifier.id=absence.verifier_worker_run_id
                   AND verifier.operation_id=absence.operation_id
                   AND verifier.organization_id=absence.organization_id_at_time
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status='verified_absent'
                   AND attempt.completed_at IS NOT NULL
                   AND absence.disposition='absent'
                   AND absence.resource_identity_hash=obligation.resource_identity_hash
                   AND executor.id<>verifier.id
                   AND EXISTS (
                       SELECT 1 FROM cleanup_attempt_evidence AS execution_evidence
                        WHERE execution_evidence.attempt_id=attempt.id
                          AND execution_evidence.role IN ('execution','result')
                   )
                   AND EXISTS (
                       SELECT 1 FROM cleanup_absence_check_evidence AS absence_evidence
                        WHERE absence_evidence.absence_check_id=absence.id
                          AND absence_evidence.role='absence'
                   )
                   AND NOT EXISTS (
                       SELECT 1
                         FROM cleanup_attempt_evidence AS execution_evidence
                         JOIN cleanup_absence_check_evidence AS absence_evidence
                           ON absence_evidence.evidence_id=execution_evidence.evidence_id
                        WHERE execution_evidence.attempt_id=attempt.id
                          AND absence_evidence.absence_check_id=absence.id
                   )
            ) = 1
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_waivers AS waiver
                 WHERE waiver.obligation_id=obligation.id
            )
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_blocked_decisions AS decision
                 WHERE decision.obligation_id=obligation.id
            )
        WHEN 'waived_by_user' THEN
            obligation.terminal_at IS NOT NULL
            AND obligation.residual_risk IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status IN (
                       'claimed','executing','cleaned_pending_verification','verified_absent'
                   )
            )
            AND (
                SELECT COUNT(*)
                  FROM cleanup_waivers AS waiver
                  JOIN operator_principals AS principal
                    ON principal.id=waiver.decided_by_principal_id
                 WHERE waiver.obligation_id=obligation.id
                   AND waiver.residual_risk=obligation.residual_risk
                   AND principal.principal_kind='local_operator'
                   AND EXISTS (
                       SELECT 1 FROM cleanup_waiver_evidence AS evidence
                        WHERE evidence.waiver_id=waiver.id
                          AND evidence.role='decision'
                   )
            ) = 1
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_blocked_decisions AS decision
                 WHERE decision.obligation_id=obligation.id
            )
        WHEN 'blocked' THEN
            obligation.terminal_at IS NOT NULL
            AND obligation.residual_risk IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_attempts AS attempt
                 WHERE attempt.obligation_id=obligation.id
                   AND attempt.status IN (
                       'claimed','executing','cleaned_pending_verification','verified_absent'
                   )
            )
            AND (
                SELECT COUNT(*)
                  FROM cleanup_blocked_decisions AS decision
                  JOIN operator_principals AS principal
                    ON principal.id=decision.decided_by_principal_id
                 WHERE decision.obligation_id=obligation.id
                   AND decision.residual_risk=obligation.residual_risk
                   AND principal.principal_kind='local_operator'
                   AND EXISTS (
                       SELECT 1 FROM cleanup_blocked_decision_evidence AS evidence
                        WHERE evidence.blocked_decision_id=decision.id
                          AND evidence.role='decision'
                   )
            ) = 1
            AND NOT EXISTS (
                SELECT 1 FROM cleanup_waivers AS waiver
                 WHERE waiver.obligation_id=obligation.id
            )
        ELSE FALSE
    END
      FROM cleanup_obligations AS obligation
     WHERE obligation.id=obligation_uuid;
$$ LANGUAGE sql STABLE;

CREATE FUNCTION enforce_cleanup_obligation_state_truth()
RETURNS trigger AS $$
DECLARE
    obligation_uuid UUID;
BEGIN
    obligation_uuid := CASE TG_TABLE_NAME
        WHEN 'cleanup_obligations' THEN (to_jsonb(NEW)->>'id')::UUID
        WHEN 'cleanup_attempts' THEN (to_jsonb(NEW)->>'obligation_id')::UUID
        WHEN 'cleanup_absence_checks' THEN (to_jsonb(NEW)->>'obligation_id')::UUID
        WHEN 'cleanup_waivers' THEN (to_jsonb(NEW)->>'obligation_id')::UUID
        WHEN 'cleanup_blocked_decisions' THEN (to_jsonb(NEW)->>'obligation_id')::UUID
        WHEN 'cleanup_absence_check_evidence' THEN (
            SELECT obligation_id FROM cleanup_absence_checks
             WHERE id=(to_jsonb(NEW)->>'absence_check_id')::UUID
        )
        WHEN 'cleanup_waiver_evidence' THEN (
            SELECT obligation_id FROM cleanup_waivers
             WHERE id=(to_jsonb(NEW)->>'waiver_id')::UUID
        )
        WHEN 'cleanup_blocked_decision_evidence' THEN (
            SELECT obligation_id FROM cleanup_blocked_decisions
             WHERE id=(to_jsonb(NEW)->>'blocked_decision_id')::UUID
        )
        ELSE NULL
    END;
    IF obligation_uuid IS NULL
        OR NOT COALESCE(cleanup_obligation_state_truth_is_exact(obligation_uuid), FALSE)
    THEN
        RAISE EXCEPTION 'cleanup_terminal_truth_invalid' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER cleanup_obligations_state_truth_exact
AFTER INSERT OR UPDATE OF status,terminal_at,residual_risk
ON cleanup_obligations DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_attempts_state_truth_exact
AFTER INSERT OR UPDATE OF status,worker_run_id,completed_at
ON cleanup_attempts DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_absence_checks_state_truth_exact
AFTER INSERT ON cleanup_absence_checks DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_absence_evidence_state_truth_exact
AFTER INSERT ON cleanup_absence_check_evidence DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_waivers_state_truth_exact
AFTER INSERT ON cleanup_waivers DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_waiver_evidence_state_truth_exact
AFTER INSERT ON cleanup_waiver_evidence DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_blocked_decisions_state_truth_exact
AFTER INSERT ON cleanup_blocked_decisions DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE CONSTRAINT TRIGGER cleanup_blocked_evidence_state_truth_exact
AFTER INSERT ON cleanup_blocked_decision_evidence DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_state_truth();

CREATE TABLE organization_deletion_jobs (
    id UUID PRIMARY KEY,
    root_organization_id_at_time UUID NOT NULL,
    project_scope_id UUID NOT NULL
        REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    project_path_at_time TEXT NOT NULL CHECK (BTRIM(project_path_at_time) <> ''),
    requested_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN (
        'waiting_for_invalidation_delivery',
        'pending_artifact_cleanup',
        'artifact_cleanup_succeeded',
        'hard_delete_committed'
    )),
    organization_snapshot JSONB NOT NULL CHECK (
        jsonb_typeof(organization_snapshot) = 'array'
        AND jsonb_array_length(organization_snapshot) > 0
    ),
    target_snapshot JSONB NOT NULL CHECK (jsonb_typeof(target_snapshot) = 'array'),
    required_invalidation_count INTEGER NOT NULL DEFAULT 0 CHECK (
        required_invalidation_count >= 0
    ),
    lease_owner TEXT,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    artifact_retry_not_before TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hard_delete_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (
        hard_delete_attempt_count >= 0
    ),
    hard_delete_retry_not_before TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    last_error_code TEXT,
    last_error TEXT,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    artifact_cleanup_started_at TIMESTAMPTZ,
    artifact_cleanup_completed_at TIMESTAMPTZ,
    hard_delete_committed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (lease_owner IS NULL AND lease_token IS NULL AND lease_expires_at IS NULL)
        OR (
            state = 'pending_artifact_cleanup'
            AND BTRIM(COALESCE(lease_owner, '')) <> ''
            AND lease_token IS NOT NULL
            AND lease_expires_at IS NOT NULL
        )
    ),
    CHECK (
        (state = 'hard_delete_committed' AND hard_delete_committed_at IS NOT NULL)
        OR (state <> 'hard_delete_committed' AND hard_delete_committed_at IS NULL)
    )
);

CREATE UNIQUE INDEX organization_deletion_jobs_one_active_root
    ON organization_deletion_jobs(root_organization_id_at_time)
    WHERE state <> 'hard_delete_committed';

CREATE TABLE organization_deletion_job_units (
    job_id UUID NOT NULL REFERENCES organization_deletion_jobs(id) ON DELETE RESTRICT,
    organization_id_at_time UUID NOT NULL,
    parent_organization_id_at_time UUID,
    organization_name_at_time TEXT NOT NULL CHECK (BTRIM(organization_name_at_time) <> ''),
    depth INTEGER NOT NULL CHECK (depth >= 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (job_id,organization_id_at_time),
    UNIQUE (job_id,ordinal)
);

CREATE TABLE organization_deletion_job_targets (
    job_id UUID NOT NULL REFERENCES organization_deletion_jobs(id) ON DELETE RESTRICT,
    target_id_at_time UUID NOT NULL,
    live_target_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    organization_id_at_time UUID NOT NULL,
    canonical_target_snapshot JSONB NOT NULL CHECK (
        jsonb_typeof(canonical_target_snapshot) = 'object'
    ),
    PRIMARY KEY (job_id,target_id_at_time),
    FOREIGN KEY (job_id,organization_id_at_time)
        REFERENCES organization_deletion_job_units(job_id,organization_id_at_time)
        ON DELETE RESTRICT
);

CREATE TABLE organization_deletion_job_invalidations (
    job_id UUID NOT NULL REFERENCES organization_deletion_jobs(id) ON DELETE RESTRICT,
    event_id UUID NOT NULL REFERENCES knowledge_outbox_events(event_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL CHECK (BTRIM(source_stream_key) <> ''),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    -- Freeze the exact projector contract produced by the event catalog when
    -- the deletion request commits.  The worker must not infer a magic route
    -- count: projector additions/removals are represented by this manifest.
    required_delivery_manifest JSONB NOT NULL CHECK (
        jsonb_typeof(required_delivery_manifest) = 'array'
        AND jsonb_array_length(required_delivery_manifest) > 0
    ),
    PRIMARY KEY (job_id,event_id)
);

CREATE TABLE organization_deletion_job_state_history (
    job_id UUID NOT NULL REFERENCES organization_deletion_jobs(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    state TEXT NOT NULL CHECK (state IN (
        'deleting_db_committed',
        'waiting_for_invalidation_delivery',
        'pending_artifact_cleanup',
        'artifact_cleanup_succeeded',
        'hard_delete_committed'
    )),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    detail JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(detail) = 'object'),
    PRIMARY KEY (job_id,ordinal)
);

CREATE FUNCTION reject_organization_delete_job_history_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'organization deletion history is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION retain_organization_deletion_job_identity()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'organization deletion job history is immutable';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
        OR NEW.root_organization_id_at_time IS DISTINCT FROM OLD.root_organization_id_at_time
        OR NEW.project_scope_id IS DISTINCT FROM OLD.project_scope_id
        OR NEW.project_path_at_time IS DISTINCT FROM OLD.project_path_at_time
        OR NEW.requested_by_principal_id IS DISTINCT FROM OLD.requested_by_principal_id
        OR NEW.organization_snapshot IS DISTINCT FROM OLD.organization_snapshot
        OR NEW.target_snapshot IS DISTINCT FROM OLD.target_snapshot
        OR NEW.requested_at IS DISTINCT FROM OLD.requested_at
    THEN
        RAISE EXCEPTION 'organization deletion job identity is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organization_deletion_jobs_retain_identity
BEFORE UPDATE OR DELETE ON organization_deletion_jobs
FOR EACH ROW EXECUTE FUNCTION retain_organization_deletion_job_identity();

CREATE TRIGGER organization_deletion_job_units_immutable
BEFORE UPDATE OR DELETE ON organization_deletion_job_units
FOR EACH ROW EXECUTE FUNCTION reject_organization_delete_job_history_change();

CREATE FUNCTION retain_organization_deletion_target_snapshot()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'organization deletion target history is immutable';
    END IF;
    IF NEW.job_id IS DISTINCT FROM OLD.job_id
        OR NEW.target_id_at_time IS DISTINCT FROM OLD.target_id_at_time
        OR NEW.organization_id_at_time IS DISTINCT FROM OLD.organization_id_at_time
        OR NEW.canonical_target_snapshot IS DISTINCT FROM OLD.canonical_target_snapshot
        OR NOT (OLD.live_target_id IS NOT NULL AND NEW.live_target_id IS NULL)
    THEN
        RAISE EXCEPTION 'organization deletion target history is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organization_deletion_job_targets_immutable
BEFORE UPDATE OR DELETE ON organization_deletion_job_targets
FOR EACH ROW EXECUTE FUNCTION retain_organization_deletion_target_snapshot();

CREATE TRIGGER organization_deletion_job_invalidations_immutable
BEFORE UPDATE OR DELETE ON organization_deletion_job_invalidations
FOR EACH ROW EXECUTE FUNCTION reject_organization_delete_job_history_change();

CREATE TRIGGER organization_deletion_job_state_history_immutable
BEFORE UPDATE OR DELETE ON organization_deletion_job_state_history
FOR EACH ROW EXECUTE FUNCTION reject_organization_delete_job_history_change();

-- An active job makes every organization in its frozen subtree read-only.
-- Final hard-delete is allowed only after the filesystem result is durable.
CREATE FUNCTION enforce_organization_deletion_read_only()
RETURNS trigger AS $$
DECLARE
    active_state TEXT;
    row_id UUID;
BEGIN
    row_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    SELECT job.state INTO active_state
      FROM organization_deletion_job_units AS unit
      JOIN organization_deletion_jobs AS job ON job.id = unit.job_id
     WHERE unit.organization_id_at_time = row_id
       AND job.state <> 'hard_delete_committed'
     LIMIT 1;
    IF active_state IS NULL THEN
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    END IF;
    IF TG_OP = 'DELETE' AND active_state = 'artifact_cleanup_succeeded' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'organization_deletion_in_progress' USING ERRCODE = '55000';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organizations_deletion_read_only
BEFORE UPDATE OR DELETE ON organizations
FOR EACH ROW EXECUTE FUNCTION enforce_organization_deletion_read_only();

CREATE FUNCTION reject_child_attachment_to_deleting_subtree()
RETURNS trigger AS $$
BEGIN
    IF NEW.parent_id IS NOT NULL AND EXISTS (
        SELECT 1
          FROM organization_deletion_job_units AS unit
          JOIN organization_deletion_jobs AS job ON job.id = unit.job_id
         WHERE unit.organization_id_at_time = NEW.parent_id
           AND job.state <> 'hard_delete_committed'
    ) THEN
        RAISE EXCEPTION 'organization_deletion_in_progress' USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organizations_reject_child_attachment_while_deleting
BEFORE INSERT OR UPDATE OF parent_id ON organizations
FOR EACH ROW EXECUTE FUNCTION reject_child_attachment_to_deleting_subtree();

-- The artifact plan is frozen from live targets in the deletion request
-- transaction.  Keep those rows immutable until hard delete, otherwise a new
-- or re-bound target could escape the committed filesystem cleanup snapshot.
CREATE FUNCTION enforce_deleting_organization_target_read_only()
RETURNS trigger AS $$
DECLARE
    old_organization_id UUID;
    new_organization_id UUID;
    active_state TEXT;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        old_organization_id := OLD.organization_id;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        new_organization_id := NEW.organization_id;
    END IF;
    SELECT job.state INTO active_state
      FROM organization_deletion_job_units AS unit
      JOIN organization_deletion_jobs AS job ON job.id = unit.job_id
     WHERE unit.organization_id_at_time IN (
               old_organization_id,
               new_organization_id
           )
       AND job.state <> 'hard_delete_committed'
     ORDER BY job.requested_at,job.id
     LIMIT 1;
    IF active_state IS NULL THEN
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    END IF;
    IF TG_OP = 'DELETE' AND active_state = 'artifact_cleanup_succeeded' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'organization_deletion_in_progress' USING ERRCODE = '55000';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER targets_deleting_organization_read_only
BEFORE INSERT OR UPDATE OF organization_id,target_type,value,project_path OR DELETE ON targets
FOR EACH ROW EXECUTE FUNCTION enforce_deleting_organization_target_read_only();

CREATE INDEX organization_deletion_jobs_claim
    ON organization_deletion_jobs(
        state,artifact_retry_not_before,hard_delete_retry_not_before,
        lease_expires_at,requested_at
    )
    WHERE state <> 'hard_delete_committed';
CREATE INDEX organization_deletion_job_units_org
    ON organization_deletion_job_units(organization_id_at_time,job_id);
CREATE INDEX organization_deletion_job_targets_live
    ON organization_deletion_job_targets(live_target_id)
    WHERE live_target_id IS NOT NULL;
