-- Bind one unified Investigation analysis work item to the exact Candidate
-- snapshot and ordinal-zero analysis attempt that it owns.  This is an
-- authority bridge only: it deliberately carries no legacy attack_candidate
-- scheduler, role, lane, or wave identity.

CREATE TABLE investigation_analysis_attempt_bindings (
    binding_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL DEFAULT 'investigation' CHECK (stage_kind='investigation'),
    work_id UUID NOT NULL,
    candidate_snapshot_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    attempt_ordinal INTEGER NOT NULL DEFAULT 0 CHECK (attempt_ordinal=0),
    contract_version TEXT NOT NULL DEFAULT 'unified_investigation_analysis_binding.v1'
        CHECK (contract_version='unified_investigation_analysis_binding.v1'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(authority_id,work_id),
    UNIQUE(analysis_attempt_id),
    UNIQUE(
        binding_id,authority_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id,work_id,candidate_snapshot_id,
        analysis_attempt_id
    ),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_stage_run_authorities(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_run_unit_id,operation_id,stage_execution_id,organization_id,stage_kind
    ) REFERENCES stage_run_units(
        id,operation_id,stage_execution_id,organization_id,stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        work_id,authority_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ) REFERENCES investigation_run_work_items(
        work_id,authority_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(candidate_snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_snapshots(snapshot_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(
        analysis_attempt_id,candidate_snapshot_id,operation_id,organization_id
    ) REFERENCES candidate_analysis_attempts(
        analysis_attempt_id,snapshot_id,operation_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_validate_analysis_attempt_binding()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    bound_work_kind TEXT;
    bound_snapshot_scope_id UUID;
    bound_snapshot_status TEXT;
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

    SELECT snapshot.scope_snapshot_id,snapshot.snapshot_status
      INTO STRICT bound_snapshot_scope_id,bound_snapshot_status
      FROM candidate_analysis_snapshots snapshot
     WHERE snapshot.snapshot_id=NEW.candidate_snapshot_id
       AND snapshot.operation_id=NEW.operation_id
       AND snapshot.organization_id=NEW.organization_id
     FOR SHARE;
    IF bound_snapshot_scope_id IS DISTINCT FROM NEW.scope_snapshot_id THEN
        RAISE EXCEPTION 'INVESTIGATION_ANALYSIS_BINDING_SCOPE_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    IF bound_snapshot_status<>'sealed_ready' THEN
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

CREATE TRIGGER investigation_analysis_attempt_bindings_validate
BEFORE INSERT ON investigation_analysis_attempt_bindings
FOR EACH ROW EXECUTE FUNCTION investigation_validate_analysis_attempt_binding();

CREATE TRIGGER investigation_analysis_attempt_bindings_append_only
BEFORE UPDATE OR DELETE ON investigation_analysis_attempt_bindings
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
