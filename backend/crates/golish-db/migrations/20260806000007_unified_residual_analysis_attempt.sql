-- A residual-ready snapshot is executable only inside the unified
-- Investigation topology.  Legacy Candidate attempts remain all-fresh-only.
CREATE OR REPLACE FUNCTION candidate_attempt_requires_ready_snapshot()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM candidate_analysis_snapshots snapshot
          JOIN operation_state operation
            ON operation.operation_id=snapshot.operation_id
         WHERE snapshot.snapshot_id=NEW.snapshot_id
           AND (
                snapshot.snapshot_status='sealed_ready'
                OR (
                    snapshot.snapshot_status='sealed_analysis_ready_with_residuals'
                    AND operation.stage_topology_contract='unified_investigation_v1'
                )
           )
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ANALYSIS_SNAPSHOT_NOT_READY' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION investigation_validate_analysis_attempt_binding()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    bound_work_kind TEXT;
    bound_snapshot_scope_id UUID;
    bound_snapshot_status TEXT;
    bound_topology TEXT;
    bound_attempt_ordinal INTEGER;
BEGIN
    SELECT work.work_kind
      INTO STRICT bound_work_kind
      FROM investigation_run_work_items work
     WHERE work.work_id=NEW.work_id
       AND work.authority_id=NEW.authority_id
       AND work.operation_id=NEW.operation_id
       AND work.stage_execution_id=NEW.stage_execution_id
       AND work.stage_run_unit_id=NEW.stage_run_unit_id
       AND work.organization_id=NEW.organization_id
     FOR SHARE;
    IF bound_work_kind<>'analysis' THEN
        RAISE EXCEPTION 'INVESTIGATION_ANALYSIS_BINDING_REQUIRES_ANALYSIS_WORK'
            USING ERRCODE='23514';
    END IF;

    SELECT snapshot.scope_snapshot_id,snapshot.snapshot_status,
           operation.stage_topology_contract
      INTO STRICT bound_snapshot_scope_id,bound_snapshot_status,bound_topology
      FROM candidate_analysis_snapshots snapshot
      JOIN operation_state operation ON operation.operation_id=snapshot.operation_id
     WHERE snapshot.snapshot_id=NEW.candidate_snapshot_id
       AND snapshot.operation_id=NEW.operation_id
       AND snapshot.organization_id=NEW.organization_id
     FOR SHARE OF snapshot,operation;
    IF bound_snapshot_scope_id IS DISTINCT FROM NEW.scope_snapshot_id THEN
        RAISE EXCEPTION 'INVESTIGATION_ANALYSIS_BINDING_SCOPE_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    IF bound_snapshot_status<>'sealed_ready'
       AND NOT (
           bound_snapshot_status='sealed_analysis_ready_with_residuals'
           AND bound_topology='unified_investigation_v1'
       ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ANALYSIS_BINDING_SNAPSHOT_NOT_READY'
            USING ERRCODE='23514';
    END IF;

    SELECT attempt.attempt_ordinal
      INTO STRICT bound_attempt_ordinal
      FROM candidate_analysis_attempts attempt
     WHERE attempt.analysis_attempt_id=NEW.analysis_attempt_id
       AND attempt.snapshot_id=NEW.candidate_snapshot_id
       AND attempt.operation_id=NEW.operation_id
       AND attempt.organization_id=NEW.organization_id
     FOR SHARE;
    IF bound_attempt_ordinal<>0 OR bound_attempt_ordinal<>NEW.attempt_ordinal THEN
        RAISE EXCEPTION 'INVESTIGATION_ANALYSIS_BINDING_REQUIRES_ORDINAL_ZERO_ATTEMPT'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION candidate_attempt_requires_ready_snapshot() IS
'Legacy Candidate requires sealed_ready; unified Investigation may execute an explicitly residual-ready snapshot without converting gaps into checked-empty truth.';
