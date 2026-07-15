-- Forward repair for the audited edit to 20260714000002.
--
-- The prior migration checksum is repaired only for one exact old/new pair.
-- This migration then installs the two function bodies that differed from the
-- already-applied schema, preserving SQLx migration immutability going forward.

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
