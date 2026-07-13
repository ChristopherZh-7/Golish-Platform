-- P7a cleanup obligation kernel. This migration makes every persisted
-- side-effect action inseparable from one retained cleanup obligation while
-- still exposing no executable cleanup or mutation tool.

ALTER TABLE post_exploit_actions
    ADD COLUMN cleanup_obligation_id UUID,
    ADD COLUMN prepared_by_principal_id UUID
        REFERENCES operator_principals(id) ON DELETE RESTRICT;

CREATE TABLE cleanup_obligations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    source_action_id UUID NOT NULL UNIQUE,
    source_action_plan_hash TEXT NOT NULL CHECK (
        source_action_plan_hash ~ '^[0-9a-f]{64}$'
    ),
    affected_resource_snapshot JSONB NOT NULL CHECK (
        jsonb_typeof(affected_resource_snapshot) = 'object'
    ),
    resource_identity_hash TEXT NOT NULL CHECK (
        resource_identity_hash ~ '^[0-9a-f]{64}$'
    ),
    cleanup_strategy JSONB NOT NULL CHECK (
        jsonb_typeof(cleanup_strategy) = 'object'
    ),
    proof_requirements JSONB NOT NULL CHECK (
        jsonb_typeof(proof_requirements) = 'array'
        AND jsonb_array_length(proof_requirements) BETWEEN 1 AND 64
    ),
    deadline TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (
        status IN ('open','in_progress','verified_absent','blocked','waived_by_user')
    ),
    residual_risk JSONB CHECK (
        residual_risk IS NULL OR jsonb_typeof(residual_risk) = 'object'
    ),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (
        id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time,source_action_plan_hash
    ),
    UNIQUE (
        id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time
    ),
    FOREIGN KEY (operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id,operation_id)
        REFERENCES operation_org_scope_snapshots(id,operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id,organization_id_at_time)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_action_id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time,source_action_plan_hash
    ) REFERENCES post_exploit_actions(
        id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time,plan_hash
    ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (status IN ('verified_absent','blocked','waived_by_user') AND terminal_at IS NOT NULL)
        OR (status IN ('open','in_progress') AND terminal_at IS NULL)
    ),
    CHECK ((status = 'waived_by_user') = (residual_risk IS NOT NULL)),
    CHECK (deadline > created_at)
);

ALTER TABLE post_exploit_actions
    ADD CONSTRAINT post_exploit_actions_cleanup_obligation_fk
    FOREIGN KEY (
        cleanup_obligation_id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time,plan_hash
    ) REFERENCES cleanup_obligations(
        id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time,source_action_plan_hash
    ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

DROP TRIGGER post_exploit_actions_p6a_side_effect_boundary ON post_exploit_actions;
DROP FUNCTION enforce_p6a_side_effect_boundary();

CREATE FUNCTION enforce_cleanup_obligation_boundary()
RETURNS trigger AS $$
BEGIN
    IF NEW.side_effect_class = 'none' THEN
        IF NEW.cleanup_obligation_id IS NOT NULL
            OR NEW.prepared_by_principal_id IS NOT NULL
        THEN
            RAISE EXCEPTION 'side-effect-free actions cannot claim a cleanup obligation';
        END IF;
    ELSIF NEW.cleanup_obligation_id IS NULL
        OR NEW.prepared_by_principal_id IS NULL
    THEN
        RAISE EXCEPTION 'cleanup_obligation_required' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER post_exploit_actions_cleanup_obligation_boundary
BEFORE INSERT OR UPDATE OF side_effect_class,cleanup_obligation_id,prepared_by_principal_id
ON post_exploit_actions
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_obligation_boundary();

CREATE FUNCTION enforce_cleanup_action_back_reference()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM post_exploit_actions AS action
         WHERE action.id = NEW.source_action_id
           AND action.operation_id = NEW.operation_id
           AND action.project_scope_id = NEW.project_scope_id
           AND action.scope_snapshot_id = NEW.scope_snapshot_id
           AND action.organization_id_at_time = NEW.organization_id_at_time
           AND action.plan_hash = NEW.source_action_plan_hash
           AND action.side_effect_class <> 'none'
           AND action.cleanup_obligation_id = NEW.id
    ) THEN
        RAISE EXCEPTION 'cleanup obligation lacks its exact side-effect action back-reference';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER cleanup_obligations_exact_action_back_reference
AFTER INSERT OR UPDATE OF source_action_id,source_action_plan_hash
ON cleanup_obligations DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_cleanup_action_back_reference();

CREATE TABLE cleanup_obligation_evidence (
    obligation_id UUID NOT NULL REFERENCES cleanup_obligations(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('source','strategy','support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (obligation_id,evidence_id,role)
);

CREATE TABLE cleanup_attempts (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    obligation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    status TEXT NOT NULL DEFAULT 'claimed' CHECK (
        status IN (
            'claimed','executing','cleaned_pending_verification',
            'verified_absent','verification_failed','execution_failed'
        )
    ),
    lease_token UUID NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    result JSONB CHECK (result IS NULL OR jsonb_typeof(result) = 'object'),
    terminal_note TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    UNIQUE (obligation_id,ordinal),
    UNIQUE (id,obligation_id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time),
    FOREIGN KEY (
        obligation_id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time
    ) REFERENCES cleanup_obligations(
        id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time
    ) ON DELETE RESTRICT,
    CHECK (
        (status IN ('claimed','executing','cleaned_pending_verification') AND completed_at IS NULL)
        OR (status IN ('verified_absent','verification_failed','execution_failed') AND completed_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX cleanup_attempts_one_live
    ON cleanup_attempts(obligation_id)
    WHERE status IN ('claimed','executing','cleaned_pending_verification');

CREATE TABLE cleanup_attempt_evidence (
    attempt_id UUID NOT NULL REFERENCES cleanup_attempts(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('execution','result','support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (attempt_id,evidence_id,role)
);

CREATE TABLE cleanup_absence_checks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    obligation_id UUID NOT NULL,
    cleanup_attempt_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    verifier_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    verifier_key TEXT NOT NULL CHECK (BTRIM(verifier_key) <> ''),
    resource_identity_hash TEXT NOT NULL CHECK (
        resource_identity_hash ~ '^[0-9a-f]{64}$'
    ),
    disposition TEXT NOT NULL CHECK (
        disposition IN ('absent','still_present','inconclusive')
    ),
    checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (cleanup_attempt_id),
    FOREIGN KEY (
        cleanup_attempt_id,obligation_id,operation_id,project_scope_id,
        scope_snapshot_id,organization_id_at_time
    ) REFERENCES cleanup_attempts(
        id,obligation_id,operation_id,project_scope_id,
        scope_snapshot_id,organization_id_at_time
    ) ON DELETE RESTRICT
);

CREATE TABLE cleanup_absence_check_evidence (
    absence_check_id UUID NOT NULL REFERENCES cleanup_absence_checks(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('absence','presence','inconclusive','support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (absence_check_id,evidence_id,role)
);

CREATE TABLE cleanup_waivers (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    obligation_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    decided_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (
        BTRIM(reason) <> '' AND LENGTH(reason) <= 4096
    ),
    residual_risk JSONB NOT NULL CHECK (jsonb_typeof(residual_risk) = 'object'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (
        obligation_id,operation_id,project_scope_id,scope_snapshot_id,
        organization_id_at_time
    ) REFERENCES cleanup_obligations(
        id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time
    ) ON DELETE RESTRICT
);

CREATE TABLE cleanup_waiver_evidence (
    waiver_id UUID NOT NULL REFERENCES cleanup_waivers(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('decision','residual','support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (waiver_id,evidence_id,role)
);

CREATE FUNCTION validate_cleanup_absence_identity()
RETURNS trigger AS $$
DECLARE
    obligation_hash TEXT;
    executor_worker UUID;
BEGIN
    SELECT obligation.resource_identity_hash,attempt.worker_run_id
      INTO obligation_hash,executor_worker
      FROM cleanup_obligations AS obligation
      JOIN cleanup_attempts AS attempt ON attempt.id = NEW.cleanup_attempt_id
     WHERE obligation.id = NEW.obligation_id
       AND attempt.obligation_id = obligation.id;
    IF obligation_hash IS NULL
        OR obligation_hash IS DISTINCT FROM NEW.resource_identity_hash
        OR (
            executor_worker IS NOT NULL
            AND NEW.verifier_worker_run_id IS NOT NULL
            AND executor_worker = NEW.verifier_worker_run_id
        )
    THEN
        RAISE EXCEPTION 'cleanup absence verifier is not independent or identity drifted';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER cleanup_absence_checks_validate_identity
BEFORE INSERT ON cleanup_absence_checks
FOR EACH ROW EXECUTE FUNCTION validate_cleanup_absence_identity();

CREATE FUNCTION retain_cleanup_identity()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'cleanup retained rows cannot be deleted';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.project_scope_id IS DISTINCT FROM OLD.project_scope_id
        OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
        OR NEW.organization_id_at_time IS DISTINCT FROM OLD.organization_id_at_time
    THEN
        RAISE EXCEPTION 'cleanup retained identity is immutable';
    END IF;
    IF TG_TABLE_NAME = 'cleanup_obligations' THEN
        NEW.updated_at = NOW();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER cleanup_obligations_retain_identity
BEFORE UPDATE OR DELETE ON cleanup_obligations
FOR EACH ROW EXECUTE FUNCTION retain_cleanup_identity();

CREATE TRIGGER cleanup_attempts_retain_identity
BEFORE UPDATE OR DELETE ON cleanup_attempts
FOR EACH ROW EXECUTE FUNCTION retain_cleanup_identity();

CREATE FUNCTION reject_cleanup_fact_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'cleanup fact history is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER cleanup_absence_checks_immutable
BEFORE UPDATE OR DELETE ON cleanup_absence_checks
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE TRIGGER cleanup_waivers_immutable
BEFORE UPDATE OR DELETE ON cleanup_waivers
FOR EACH ROW EXECUTE FUNCTION reject_cleanup_fact_change();

CREATE INDEX cleanup_obligations_scope_status_idx
    ON cleanup_obligations(project_scope_id,organization_id_at_time,status,deadline);
CREATE INDEX cleanup_attempts_scope_status_idx
    ON cleanup_attempts(project_scope_id,organization_id_at_time,status,lease_expires_at);
