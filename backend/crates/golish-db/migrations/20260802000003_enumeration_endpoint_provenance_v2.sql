-- Enumeration agent-team JS/API analysis V2.
--
-- Additive only: immutable occurrence truth is recorded beside the legacy
-- `api_endpoints` aggregate.  The aggregate is mutated only by the guarded
-- production projector; shadow and unresolved observations never project.

-- ---------------------------------------------------------------------------
-- Server-owned rollout and operation-frozen contract.
-- ---------------------------------------------------------------------------

CREATE TABLE enumeration_analysis_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    new_operation_contract TEXT NOT NULL CHECK (
        new_operation_contract IN (
            'legacy_v1','agent_team_v2_shadow','agent_team_v2'
        )
    ),
    generation BIGINT NOT NULL DEFAULT 0 CHECK (generation>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

INSERT INTO enumeration_analysis_rollout(singleton,new_operation_contract)
VALUES(TRUE,'legacy_v1');

CREATE TABLE enumeration_analysis_rollout_review_reports (
    id UUID PRIMARY KEY,
    stable_report_request_id UUID NOT NULL UNIQUE,
    predecessor_contract TEXT NOT NULL,
    promoted_contract TEXT NOT NULL,
    fixture_run_id UUID NOT NULL,
    focused_test_count BIGINT NOT NULL CHECK (focused_test_count>0),
    mismatch_count BIGINT NOT NULL CHECK (mismatch_count=0),
    unresolved_blocker_count BIGINT NOT NULL CHECK (unresolved_blocker_count=0),
    review_decision TEXT NOT NULL CHECK (review_decision='approved'),
    reviewed_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    audit_log_id BIGINT NOT NULL UNIQUE REFERENCES audit_log(id) ON DELETE RESTRICT,
    report_hash TEXT NOT NULL CHECK (report_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (predecessor_contract='legacy_v1' AND promoted_contract='agent_team_v2_shadow')
        OR (predecessor_contract='agent_team_v2_shadow' AND promoted_contract='agent_team_v2')
    )
);

CREATE TABLE enumeration_analysis_rollout_promotion_receipts (
    id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    predecessor_contract TEXT NOT NULL,
    promoted_contract TEXT NOT NULL,
    predecessor_generation BIGINT NOT NULL CHECK (predecessor_generation>=0),
    promoted_generation BIGINT NOT NULL CHECK (promoted_generation=predecessor_generation+1),
    review_report_id UUID NOT NULL
        REFERENCES enumeration_analysis_rollout_review_reports(id) ON DELETE RESTRICT,
    review_report_hash TEXT NOT NULL CHECK (review_report_hash ~ '^sha256:[0-9a-f]{64}$'),
    promoted_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    audit_log_id BIGINT NOT NULL UNIQUE REFERENCES audit_log(id) ON DELETE RESTRICT,
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (predecessor_contract='legacy_v1' AND promoted_contract='agent_team_v2_shadow')
        OR (predecessor_contract='agent_team_v2_shadow' AND promoted_contract='agent_team_v2_production')
    )
);

CREATE FUNCTION enumeration_reject_immutable()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '%',TG_ARGV[0] USING ERRCODE='23514';
END;
$$;

CREATE TRIGGER enumeration_analysis_rollout_review_report_immutable
BEFORE UPDATE OR DELETE ON enumeration_analysis_rollout_review_reports
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_analysis_rollout_review_report_immutable');
CREATE TRIGGER enumeration_analysis_rollout_receipt_immutable
BEFORE UPDATE OR DELETE ON enumeration_analysis_rollout_promotion_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_analysis_rollout_receipt_immutable');

CREATE FUNCTION enumeration_guard_rollout_audit_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM enumeration_analysis_rollout_review_reports report
         WHERE report.audit_log_id=OLD.id
        UNION ALL
        SELECT 1 FROM enumeration_analysis_rollout_promotion_receipts receipt
         WHERE receipt.audit_log_id=OLD.id
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_audit_receipt_immutable'
            USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE TRIGGER enumeration_analysis_rollout_audit_receipt_immutable
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_rollout_audit_receipt();

CREATE FUNCTION enumeration_validate_rollout_review_report()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('golish.enumeration_rollout_writer',TRUE)<>'review' THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_review_report_internal_only'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM operator_principals principal
         WHERE principal.id=NEW.reviewed_by_principal_id
           AND principal.principal_kind='local_operator' AND principal.active
         FOR SHARE
    ) OR NOT EXISTS (
        SELECT 1 FROM audit_log audit
         WHERE audit.id=NEW.audit_log_id
           AND audit.action='enumeration_analysis_review_approved'
           AND audit.category='rollout'
           AND audit.entity_type='enumeration_analysis_review_report'
           AND audit.entity_id=NEW.id::TEXT
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_review_authority_invalid'
            USING ERRCODE='23514';
    END IF;
    NEW.report_hash := tool_truth_sha256(jsonb_build_object(
        'domain','enumeration_analysis_review_report.v1',
        'id',NEW.id,
        'stable_report_request_id',NEW.stable_report_request_id,
        'predecessor_contract',NEW.predecessor_contract,
        'promoted_contract',NEW.promoted_contract,
        'fixture_run_id',NEW.fixture_run_id,
        'focused_test_count',NEW.focused_test_count,
        'mismatch_count',NEW.mismatch_count,
        'unresolved_blocker_count',NEW.unresolved_blocker_count,
        'review_decision',NEW.review_decision,
        'reviewed_by_principal_id',NEW.reviewed_by_principal_id,
        'audit_log_id',NEW.audit_log_id
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_analysis_rollout_review_report_validate
BEFORE INSERT ON enumeration_analysis_rollout_review_reports
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_rollout_review_report();

CREATE FUNCTION enumeration_validate_rollout_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('golish.enumeration_rollout_writer',TRUE)<>'promotion' THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_receipt_internal_only'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_analysis_rollout_review_reports report
        JOIN operator_principals principal
          ON principal.id=NEW.promoted_by_principal_id
         AND principal.principal_kind='local_operator' AND principal.active
        JOIN audit_log audit
          ON audit.id=NEW.audit_log_id
         AND audit.action='enumeration_analysis_rollout_promoted'
         AND audit.category='rollout'
         AND audit.entity_type='enumeration_analysis_rollout_promotion'
         AND audit.entity_id=NEW.id::TEXT
        WHERE report.id=NEW.review_report_id
          AND report.report_hash=NEW.review_report_hash
          AND report.reviewed_by_principal_id=NEW.promoted_by_principal_id
          AND report.predecessor_contract=NEW.predecessor_contract
          AND report.promoted_contract=NEW.promoted_contract
          AND report.review_decision='approved'
        FOR SHARE OF report,principal,audit
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_review_receipt_invalid'
            USING ERRCODE='23514';
    END IF;
    NEW.receipt_hash := tool_truth_sha256(jsonb_build_object(
        'domain','enumeration_analysis_rollout_promotion_receipt.v1',
        'stable_request_id',NEW.stable_request_id,
        'predecessor_contract',NEW.predecessor_contract,
        'promoted_contract',NEW.promoted_contract,
        'predecessor_generation',NEW.predecessor_generation,
        'promoted_generation',NEW.promoted_generation,
        'review_report_id',NEW.review_report_id,
        'review_report_hash',NEW.review_report_hash,
        'promoted_by_principal_id',NEW.promoted_by_principal_id,
        'audit_log_id',NEW.audit_log_id
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_analysis_rollout_receipt_validate
BEFORE INSERT ON enumeration_analysis_rollout_promotion_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_rollout_receipt();

CREATE FUNCTION enumeration_guard_rollout_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE promotion_id UUID;
BEGIN
    IF TG_OP<>'UPDATE' THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    promotion_id := NULLIF(
        current_setting('golish.enumeration_rollout_promotion_receipt_id',TRUE),''
    )::UUID;
    IF promotion_id IS NULL OR NOT EXISTS (
        SELECT 1 FROM enumeration_analysis_rollout_promotion_receipts receipt
         WHERE receipt.id=promotion_id
           AND receipt.predecessor_contract=OLD.new_operation_contract
           AND receipt.promoted_contract=NEW.new_operation_contract
           AND receipt.predecessor_generation=OLD.generation
           AND receipt.promoted_generation=NEW.generation
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    NEW.singleton := TRUE;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;

CREATE TRIGGER enumeration_analysis_rollout_mutation_guard
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_analysis_rollout
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_rollout_mutation();

CREATE FUNCTION record_enumeration_analysis_review_report(
    p_report_id UUID,
    p_stable_report_request_id UUID,
    p_predecessor_contract TEXT,
    p_promoted_contract TEXT,
    p_fixture_run_id UUID,
    p_focused_test_count BIGINT,
    p_mismatch_count BIGINT,
    p_unresolved_blocker_count BIGINT,
    p_reviewed_by_principal_id UUID
) RETURNS enumeration_analysis_rollout_review_reports
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE report enumeration_analysis_rollout_review_reports%ROWTYPE;
DECLARE audit_id BIGINT;
BEGIN
    SELECT * INTO report FROM enumeration_analysis_rollout_review_reports
     WHERE stable_report_request_id=p_stable_report_request_id;
    IF FOUND THEN
        IF report.id<>p_report_id
           OR report.predecessor_contract<>p_predecessor_contract
           OR report.promoted_contract<>p_promoted_contract
           OR report.fixture_run_id<>p_fixture_run_id
           OR report.focused_test_count<>p_focused_test_count
           OR report.mismatch_count<>p_mismatch_count
           OR report.unresolved_blocker_count<>p_unresolved_blocker_count
           OR report.reviewed_by_principal_id<>p_reviewed_by_principal_id THEN
            RAISE EXCEPTION 'enumeration_analysis_review_report_idempotency_conflict'
                USING ERRCODE='23514';
        END IF;
        RETURN report;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM operator_principals principal
         WHERE principal.id=p_reviewed_by_principal_id
           AND principal.principal_kind='local_operator' AND principal.active
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_review_principal_invalid'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO audit_log(action,category,details,entity_type,entity_id)
    VALUES(
        'enumeration_analysis_review_approved','rollout',
        'server-authored structured Enumeration JS/API rollout review',
        'enumeration_analysis_review_report',p_report_id::TEXT
    ) RETURNING id INTO audit_id;
    PERFORM set_config('golish.enumeration_rollout_writer','review',TRUE);
    INSERT INTO enumeration_analysis_rollout_review_reports(
        id,stable_report_request_id,predecessor_contract,promoted_contract,
        fixture_run_id,focused_test_count,mismatch_count,unresolved_blocker_count,
        review_decision,reviewed_by_principal_id,audit_log_id,report_hash
    ) VALUES(
        p_report_id,p_stable_report_request_id,p_predecessor_contract,p_promoted_contract,
        p_fixture_run_id,p_focused_test_count,p_mismatch_count,p_unresolved_blocker_count,
        'approved',p_reviewed_by_principal_id,audit_id,
        'sha256:'||repeat('0',64)
    ) RETURNING * INTO report;
    RETURN report;
END;
$$;

CREATE FUNCTION promote_enumeration_analysis_rollout(
    p_receipt_id UUID,
    p_stable_request_id UUID,
    p_expected_contract TEXT,
    p_promoted_contract TEXT,
    p_review_report_id UUID,
    p_promoted_by_principal_id UUID
) RETURNS enumeration_analysis_rollout_promotion_receipts
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE rollout enumeration_analysis_rollout%ROWTYPE;
DECLARE receipt enumeration_analysis_rollout_promotion_receipts%ROWTYPE;
DECLARE review_report enumeration_analysis_rollout_review_reports%ROWTYPE;
DECLARE audit_id BIGINT;
BEGIN
    SELECT * INTO receipt FROM enumeration_analysis_rollout_promotion_receipts
     WHERE stable_request_id=p_stable_request_id;
    IF FOUND THEN
        IF receipt.id<>p_receipt_id
           OR receipt.predecessor_contract<>p_expected_contract
           OR receipt.promoted_contract<>p_promoted_contract
           OR receipt.review_report_id<>p_review_report_id
           OR receipt.promoted_by_principal_id<>p_promoted_by_principal_id THEN
            RAISE EXCEPTION 'enumeration_analysis_rollout_idempotency_conflict' USING ERRCODE='23514';
        END IF;
        RETURN receipt;
    END IF;
    SELECT * INTO rollout FROM enumeration_analysis_rollout WHERE singleton FOR UPDATE;
    IF rollout.new_operation_contract IS DISTINCT FROM p_expected_contract THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_predecessor_mismatch' USING ERRCODE='40001';
    END IF;
    SELECT * INTO review_report FROM enumeration_analysis_rollout_review_reports
     WHERE id=p_review_report_id
       AND predecessor_contract=p_expected_contract
       AND promoted_contract=p_promoted_contract
       AND reviewed_by_principal_id=p_promoted_by_principal_id
       AND review_decision='approved'
     FOR SHARE;
    IF NOT FOUND OR NOT EXISTS (
        SELECT 1 FROM operator_principals principal
         WHERE principal.id=p_promoted_by_principal_id
           AND principal.principal_kind='local_operator' AND principal.active
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_analysis_rollout_review_required' USING ERRCODE='23514';
    END IF;
    INSERT INTO audit_log(action,category,details,entity_type,entity_id)
    VALUES(
        'enumeration_analysis_rollout_promoted','rollout',
        'server-authored Enumeration JS/API rollout promotion',
        'enumeration_analysis_rollout_promotion',p_receipt_id::TEXT
    ) RETURNING id INTO audit_id;
    PERFORM set_config('golish.enumeration_rollout_writer','promotion',TRUE);
    INSERT INTO enumeration_analysis_rollout_promotion_receipts(
        id,stable_request_id,predecessor_contract,promoted_contract,
        predecessor_generation,promoted_generation,review_report_id,review_report_hash,
        promoted_by_principal_id,audit_log_id,receipt_hash
    ) VALUES (
        p_receipt_id,p_stable_request_id,p_expected_contract,p_promoted_contract,
        rollout.generation,rollout.generation+1,p_review_report_id,review_report.report_hash,
        p_promoted_by_principal_id,audit_id,'sha256:'||repeat('0',64)
    ) RETURNING * INTO receipt;
    PERFORM set_config(
        'golish.enumeration_rollout_promotion_receipt_id',p_receipt_id::TEXT,TRUE
    );
    UPDATE enumeration_analysis_rollout
       SET new_operation_contract=p_promoted_contract,
           generation=rollout.generation+1,
           updated_at=statement_timestamp()
     WHERE singleton;
    RETURN receipt;
END;
$$;

REVOKE ALL ON FUNCTION record_enumeration_analysis_review_report(
    UUID,UUID,TEXT,TEXT,UUID,BIGINT,BIGINT,BIGINT,UUID
) FROM PUBLIC;
REVOKE ALL ON FUNCTION promote_enumeration_analysis_rollout(
    UUID,UUID,TEXT,TEXT,UUID,UUID
) FROM PUBLIC;
REVOKE INSERT,UPDATE,DELETE ON enumeration_analysis_rollout FROM PUBLIC;
REVOKE INSERT,UPDATE,DELETE ON enumeration_analysis_rollout_review_reports FROM PUBLIC;
REVOKE INSERT,UPDATE,DELETE ON enumeration_analysis_rollout_promotion_receipts FROM PUBLIC;

ALTER TABLE operation_state ADD COLUMN enumeration_analysis_contract TEXT;
UPDATE operation_state SET enumeration_analysis_contract='legacy_v1';
ALTER TABLE operation_state
    ALTER COLUMN enumeration_analysis_contract SET NOT NULL,
    ADD CONSTRAINT operation_state_enumeration_analysis_contract_check CHECK (
        enumeration_analysis_contract IN (
            'legacy_v1','agent_team_v2_shadow','agent_team_v2'
        )
    );

CREATE FUNCTION enumeration_freeze_operation_contract()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE deployed TEXT;
BEGIN
    SELECT new_operation_contract INTO deployed
      FROM enumeration_analysis_rollout WHERE singleton FOR SHARE;
    NEW.enumeration_analysis_contract := deployed;
    RETURN NEW;
END;
$$;
CREATE TRIGGER operation_state_enumeration_contract_freeze
BEFORE INSERT ON operation_state
FOR EACH ROW EXECUTE FUNCTION enumeration_freeze_operation_contract();

CREATE FUNCTION enumeration_guard_operation_contract_immutable()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.enumeration_analysis_contract IS DISTINCT FROM OLD.enumeration_analysis_contract THEN
        RAISE EXCEPTION 'operation_enumeration_analysis_contract_immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER operation_state_enumeration_contract_immutable
BEFORE UPDATE OF enumeration_analysis_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_operation_contract_immutable();

-- Additive candidate keys let every V2 row carry and enforce its owner tuple.
ALTER TABLE targets ADD CONSTRAINT targets_enumeration_owner_unique
    UNIQUE(id,organization_id,project_path);
ALTER TABLE web_origins ADD CONSTRAINT web_origins_enumeration_owner_unique
    UNIQUE(id,organization_id,project_path);
ALTER TABLE tool_truth_execution_authorities
    ADD CONSTRAINT tool_truth_execution_authorities_enumeration_compound_unique
    UNIQUE(id,operation_id,project_scope_id,project_path_at_freeze,
           scope_snapshot_id,organization_id,stage_execution_id);
ALTER TABLE capability_execution_receipt_inputs
    ADD CONSTRAINT capability_execution_receipt_inputs_enumeration_compound_unique
    UNIQUE(id,receipt_id,execution_authority_id);

-- `seal_source_denominator(StageTeamUnit)` intentionally owns a host-stage
-- authority. Enumeration producers must not reuse that authority as if it
-- were a worker/tool call. The host clones the already sealed exact member
-- census into one worker-owned root and records this immutable bridge. All
-- script, candidate and parameter children then stay below that worker root,
-- so the generic same-authority parent FKs remain both useful and reachable.
CREATE TABLE enumeration_worker_authority_roots (
    id UUID PRIMARY KEY,
    stable_root_request_id UUID NOT NULL,
    source_root_denominator_id UUID NOT NULL,
    source_execution_authority_id UUID NOT NULL,
    source_denominator_hash TEXT NOT NULL CHECK (
        source_denominator_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    worker_root_denominator_id UUID NOT NULL,
    worker_execution_authority_id UUID NOT NULL,
    worker_denominator_hash TEXT NOT NULL CHECK (
        worker_denominator_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    worker_run_id UUID NOT NULL,
    worker_attempt_epoch BIGINT NOT NULL CHECK (worker_attempt_epoch>=0),
    lease_token UUID NOT NULL,
    source_tool_call_id UUID NOT NULL,
    source_member_count BIGINT NOT NULL CHECK (source_member_count>0),
    source_member_set_hash TEXT NOT NULL CHECK (
        source_member_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    root_seal_hash TEXT NOT NULL CHECK (root_seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,stable_root_request_id),
    UNIQUE(worker_execution_authority_id),
    UNIQUE(worker_root_denominator_id),
    UNIQUE(worker_root_denominator_id,worker_execution_authority_id),
    FOREIGN KEY(source_root_denominator_id,source_execution_authority_id,source_denominator_hash)
        REFERENCES coverage_denominators(id,execution_authority_id,denominator_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(worker_root_denominator_id,worker_execution_authority_id,worker_denominator_hash)
        REFERENCES coverage_denominators(id,execution_authority_id,denominator_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(worker_execution_authority_id,operation_id,project_scope_id,
                project_path_at_freeze,scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(source_tool_call_id,operation_id,stage_execution_id,stage_run_unit_id,
                organization_id,worker_run_id,worker_attempt_epoch,lease_token)
        REFERENCES tool_calls(id,operation_id,stage_execution_id,stage_run_unit_id,
                              organization_id,worker_run_id,attempt_epoch,lease_token)
        ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_worker_authority_root()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE source_authority tool_truth_execution_authorities%ROWTYPE;
DECLARE worker_authority tool_truth_execution_authorities%ROWTYPE;
DECLARE source_root coverage_denominators%ROWTYPE;
DECLARE worker_root coverage_denominators%ROWTYPE;
DECLARE expected_worker_denominator_hash TEXT;
BEGIN
    SELECT * INTO source_root FROM coverage_denominators
     WHERE id=NEW.source_root_denominator_id
       AND execution_authority_id=NEW.source_execution_authority_id
       AND denominator_hash=NEW.source_denominator_hash FOR SHARE;
    SELECT * INTO worker_root FROM coverage_denominators
     WHERE id=NEW.worker_root_denominator_id
       AND execution_authority_id=NEW.worker_execution_authority_id
       AND denominator_hash=NEW.worker_denominator_hash FOR SHARE;
    SELECT * INTO source_authority FROM tool_truth_execution_authorities
     WHERE id=NEW.source_execution_authority_id FOR SHARE;
    SELECT * INTO worker_authority FROM tool_truth_execution_authorities
     WHERE id=NEW.worker_execution_authority_id FOR SHARE;

    IF source_root.id IS NULL OR worker_root.id IS NULL
       OR source_root.denominator_kind<>'root' OR worker_root.denominator_kind<>'root'
       OR source_root.sealed_at IS NULL OR worker_root.sealed_at IS NULL
       OR source_root.member_count IS NULL OR source_root.member_count<=0
       OR source_root.member_count IS DISTINCT FROM worker_root.member_count
       OR source_root.member_set_hash IS DISTINCT FROM worker_root.member_set_hash
       OR source_root.input_manifest_hash<>worker_root.input_manifest_hash
       OR source_root.contract<>worker_root.contract
       OR source_authority.execution_owner_kind<>'host_stage'
       OR source_authority.execution_source_kind<>'stage_unit'
       OR source_authority.stage_run_unit_id IS NULL
       OR worker_authority.execution_owner_kind<>'worker_tool'
       OR worker_authority.execution_source_kind<>'stage_unit'
       OR worker_authority.stage_run_unit_id IS DISTINCT FROM source_authority.stage_run_unit_id
       OR NEW.source_execution_authority_id=NEW.worker_execution_authority_id
       OR NEW.operation_id<>worker_authority.operation_id
       OR NEW.project_scope_id<>worker_authority.project_scope_id
       OR NEW.project_path_at_freeze<>worker_authority.project_path_at_freeze
       OR NEW.scope_snapshot_id<>worker_authority.scope_snapshot_id
       OR NEW.organization_id<>worker_authority.organization_id
       OR NEW.stage_execution_id<>worker_authority.stage_execution_id
       OR NEW.stage_run_unit_id IS DISTINCT FROM worker_authority.stage_run_unit_id
       OR NEW.worker_run_id IS DISTINCT FROM worker_authority.worker_run_id
       OR NEW.worker_attempt_epoch IS DISTINCT FROM worker_authority.worker_attempt_epoch
       OR NEW.lease_token IS DISTINCT FROM worker_authority.lease_token
       OR NEW.source_tool_call_id IS DISTINCT FROM worker_authority.source_tool_call_id
       OR (worker_authority.operation_id,worker_authority.project_scope_id,
           worker_authority.project_path_at_freeze,worker_authority.scope_snapshot_id,
           worker_authority.organization_id,worker_authority.stage_execution_id)
          IS DISTINCT FROM
          (source_authority.operation_id,source_authority.project_scope_id,
           source_authority.project_path_at_freeze,source_authority.scope_snapshot_id,
           source_authority.organization_id,source_authority.stage_execution_id)
    THEN
        RAISE EXCEPTION 'enumeration_worker_root_authority_mismatch' USING ERRCODE='23514';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM stage_worker_runs worker
        JOIN tool_calls call ON call.id=NEW.source_tool_call_id
         AND call.worker_run_id=worker.id
         AND call.operation_id=worker.operation_id
         AND call.stage_execution_id=worker.stage_execution_id
         AND call.stage_run_unit_id=worker.stage_run_unit_id
         AND call.organization_id=worker.organization_id
         AND call.attempt_epoch=worker.attempt_epoch
         AND call.lease_token=worker.lease_token
        WHERE worker.id=NEW.worker_run_id
          AND worker.operation_id=NEW.operation_id
          AND worker.stage_execution_id=NEW.stage_execution_id
          AND worker.stage_run_unit_id=NEW.stage_run_unit_id
          AND worker.organization_id=NEW.organization_id
          AND worker.attempt_epoch=NEW.worker_attempt_epoch
          AND worker.lease_token=NEW.lease_token
          AND worker.active_tool_call_id=NEW.source_tool_call_id
          AND worker.status IN ('running','waiting_background')
          AND worker.lease_expires_at>statement_timestamp()
          AND call.status IN ('received','running')
        FOR SHARE OF worker,call
    ) THEN
        RAISE EXCEPTION 'enumeration_worker_root_live_tool_fence_required'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        (SELECT ordinal,input_key,target_id,exact_asset,technique,expected_capability,member_hash
           FROM coverage_denominator_items
          WHERE denominator_id=NEW.source_root_denominator_id
         EXCEPT
         SELECT ordinal,input_key,target_id,exact_asset,technique,expected_capability,member_hash
           FROM coverage_denominator_items
          WHERE denominator_id=NEW.worker_root_denominator_id)
        UNION ALL
        (SELECT ordinal,input_key,target_id,exact_asset,technique,expected_capability,member_hash
           FROM coverage_denominator_items
          WHERE denominator_id=NEW.worker_root_denominator_id
         EXCEPT
         SELECT ordinal,input_key,target_id,exact_asset,technique,expected_capability,member_hash
           FROM coverage_denominator_items
          WHERE denominator_id=NEW.source_root_denominator_id)
    ) THEN
        RAISE EXCEPTION 'enumeration_worker_root_member_census_mismatch' USING ERRCODE='23514';
    END IF;

    expected_worker_denominator_hash := tool_truth_sha256(jsonb_build_object(
        'execution_authority_hash',worker_authority.authority_hash,
        'input_manifest_hash',worker_root.input_manifest_hash,
        'contract',worker_root.contract,
        'denominator_kind','root'
    )::TEXT);
    IF worker_root.denominator_hash<>expected_worker_denominator_hash THEN
        RAISE EXCEPTION 'enumeration_worker_root_denominator_hash_mismatch' USING ERRCODE='23514';
    END IF;

    NEW.source_member_count := source_root.member_count;
    NEW.source_member_set_hash := source_root.member_set_hash;
    NEW.root_seal_hash := tool_truth_sha256(jsonb_build_object(
        'domain','enumeration_worker_authority_root.v1',
        'stable_root_request_id',NEW.stable_root_request_id,
        'source_root_denominator_id',NEW.source_root_denominator_id,
        'source_execution_authority_id',NEW.source_execution_authority_id,
        'source_denominator_hash',NEW.source_denominator_hash,
        'worker_root_denominator_id',NEW.worker_root_denominator_id,
        'worker_execution_authority_id',NEW.worker_execution_authority_id,
        'worker_denominator_hash',NEW.worker_denominator_hash,
        'stage_run_unit_id',NEW.stage_run_unit_id,
        'worker_run_id',NEW.worker_run_id,
        'worker_attempt_epoch',NEW.worker_attempt_epoch,
        'lease_token',NEW.lease_token,
        'source_tool_call_id',NEW.source_tool_call_id,
        'source_member_count',source_root.member_count,
        'source_member_set_hash',source_root.member_set_hash
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_worker_authority_roots_validate
BEFORE INSERT ON enumeration_worker_authority_roots
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_worker_authority_root();
CREATE TRIGGER enumeration_worker_authority_roots_immutable
BEFORE UPDATE OR DELETE ON enumeration_worker_authority_roots
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_worker_authority_root_immutable'
);

CREATE FUNCTION enumeration_denominator_has_worker_root(
    p_denominator_id UUID, p_execution_authority_id UUID
) RETURNS BOOLEAN LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE lineage AS (
        SELECT d.id,d.parent_denominator_id,d.execution_authority_id
          FROM coverage_denominators d
         WHERE d.id=p_denominator_id
           AND d.execution_authority_id=p_execution_authority_id
           AND d.sealed_at IS NOT NULL
        UNION ALL
        SELECT parent.id,parent.parent_denominator_id,parent.execution_authority_id
          FROM coverage_denominators parent
          JOIN lineage child ON child.parent_denominator_id=parent.id
         WHERE parent.execution_authority_id=p_execution_authority_id
           AND parent.sealed_at IS NOT NULL
    )
    SELECT EXISTS (
        SELECT 1 FROM lineage
        JOIN enumeration_worker_authority_roots root
          ON root.worker_root_denominator_id=lineage.id
         AND root.worker_execution_authority_id=lineage.execution_authority_id
    )
$$;

-- Frozen exact-origin membership is the only scope authority accepted by V2
-- producers. Current targets/observations may change after the StageTeamUnit
-- root is sealed and therefore cannot authorize a source or cross-origin B.
CREATE FUNCTION enumeration_worker_root_has_exact_origin(
    p_execution_authority_id UUID,
    p_target_id UUID,
    p_web_origin_id UUID
) RETURNS BOOLEAN LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM enumeration_worker_authority_roots root
          JOIN web_origins origin
            ON origin.id=p_web_origin_id
           AND origin.organization_id=root.organization_id
           AND origin.project_path=root.project_path_at_freeze
          JOIN targets target
            ON target.id=p_target_id
           AND target.organization_id=root.organization_id
           AND target.project_path=root.project_path_at_freeze
          JOIN coverage_denominator_items item
            ON item.denominator_id=root.worker_root_denominator_id
           AND item.execution_authority_id=root.worker_execution_authority_id
           AND item.target_id=target.id
           AND item.exact_asset=origin.origin
         WHERE root.worker_execution_authority_id=p_execution_authority_id
         GROUP BY root.id,item.target_id,item.exact_asset
        HAVING COUNT(DISTINCT item.technique)=4
           AND BOOL_AND(item.technique IN (
               'GOLISH-ENUM-DIR','GOLISH-ENUM-JS',
               'GOLISH-ENUM-JSAPI','GOLISH-ENUM-PARAM'
           ))
    )
$$;

-- A receipt-input census is the Enumeration terminal boundary. It proves the
-- exact expected-capability item set is sealed and every member carries at
-- least one normalized Tool Truth evidence/business authority. Domain writers
-- may not treat a merely-created generic receipt as terminal.
CREATE TABLE enumeration_receipt_input_census_seals (
    id UUID PRIMARY KEY,
    stable_seal_request_id UUID NOT NULL UNIQUE,
    receipt_id UUID NOT NULL UNIQUE,
    denominator_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    input_count BIGINT NOT NULL CHECK (input_count>=0),
    input_set_hash TEXT NOT NULL CHECK (input_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(receipt_id,execution_authority_id),
    FOREIGN KEY(receipt_id,denominator_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,denominator_id,execution_authority_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(denominator_id,execution_authority_id)
        REFERENCES coverage_denominators(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_receipt_input_census_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_count BIGINT;
DECLARE actual_count BIGINT;
DECLARE actual_hash TEXT;
DECLARE denominator_member_count BIGINT;
DECLARE denominator_sealed_empty BOOLEAN;
BEGIN
    IF NOT enumeration_denominator_has_worker_root(
        NEW.denominator_id,NEW.execution_authority_id
    ) THEN
        RAISE EXCEPTION 'enumeration_receipt_worker_root_required' USING ERRCODE='23514';
    END IF;
    SELECT denominator.member_count,denominator.sealed_empty
      INTO denominator_member_count,denominator_sealed_empty
      FROM capability_execution_receipts receipt
      JOIN coverage_denominators denominator
        ON denominator.id=receipt.denominator_id
       AND denominator.execution_authority_id=receipt.execution_authority_id
     WHERE receipt.id=NEW.receipt_id
       AND receipt.denominator_id=NEW.denominator_id
       AND receipt.execution_authority_id=NEW.execution_authority_id
       AND denominator.sealed_at IS NOT NULL
     FOR SHARE OF receipt,denominator;
    SELECT COUNT(*)::BIGINT INTO expected_count
      FROM capability_execution_receipts receipt
      JOIN coverage_denominator_items item
        ON item.denominator_id=receipt.denominator_id
       AND item.expected_capability=receipt.capability
     WHERE receipt.id=NEW.receipt_id
       AND receipt.denominator_id=NEW.denominator_id
       AND receipt.execution_authority_id=NEW.execution_authority_id;
    SELECT COUNT(*)::BIGINT,
           tool_truth_sha256(COALESCE(jsonb_agg(jsonb_build_object(
               'input_id',input.id,
               'denominator_item_id',input.denominator_item_id,
               'input_key',input.input_key,
               'attempt_state',input.attempt_state,
               'landing_state',input.landing_state,
               'observation_state',input.observation_state,
               'coverage_extent',input.coverage_extent,
               'coverage_gap_reason',input.coverage_gap_reason,
               'member_set_hash',input.member_set_hash
           ) ORDER BY input.denominator_item_id),'[]'::JSONB)::TEXT)
      INTO actual_count,actual_hash
      FROM capability_execution_receipt_inputs input
     WHERE input.receipt_id=NEW.receipt_id
       AND input.denominator_id=NEW.denominator_id
       AND input.execution_authority_id=NEW.execution_authority_id
       AND input.sealed_at IS NOT NULL
       AND input.attempt_state IN ('succeeded','failed','outcome_unknown','exhausted','superseded')
       AND COALESCE(input.member_count,0)>0;
    IF denominator_member_count IS NULL
       OR expected_count<>denominator_member_count
       OR denominator_sealed_empty IS DISTINCT FROM (expected_count=0)
       OR actual_count<>expected_count OR EXISTS (
        SELECT 1 FROM coverage_denominator_items item
        JOIN capability_execution_receipts receipt ON receipt.id=NEW.receipt_id
        LEFT JOIN capability_execution_receipt_inputs input
          ON input.receipt_id=receipt.id AND input.denominator_item_id=item.id
         AND input.execution_authority_id=NEW.execution_authority_id
         AND input.sealed_at IS NOT NULL
       WHERE item.denominator_id=NEW.denominator_id
         AND item.expected_capability=receipt.capability
         AND input.id IS NULL
    ) THEN
        RAISE EXCEPTION 'enumeration_receipt_input_exact_census_incomplete'
            USING ERRCODE='23514';
    END IF;
    NEW.input_count := actual_count;
    NEW.input_set_hash := actual_hash;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_receipt_input_census_seals_validate
BEFORE INSERT ON enumeration_receipt_input_census_seals
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_receipt_input_census_seal();
CREATE TRIGGER enumeration_receipt_input_census_seals_immutable
BEFORE UPDATE OR DELETE ON enumeration_receipt_input_census_seals
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_receipt_input_census_seal_immutable'
);

-- Legacy readers keep `body_or_form`; V2 may retain a more precise location.
ALTER TABLE enumeration_endpoint_parameters
    DROP CONSTRAINT enumeration_endpoint_parameter_location_check,
    ADD CONSTRAINT enumeration_endpoint_parameter_location_check CHECK (
        location IN (
            'query','body_or_form','body','form','path','header','graphql_variable','unknown'
        )
    );

-- Value-free JSON is a closed metadata language. Arbitrary object keys are
-- rejected instead of relying on a small deny-list that can be bypassed by
-- placing a credential under an innocuous key such as `note`.
CREATE FUNCTION enumeration_json_metadata_is_value_free(value JSONB)
RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE entry RECORD;
DECLARE scalar TEXT;
BEGIN
    IF value IS NULL THEN
        RETURN TRUE;
    END IF;
    IF jsonb_typeof(value)='object' THEN
        FOR entry IN SELECT key,nested FROM jsonb_each(value) AS fields(key,nested) LOOP
            IF entry.key<>ALL(ARRAY[
                'kind','name','type','value_type','location','requirement','required',
                'confidence','source_anchor','source_anchor_id','source_anchor_ids',
                'applies_to','base_kind','base_url','url','method','protocol','status',
                'reason_code','reason','start_byte','end_byte','start_line','start_column',
                'end_line','end_column','artifact_id','artifact_sha256','ordinal',
                'shape_hash','schema_hash','length','body_length','header_count','field_count',
                'redacted','present','fields','field_names','properties','items','source_urls',
                'discovered_from','document_bases','duplicate_of','chunk_name',
                'source_map_status','capture_kind','compatibility_version','policy_version',
                'schema_version','step','selected','selected_url','receiver','binding_id',
                'base_path','resolved_path','disposition','candidate_id','source_file','callee',
                'client_kind','content_type','query','body','form','header','graphql_variables',
                'path','request_id_hash','initiator_status','has_body','facts','document_base',
                'html_base','app_base','router_base','client_base','bundler_base','candidates'
            ]::TEXT[]) THEN
                RETURN FALSE;
            END IF;
            IF NOT enumeration_json_metadata_is_value_free(entry.nested) THEN
                RETURN FALSE;
            END IF;
        END LOOP;
        RETURN TRUE;
    ELSIF jsonb_typeof(value)='array' THEN
        FOR entry IN SELECT nested FROM jsonb_array_elements(value) AS elements(nested) LOOP
            IF NOT enumeration_json_metadata_is_value_free(entry.nested) THEN
                RETURN FALSE;
            END IF;
        END LOOP;
        RETURN TRUE;
    ELSIF jsonb_typeof(value)='string' THEN
        scalar := value #>> '{}';
        RETURN OCTET_LENGTH(scalar)<=4096
           AND scalar !~ '[[:cntrl:]]'
           AND scalar !~* '(authorization[[:space:]]*[:=]|cookie[[:space:]]*[:=]|password[[:space:]]*=|secret[[:space:]]*=|token[[:space:]]*=|api[_-]?key[[:space:]]*=|bearer[[:space:]]+[A-Za-z0-9._~+/-]+)'
           AND scalar !~ '://[^/?#]*@'
           AND (scalar !~ '://' OR scalar !~ '[?#]');
    END IF;
    RETURN TRUE;
END;
$$;

CREATE FUNCTION enumeration_json_object_has_only_keys(value JSONB, allowed_keys TEXT[])
RETURNS BOOLEAN LANGUAGE sql IMMUTABLE AS $$
    SELECT jsonb_typeof(value)='object'
       AND NOT EXISTS(
           SELECT 1 FROM jsonb_object_keys(value) AS key
            WHERE key<>ALL(allowed_keys)
       )
       AND enumeration_json_metadata_is_value_free(value)
$$;

CREATE FUNCTION enumeration_route_template_matches(route_template TEXT, concrete_url TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE template_segments TEXT[];
DECLARE concrete_segments TEXT[];
DECLARE template_segment TEXT;
DECLARE concrete_segment TEXT;
DECLARE index_value INTEGER;
BEGIN
    IF route_template IS NULL OR concrete_url IS NULL
       OR BTRIM(route_template)='' OR route_template ~ '[?#]'
       OR concrete_url ~ '[?#]' THEN
        RETURN FALSE;
    END IF;
    template_segments := string_to_array(BTRIM(route_template,'/'),'/');
    concrete_segments := string_to_array(BTRIM(concrete_url,'/'),'/');
    IF cardinality(template_segments)<>cardinality(concrete_segments) THEN
        RETURN FALSE;
    END IF;
    IF cardinality(template_segments)=0 THEN
        RETURN TRUE;
    END IF;
    FOR index_value IN 1..cardinality(template_segments) LOOP
        template_segment := template_segments[index_value];
        concrete_segment := concrete_segments[index_value];
        IF template_segment ~ '^\{[A-Za-z_][A-Za-z0-9_]*\}$' THEN
            IF concrete_segment IS NULL OR concrete_segment='' THEN
                RETURN FALSE;
            END IF;
        ELSIF template_segment IS DISTINCT FROM concrete_segment THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
END;
$$;

CREATE FUNCTION enumeration_display_url_is_value_free(value TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE query_part TEXT;
DECLARE query_name TEXT;
BEGIN
    IF value IS NULL THEN
        RETURN TRUE;
    END IF;
    IF BTRIM(value)='' OR value ~ '#' OR value ~ '://[^/?#]*@' THEN
        RETURN FALSE;
    END IF;
    IF value !~ '\?' THEN
        RETURN TRUE;
    END IF;
    query_part := split_part(value,'?',2);
    IF query_part='' OR query_part ~ '=' THEN
        RETURN FALSE;
    END IF;
    FOREACH query_name IN ARRAY string_to_array(query_part,'&') LOOP
        IF query_name !~ '^[A-Za-z0-9_.~-]+$' THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
END;
$$;

CREATE FUNCTION enumeration_url_matches_web_origin(value TEXT, origin_id UUID)
RETURNS BOOLEAN LANGUAGE plpgsql VOLATILE AS $$
DECLARE origin_scheme TEXT;
DECLARE origin_host TEXT;
DECLARE origin_port INTEGER;
DECLARE explicit_origin TEXT;
DECLARE implicit_origin TEXT;
DECLARE normalized_value TEXT;
BEGIN
    IF value IS NULL OR value ~ '[?#]' OR value ~ '://[^/?#]*@' THEN
        RETURN FALSE;
    END IF;
    SELECT LOWER(scheme),LOWER(host),port
      INTO origin_scheme,origin_host,origin_port
      FROM web_origins WHERE id=origin_id FOR SHARE;
    IF origin_scheme NOT IN ('http','https') OR origin_host IS NULL THEN
        RETURN FALSE;
    END IF;
    IF origin_host ~ ':' AND origin_host !~ '^\[.*\]$' THEN
        origin_host := '['||origin_host||']';
    END IF;
    explicit_origin := origin_scheme||'://'||origin_host||':'||origin_port::TEXT;
    implicit_origin := CASE
        WHEN (origin_scheme='http' AND origin_port=80)
          OR (origin_scheme='https' AND origin_port=443)
        THEN origin_scheme||'://'||origin_host
        ELSE NULL
    END;
    normalized_value := LOWER(value);
    IF normalized_value LIKE 'ws://%' THEN
        normalized_value := 'http://' || SUBSTRING(normalized_value FROM 6);
    ELSIF normalized_value LIKE 'wss://%' THEN
        normalized_value := 'https://' || SUBSTRING(normalized_value FROM 7);
    END IF;
    RETURN normalized_value=explicit_origin
        OR normalized_value LIKE explicit_origin||'/%'
        OR (implicit_origin IS NOT NULL AND (
            normalized_value=implicit_origin
            OR normalized_value LIKE implicit_origin||'/%'
        ));
END;
$$;

-- ---------------------------------------------------------------------------
-- Denominator-bound descriptors and immutable occurrence truth.
-- ---------------------------------------------------------------------------

CREATE TABLE enumeration_js_analysis_items (
    id UUID PRIMARY KEY,
    stable_descriptor_request_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    terminal_receipt_id UUID,
    terminal_receipt_input_id UUID,
    manifest_url TEXT NOT NULL CHECK (
        BTRIM(manifest_url)<>'' AND manifest_url !~ '[?#]' AND manifest_url !~ '://[^/]*@'
    ),
    page_url TEXT NOT NULL CHECK (
        BTRIM(page_url)<>'' AND page_url !~ '[?#]' AND page_url !~ '://[^/]*@'
    ),
    document_url TEXT CHECK (
        document_url IS NULL OR (document_url !~ '[?#]' AND document_url !~ '://[^/]*@')
    ),
    chunk_ordinal INTEGER NOT NULL CHECK (chunk_ordinal>=0),
    source_map_url TEXT CHECK (
        source_map_url IS NULL OR (source_map_url !~ '[?#]' AND source_map_url !~ '://[^/]*@')
    ),
    script_sha256 TEXT CHECK (script_sha256 IS NULL OR script_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    descriptor_metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (
        enumeration_json_object_has_only_keys(descriptor_metadata,ARRAY[
            'source_urls','discovered_from','document_bases','duplicate_of','chunk_name',
            'source_map_status','capture_kind','compatibility_version'
        ]::TEXT[])
    ),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version BETWEEN 0 AND 1),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    terminal_bound_at TIMESTAMPTZ,
    UNIQUE(execution_authority_id,stable_descriptor_request_id),
    UNIQUE(id,execution_authority_id),
    CHECK ((terminal_receipt_id IS NULL)=(terminal_receipt_input_id IS NULL)),
    CHECK ((terminal_receipt_input_id IS NULL AND terminal_bound_at IS NULL AND row_version=0)
        OR (terminal_receipt_input_id IS NOT NULL AND terminal_bound_at IS NOT NULL AND row_version=1)),
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id) ON DELETE RESTRICT,
    FOREIGN KEY(denominator_item_id,denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,denominator_item_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
            id,receipt_id,denominator_item_id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_guard_js_analysis_item()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'enumeration_js_analysis_item_immutable' USING ERRCODE='23514';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.terminal_receipt_input_id IS NOT NULL OR NOT EXISTS (
            SELECT 1 FROM coverage_denominators d
             WHERE d.id=NEW.denominator_id AND d.execution_authority_id=NEW.execution_authority_id
               AND d.sealed_at IS NOT NULL
               AND enumeration_denominator_has_worker_root(d.id,d.execution_authority_id)
             FOR SHARE
        ) THEN
            RAISE EXCEPTION 'enumeration_js_analysis_descriptor_unsealed' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.terminal_receipt_input_id IS NOT NULL
       OR NEW.terminal_receipt_input_id IS NULL OR NEW.row_version<>1
       OR (to_jsonb(NEW)-ARRAY['terminal_receipt_id','terminal_receipt_input_id','terminal_bound_at','row_version'])
          IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['terminal_receipt_id','terminal_receipt_input_id','terminal_bound_at','row_version'])
       OR NOT EXISTS (
            SELECT 1 FROM capability_execution_receipt_inputs i
             WHERE i.id=NEW.terminal_receipt_input_id AND i.receipt_id=NEW.terminal_receipt_id
               AND i.denominator_item_id=NEW.denominator_item_id
               AND i.execution_authority_id=NEW.execution_authority_id
               AND i.sealed_at IS NOT NULL
               AND i.attempt_state IN ('succeeded','failed','outcome_unknown','exhausted','superseded')
               AND EXISTS (
                   SELECT 1 FROM enumeration_receipt_input_census_seals census
                    WHERE census.receipt_id=NEW.terminal_receipt_id
                      AND census.denominator_id=NEW.denominator_id
                      AND census.execution_authority_id=NEW.execution_authority_id
               )
            FOR SHARE
       ) THEN
        RAISE EXCEPTION 'enumeration_js_analysis_terminal_cas_required' USING ERRCODE='23514';
    END IF;
    NEW.terminal_bound_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_js_analysis_items_guard
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_js_analysis_items
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_js_analysis_item();

CREATE TABLE enumeration_endpoint_candidate_inputs (
    id UUID PRIMARY KEY,
    stable_candidate_request_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_input_id UUID NOT NULL,
    js_analysis_item_id UUID,
    logical_input_key TEXT NOT NULL CHECK (BTRIM(logical_input_key)<>''),
    source_anchor TEXT NOT NULL CHECK (
        BTRIM(source_anchor)<>''
        AND source_anchor !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    callsite_fingerprint TEXT NOT NULL CHECK (callsite_fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    event_fingerprint TEXT NOT NULL CHECK (event_fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    duplicate_ordinal INTEGER NOT NULL CHECK (duplicate_ordinal>=0),
    resolution_input TEXT NOT NULL CHECK (
        BTRIM(resolution_input)<>'' AND resolution_input !~ '[?#]'
        AND resolution_input !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(execution_authority_id,stable_candidate_request_id),
    UNIQUE(denominator_id,logical_input_key,duplicate_ordinal),
    UNIQUE(denominator_item_id),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,terminal_receipt_input_id),
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id) ON DELETE RESTRICT,
    FOREIGN KEY(denominator_item_id,denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,denominator_item_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
            id,receipt_id,denominator_item_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(js_analysis_item_id,execution_authority_id)
        REFERENCES enumeration_js_analysis_items(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_candidate_input()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM coverage_denominators d
        JOIN capability_execution_receipt_inputs i
          ON i.id=NEW.terminal_receipt_input_id AND i.receipt_id=NEW.terminal_receipt_id
         AND i.denominator_item_id=NEW.denominator_item_id
         AND i.execution_authority_id=NEW.execution_authority_id
        WHERE d.id=NEW.denominator_id AND d.execution_authority_id=NEW.execution_authority_id
          AND d.denominator_kind='derived_child'
          AND d.sealed_at IS NOT NULL AND i.sealed_at IS NOT NULL
          AND enumeration_denominator_has_worker_root(d.id,d.execution_authority_id)
          AND EXISTS (
              SELECT 1 FROM enumeration_receipt_input_census_seals census
               WHERE census.receipt_id=NEW.terminal_receipt_id
                 AND census.denominator_id=NEW.denominator_id
                 AND census.execution_authority_id=NEW.execution_authority_id
          )
          AND i.input_key=NEW.logical_input_key
          AND i.attempt_state='succeeded' AND i.landing_state='committed'
          AND i.observation_state='found' AND i.coverage_extent='complete'
          AND i.coverage_gap_reason='none'
        FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_candidate_terminal_receipt_required' USING ERRCODE='23514';
    END IF;
    IF NEW.js_analysis_item_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM enumeration_js_analysis_items js_item
        JOIN coverage_denominators candidate_denominator
          ON candidate_denominator.id=NEW.denominator_id
         AND candidate_denominator.execution_authority_id=NEW.execution_authority_id
        WHERE js_item.id=NEW.js_analysis_item_id
          AND js_item.execution_authority_id=NEW.execution_authority_id
          AND js_item.terminal_receipt_input_id IS NOT NULL
          AND candidate_denominator.parent_denominator_id=js_item.denominator_id
          AND candidate_denominator.parent_denominator_item_id=js_item.denominator_item_id
        FOR SHARE OF js_item,candidate_denominator
    ) THEN
        RAISE EXCEPTION 'enumeration_candidate_denominator_parent_mismatch'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_candidate_inputs_validate
BEFORE INSERT ON enumeration_endpoint_candidate_inputs
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_candidate_input();
CREATE TRIGGER enumeration_endpoint_candidate_inputs_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_candidate_inputs
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_candidate_input_immutable');
CREATE INDEX enumeration_endpoint_candidate_inputs_authority_idx
    ON enumeration_endpoint_candidate_inputs(execution_authority_id,logical_input_key);

CREATE TABLE enumeration_endpoint_candidate_capture_events (
    capture_event_id UUID PRIMARY KEY,
    candidate_input_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    capture_attempt_ordinal INTEGER NOT NULL CHECK (capture_attempt_ordinal>0),
    event_fingerprint TEXT NOT NULL CHECK (event_fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    captured_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(capture_event_id,candidate_input_id,execution_authority_id),
    UNIQUE(candidate_input_id,capture_attempt_ordinal),
    FOREIGN KEY(candidate_input_id,execution_authority_id)
        REFERENCES enumeration_endpoint_candidate_inputs(id,execution_authority_id)
        ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_candidate_capture_events_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_candidate_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_endpoint_candidate_capture_event_immutable'
);

CREATE FUNCTION enumeration_validate_candidate_capture_event()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
         WHERE candidate.id=NEW.candidate_input_id
           AND candidate.execution_authority_id=NEW.execution_authority_id
           AND candidate.event_fingerprint=NEW.event_fingerprint
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_candidate_capture_event_identity_mismatch'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_candidate_capture_events_validate
BEFORE INSERT ON enumeration_endpoint_candidate_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_candidate_capture_event();

CREATE TABLE enumeration_endpoint_occurrences (
    id UUID PRIMARY KEY,
    stable_occurrence_request_id UUID NOT NULL,
    candidate_input_id UUID NOT NULL,
    initial_capture_event_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_input_id UUID NOT NULL,
    source_target_id UUID NOT NULL,
    source_web_origin_id UUID NOT NULL,
    resolved_target_id UUID,
    resolved_web_origin_id UUID,
    parent_occurrence_id UUID,
    source_url TEXT NOT NULL CHECK (
        BTRIM(source_url)<>'' AND source_url !~ '[?#]' AND source_url !~ '://[^/]*@'
    ),
    document_url TEXT CHECK (
        document_url IS NULL OR (document_url !~ '[?#]' AND document_url !~ '://[^/]*@')
    ),
    script_url TEXT CHECK (
        script_url IS NULL OR (script_url !~ '[?#]' AND script_url !~ '://[^/]*@')
    ),
    script_sha256 TEXT CHECK (script_sha256 IS NULL OR script_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_span JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (
        enumeration_json_object_has_only_keys(source_span,ARRAY[
            'start_byte','end_byte','start_line','start_column','end_line','end_column',
            'artifact_id','artifact_sha256','status','reason_code'
        ]::TEXT[])
    ),
    initiator_url TEXT CHECK (
        initiator_url IS NULL OR (initiator_url !~ '[?#]' AND initiator_url !~ '://[^/]*@')
    ),
    initiator_status TEXT NOT NULL CHECK (
        initiator_status IN ('matched','unsupported_cdp','unmatched','not_applicable')
    ),
    initiator_line INTEGER CHECK (initiator_line IS NULL OR initiator_line>=0),
    initiator_column INTEGER CHECK (initiator_column IS NULL OR initiator_column>=0),
    cdp_request_id_hash TEXT CHECK (
        cdp_request_id_hash IS NULL OR cdp_request_id_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    protocol TEXT NOT NULL CHECK (protocol IN ('http','https','websocket','graphql','unknown')),
    method TEXT NOT NULL CHECK (BTRIM(method)<>''),
    graphql_operation_name TEXT,
    websocket_subprotocol TEXT CHECK (
        websocket_subprotocol IS NULL OR websocket_subprotocol !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    raw_expression TEXT CHECK (
        raw_expression IS NULL OR (
            BTRIM(raw_expression)<>''
            AND raw_expression !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
        )
    ),
    receiver_kind TEXT CHECK (
        receiver_kind IS NULL OR receiver_kind !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    observation_kind TEXT NOT NULL CHECK (
        observation_kind IN ('runtime_request','html_form','static_ast','ai_analysis')
    ),
    inference_level TEXT NOT NULL CHECK (inference_level IN ('observed','deterministic','ai_inferred')),
    resolution_status TEXT NOT NULL CHECK (resolution_status IN ('resolved','ambiguous','unresolved','not_applicable')),
    scope_decision TEXT NOT NULL CHECK (scope_decision IN ('in_scope','scope_excluded')),
    candidate_classification TEXT NOT NULL CHECK (candidate_classification IN ('endpoint','noise')),
    canonical_request_url TEXT CHECK (
        canonical_request_url IS NULL
        OR (canonical_request_url !~ '[?#]' AND canonical_request_url !~ '://[^/]*@')
    ),
    display_url TEXT CHECK (enumeration_display_url_is_value_free(display_url)),
    resolution_reason TEXT NOT NULL CHECK (
        BTRIM(resolution_reason)<>''
        AND resolution_reason !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    resolution_base_facts JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (
        enumeration_json_object_has_only_keys(resolution_base_facts,ARRAY[
            'facts','document_base','html_base','app_base','router_base','client_base',
            'bundler_base','candidates','selected_url'
        ]::TEXT[])
    ),
    resolution_candidates JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (
        jsonb_typeof(resolution_candidates)='array'
        AND enumeration_json_metadata_is_value_free(resolution_candidates)
    ),
    resolution_chain JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (
        jsonb_typeof(resolution_chain)='array'
        AND enumeration_json_metadata_is_value_free(resolution_chain)
    ),
    route_kind TEXT NOT NULL CHECK (route_kind IN ('exact','template','dynamic_unresolved')),
    route_template TEXT CHECK (route_template IS NULL OR (
        BTRIM(route_template)<>'' AND route_template !~ '[?#]'
    )),
    request_sent BOOLEAN NOT NULL,
    request_schema JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (
        enumeration_json_object_has_only_keys(request_schema,ARRAY[
            'query','body','form','header','path','graphql_variables','content_type',
            'schema_version','fields'
        ]::TEXT[])
    ),
    redaction_metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (
        enumeration_json_object_has_only_keys(redaction_metadata,ARRAY[
            'redacted','body_length','header_count','field_count','schema_hash','policy_version'
        ]::TEXT[])
    ),
    request_schema_hash TEXT NOT NULL CHECK (request_schema_hash ~ '^sha256:[0-9a-f]{64}$'),
    request_body_length BIGINT CHECK (request_body_length IS NULL OR request_body_length>=0),
    runtime_sample_url TEXT CHECK (
        runtime_sample_url IS NULL OR (runtime_sample_url !~ '[?#]' AND runtime_sample_url !~ '://[^/]*@')
    ),
    promotion_eligible BOOLEAN GENERATED ALWAYS AS (
        resolution_status='resolved'
        AND scope_decision='in_scope'
        AND candidate_classification='endpoint'
        AND inference_level IN ('observed','deterministic')
        AND route_kind IN ('exact','template')
    ) STORED,
    observed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(execution_authority_id,stable_occurrence_request_id),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,operation_id,organization_id),
    UNIQUE(id,operation_id,scope_snapshot_id,organization_id),
    UNIQUE(id,operation_id,project_scope_id,scope_snapshot_id,organization_id),
    UNIQUE(id,operation_id,project_scope_id,scope_snapshot_id,organization_id,execution_authority_id),
    UNIQUE(id,candidate_input_id,execution_authority_id),
    CHECK ((resolved_target_id IS NULL)=(resolved_web_origin_id IS NULL)),
    CHECK ((resolution_status='resolved' AND scope_decision='in_scope')
        =(resolved_target_id IS NOT NULL)),
    CHECK (scope_decision<>'scope_excluded' OR resolved_target_id IS NULL),
    CHECK (route_kind<>'template' OR (
        route_template IS NOT NULL AND BTRIM(route_template)<>''
    )),
    CHECK (route_kind<>'dynamic_unresolved' OR route_template IS NULL),
    CHECK (route_kind<>'exact' OR canonical_request_url IS NOT NULL),
    CHECK (runtime_sample_url IS NULL OR observation_kind='runtime_request'),
    CHECK (
        (initiator_status='matched' AND initiator_url IS NOT NULL
            AND initiator_line IS NOT NULL AND initiator_column IS NOT NULL
            AND cdp_request_id_hash IS NOT NULL)
        OR (initiator_status<>'matched' AND initiator_url IS NULL
            AND initiator_line IS NULL AND initiator_column IS NULL
            AND cdp_request_id_hash IS NULL)
    ),
    FOREIGN KEY(candidate_input_id,execution_authority_id,terminal_receipt_input_id)
        REFERENCES enumeration_endpoint_candidate_inputs(
            id,execution_authority_id,terminal_receipt_input_id) ON DELETE RESTRICT,
    FOREIGN KEY(initial_capture_event_id,candidate_input_id,execution_authority_id)
        REFERENCES enumeration_endpoint_candidate_capture_events(
            capture_event_id,candidate_input_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(id,receipt_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(source_target_id,organization_id,project_path_at_freeze)
        REFERENCES targets(id,organization_id,project_path) ON DELETE RESTRICT,
    FOREIGN KEY(source_web_origin_id,organization_id,project_path_at_freeze)
        REFERENCES web_origins(id,organization_id,project_path) ON DELETE RESTRICT,
    FOREIGN KEY(resolved_target_id,organization_id,project_path_at_freeze)
        REFERENCES targets(id,organization_id,project_path) ON DELETE RESTRICT,
    FOREIGN KEY(resolved_web_origin_id,organization_id,project_path_at_freeze)
        REFERENCES web_origins(id,organization_id,project_path) ON DELETE RESTRICT,
    FOREIGN KEY(parent_occurrence_id,operation_id,project_scope_id,scope_snapshot_id,
                organization_id,execution_authority_id)
        REFERENCES enumeration_endpoint_occurrences(
            id,operation_id,project_scope_id,scope_snapshot_id,organization_id,
            execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_occurrence()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE contract TEXT;
DECLARE tool_contract TEXT;
DECLARE owner_kind TEXT;
BEGIN
    SELECT o.enumeration_analysis_contract,o.tool_truth_contract,a.execution_owner_kind
      INTO contract,tool_contract,owner_kind
      FROM operation_state o
      JOIN tool_truth_execution_authorities a
        ON a.id=NEW.execution_authority_id AND a.operation_id=o.operation_id
     WHERE o.operation_id=NEW.operation_id
       AND a.project_scope_id=NEW.project_scope_id
       AND a.project_path_at_freeze=NEW.project_path_at_freeze
       AND a.scope_snapshot_id=NEW.scope_snapshot_id
       AND a.organization_id=NEW.organization_id
       AND a.stage_execution_id=NEW.stage_execution_id
     FOR SHARE OF o,a;
    IF contract IS NULL THEN
        RAISE EXCEPTION 'enumeration_occurrence_authority_mismatch' USING ERRCODE='23514';
    END IF;
    IF contract='legacy_v1' THEN
        RAISE EXCEPTION 'enumeration_v2_writer_disabled' USING ERRCODE='23514';
    END IF;
    IF contract='agent_team_v2' AND tool_contract<>'receipt_v1' THEN
        RAISE EXCEPTION 'enumeration_receipt_v1_required' USING ERRCODE='23514';
    END IF;
    IF contract='agent_team_v2_shadow' AND tool_contract NOT IN ('shadow_v1','receipt_v1') THEN
        RAISE EXCEPTION 'enumeration_shadow_tool_truth_incompatible' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
         WHERE candidate.id=NEW.candidate_input_id
           AND candidate.execution_authority_id=NEW.execution_authority_id
           AND candidate.terminal_receipt_id=NEW.terminal_receipt_id
           AND candidate.terminal_receipt_input_id=NEW.terminal_receipt_input_id
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_candidate_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_capture_events capture
         WHERE capture.capture_event_id=NEW.initial_capture_event_id
           AND capture.candidate_input_id=NEW.candidate_input_id
           AND capture.execution_authority_id=NEW.execution_authority_id
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_capture_event_mismatch' USING ERRCODE='23514';
    END IF;
    IF owner_kind<>'worker_tool' OR NOT EXISTS (
        SELECT 1 FROM tool_truth_execution_authorities a
        JOIN stage_worker_runs w ON w.id=a.worker_run_id
        JOIN tool_calls t ON t.id=a.source_tool_call_id
        WHERE a.id=NEW.execution_authority_id
          AND w.attempt_epoch=a.worker_attempt_epoch AND w.lease_token=a.lease_token
          AND t.worker_run_id=w.id AND t.attempt_epoch=w.attempt_epoch AND t.lease_token=w.lease_token
          AND enumeration_denominator_has_worker_root(
              (SELECT denominator_id FROM enumeration_endpoint_candidate_inputs
                WHERE id=NEW.candidate_input_id),a.id
          )
        FOR SHARE OF w,t
    ) THEN
        RAISE EXCEPTION 'tool_truth_worker_fence_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT enumeration_worker_root_has_exact_origin(
        NEW.execution_authority_id,NEW.source_target_id,NEW.source_web_origin_id
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_source_origin_not_in_frozen_root'
            USING ERRCODE='23514';
    END IF;
    IF NEW.scope_decision='scope_excluded' AND NEW.resolved_web_origin_id IS NOT NULL THEN
        RAISE EXCEPTION 'enumeration_scope_excluded_resolved_target_forbidden' USING ERRCODE='23514';
    END IF;
    IF NEW.resolved_web_origin_id IS NOT NULL AND NOT enumeration_worker_root_has_exact_origin(
        NEW.execution_authority_id,NEW.resolved_target_id,NEW.resolved_web_origin_id
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_resolved_origin_not_in_frozen_root'
            USING ERRCODE='23514';
    END IF;
    IF NOT enumeration_url_matches_web_origin(NEW.source_url,NEW.source_web_origin_id) THEN
        RAISE EXCEPTION 'enumeration_occurrence_source_url_origin_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.resolved_web_origin_id IS NOT NULL AND (
        (NEW.canonical_request_url IS NOT NULL AND NOT enumeration_url_matches_web_origin(
            NEW.canonical_request_url,NEW.resolved_web_origin_id
        ))
        OR (NEW.runtime_sample_url IS NOT NULL AND NOT enumeration_url_matches_web_origin(
            NEW.runtime_sample_url,NEW.resolved_web_origin_id
        ))
        OR (NEW.route_template IS NOT NULL AND NEW.route_template ~ '^[A-Za-z][A-Za-z0-9+.-]*://'
            AND NOT enumeration_url_matches_web_origin(
                NEW.route_template,NEW.resolved_web_origin_id
            ))
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_resolved_url_origin_mismatch' USING ERRCODE='23514';
    END IF;
    -- `canonical_request_url` is the exact-origin identity, not a display URL.
    -- Default ports therefore remain explicit and must match the frozen EAS
    -- `web_origins.origin`; permissive implicit-port matching is reserved for
    -- source/runtime observations that preserve how a client spelled a URL.
    IF NEW.canonical_request_url IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM web_origins origin
         WHERE origin.id=NEW.resolved_web_origin_id
           AND (
               NEW.canonical_request_url=origin.origin
               OR NEW.canonical_request_url LIKE origin.origin||'/%'
           )
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_canonical_url_not_exact_origin'
            USING ERRCODE='23514';
    END IF;
    IF NEW.parent_occurrence_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrences p
         WHERE p.id=NEW.parent_occurrence_id AND p.operation_id=NEW.operation_id
           AND p.project_scope_id=NEW.project_scope_id AND p.scope_snapshot_id=NEW.scope_snapshot_id
           AND p.organization_id=NEW.organization_id
           AND p.execution_authority_id=NEW.execution_authority_id FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_parent_authority_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.method<>UPPER(NEW.method) THEN
        RAISE EXCEPTION 'enumeration_occurrence_method_not_canonical' USING ERRCODE='23514';
    END IF;
    NEW.request_schema_hash := tool_truth_sha256(NEW.request_schema::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_occurrences_validate
BEFORE INSERT ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_occurrence();
CREATE TRIGGER enumeration_endpoint_occurrences_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_occurrence_immutable');
CREATE INDEX enumeration_endpoint_occurrences_operation_origin_idx
    ON enumeration_endpoint_occurrences(
        operation_id,organization_id,resolved_web_origin_id,protocol,method
    );

CREATE TABLE enumeration_endpoint_occurrence_capture_events (
    occurrence_id UUID NOT NULL,
    candidate_input_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    capture_event_id UUID NOT NULL,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(occurrence_id,capture_event_id),
    FOREIGN KEY(occurrence_id,candidate_input_id,execution_authority_id)
        REFERENCES enumeration_endpoint_occurrences(
            id,candidate_input_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(capture_event_id,candidate_input_id,execution_authority_id)
        REFERENCES enumeration_endpoint_candidate_capture_events(
            capture_event_id,candidate_input_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_occurrence_capture_events_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrence_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_endpoint_occurrence_capture_event_immutable'
);

-- Resolution Analyst output is advisory and append-only.  It cannot mutate a
-- parent occurrence or carry a canonical endpoint foreign key.
CREATE TABLE enumeration_js_resolution_suggestions (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    worker_run_id UUID NOT NULL,
    source_tool_call_id UUID NOT NULL,
    worker_attempt_epoch BIGINT NOT NULL CHECK (worker_attempt_epoch>=0),
    lease_token UUID NOT NULL,
    assigned_work_item_id UUID NOT NULL,
    assigned_cluster_id UUID NOT NULL,
    parent_occurrence_id UUID NOT NULL,
    candidate_input_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK (
        disposition IN ('ai_inferred','ambiguous','unresolved','ai_noise_suspected')
    ),
    artifact_id TEXT,
    artifact_sha256 TEXT CHECK (
        artifact_sha256 IS NULL OR artifact_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_start_byte BIGINT CHECK (source_start_byte IS NULL OR source_start_byte>=0),
    source_end_byte BIGINT CHECK (source_end_byte IS NULL OR source_end_byte>source_start_byte),
    capture_anchor_id TEXT,
    suggested_url TEXT CHECK (
        suggested_url IS NULL OR (
            suggested_url !~ '[?#]' AND suggested_url !~ '://[^/]*@'
        )
    ),
    method TEXT CHECK (
        method IS NULL OR method IN ('GET','POST','PUT','PATCH','DELETE','HEAD','OPTIONS')
    ),
    parameter_names JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (
        jsonb_typeof(parameter_names)='array'
        AND enumeration_json_metadata_is_value_free(parameter_names)
    ),
    reason_code TEXT NOT NULL CHECK (
        BTRIM(reason_code)<>''
        AND reason_code !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (artifact_id IS NOT NULL AND artifact_sha256 IS NOT NULL
            AND source_start_byte IS NOT NULL AND source_end_byte IS NOT NULL)
        OR capture_anchor_id IS NOT NULL
    ),
    UNIQUE(parent_occurrence_id,worker_run_id,source_tool_call_id),
    CHECK (assigned_cluster_id=parent_occurrence_id),
    FOREIGN KEY(parent_occurrence_id,operation_id,organization_id)
        REFERENCES enumeration_endpoint_occurrences(id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(candidate_input_id)
        REFERENCES enumeration_endpoint_candidate_inputs(id) ON DELETE RESTRICT,
    FOREIGN KEY(assigned_work_item_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        REFERENCES stage_work_items(id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(worker_run_id,assigned_work_item_id,operation_id,stage_execution_id,
                stage_run_unit_id,organization_id)
        REFERENCES stage_worker_runs(id,work_item_id,operation_id,stage_execution_id,
                                     stage_run_unit_id,organization_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_resolution_suggestion()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM stage_worker_runs worker
          JOIN stage_work_items item
            ON item.id=NEW.assigned_work_item_id
           AND item.operation_id=NEW.operation_id
           AND item.stage_execution_id=NEW.stage_execution_id
           AND item.stage_run_unit_id=NEW.stage_run_unit_id
           AND item.organization_id=NEW.organization_id
          JOIN stage_worker_requests request
            ON request.accepted_work_item_id=item.id
           AND request.operation_id=item.operation_id
           AND request.stage_execution_id=item.stage_execution_id
           AND request.stage_run_unit_id=item.stage_run_unit_id
           AND request.organization_id=item.organization_id
          JOIN tool_calls call ON call.id=NEW.source_tool_call_id
         WHERE worker.id=NEW.worker_run_id
           AND worker.operation_id=NEW.operation_id
           AND worker.stage_execution_id=NEW.stage_execution_id
           AND worker.stage_run_unit_id=NEW.stage_run_unit_id
           AND worker.organization_id=NEW.organization_id
           AND worker.attempt_epoch=NEW.worker_attempt_epoch
           AND worker.lease_token=NEW.lease_token
           AND worker.active_tool_call_id=call.id
           AND worker.status IN ('running','waiting_background')
           AND worker.lease_expires_at>statement_timestamp()
           AND worker.work_item_id=item.id
           AND worker.work_item_kind=item.kind
           AND worker.work_item_key=item.stable_key
           AND worker.specialist=item.role
           AND item.kind='enumeration_resolution'
           AND item.role='resolution_analyst'
           AND request.status='accepted'
           AND request.request_kind='enumeration_resolution'
           AND request.requested_role='resolution_analyst'
           AND ((request.reason_code::JSONB->>'objective')::JSONB
                    ->>'unresolved_cluster_id')=NEW.assigned_cluster_id::TEXT
           AND call.worker_run_id=worker.id
           AND call.attempt_epoch=worker.attempt_epoch
           AND call.lease_token=worker.lease_token
         FOR SHARE OF worker,item,request,call
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_worker_fence_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrences occurrence
         WHERE occurrence.id=NEW.parent_occurrence_id
           AND occurrence.candidate_input_id=NEW.candidate_input_id
           AND occurrence.operation_id=NEW.operation_id
           AND occurrence.organization_id=NEW.organization_id
           AND occurrence.resolution_status IN ('ambiguous','unresolved')
           AND occurrence.candidate_classification='endpoint'
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_parent_mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_js_resolution_suggestions_validate
BEFORE INSERT ON enumeration_js_resolution_suggestions
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_resolution_suggestion();
CREATE TRIGGER enumeration_js_resolution_suggestions_immutable
BEFORE UPDATE OR DELETE ON enumeration_js_resolution_suggestions
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_js_resolution_suggestion_immutable');
CREATE INDEX enumeration_endpoint_occurrences_parent_idx
    ON enumeration_endpoint_occurrences(parent_occurrence_id)
    WHERE parent_occurrence_id IS NOT NULL;

CREATE TABLE enumeration_endpoint_parameter_assessments (
    id UUID PRIMARY KEY,
    occurrence_id UUID NOT NULL,
    occurrence_execution_authority_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_input_id UUID NOT NULL,
    parameter_outcome TEXT NOT NULL CHECK (
        parameter_outcome IN ('found','checked_empty','unresolved','not_applicable')
    ),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(occurrence_id,denominator_item_id),
    UNIQUE(id,execution_authority_id),
    UNIQUE(id,execution_authority_id,occurrence_id,occurrence_execution_authority_id),
    UNIQUE(id,execution_authority_id,occurrence_id,occurrence_execution_authority_id,
           operation_id,project_scope_id,scope_snapshot_id,organization_id),
    FOREIGN KEY(occurrence_id,operation_id,project_scope_id,scope_snapshot_id,
                organization_id,occurrence_execution_authority_id)
        REFERENCES enumeration_endpoint_occurrences(
            id,operation_id,project_scope_id,scope_snapshot_id,organization_id,
            execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
            id,operation_id,project_scope_id,project_path_at_freeze,
            scope_snapshot_id,organization_id,stage_execution_id) ON DELETE RESTRICT,
    FOREIGN KEY(denominator_item_id,denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,denominator_item_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
            id,receipt_id,denominator_item_id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_parameter_assessments_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_parameter_assessments
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_parameter_assessment_immutable');

CREATE FUNCTION enumeration_validate_parameter_assessment()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE parameter_authority tool_truth_execution_authorities%ROWTYPE;
DECLARE discovery_authority tool_truth_execution_authorities%ROWTYPE;
BEGIN
    SELECT * INTO parameter_authority FROM tool_truth_execution_authorities
     WHERE id=NEW.execution_authority_id
       AND operation_id=NEW.operation_id
       AND project_scope_id=NEW.project_scope_id
       AND project_path_at_freeze=NEW.project_path_at_freeze
       AND scope_snapshot_id=NEW.scope_snapshot_id
       AND organization_id=NEW.organization_id
       AND stage_execution_id=NEW.stage_execution_id
     FOR SHARE;
    SELECT authority.* INTO discovery_authority
      FROM tool_truth_execution_authorities authority
      JOIN enumeration_endpoint_occurrences occurrence
        ON occurrence.id=NEW.occurrence_id
       AND occurrence.execution_authority_id=authority.id
     WHERE authority.id=NEW.occurrence_execution_authority_id
       AND authority.operation_id=NEW.operation_id
       AND authority.project_scope_id=NEW.project_scope_id
       AND authority.project_path_at_freeze=NEW.project_path_at_freeze
       AND authority.scope_snapshot_id=NEW.scope_snapshot_id
       AND authority.organization_id=NEW.organization_id
       AND authority.stage_execution_id=NEW.stage_execution_id
     FOR SHARE OF authority,occurrence;
    IF parameter_authority.id IS NULL OR discovery_authority.id IS NULL
       OR parameter_authority.execution_owner_kind<>'worker_tool'
       OR discovery_authority.execution_owner_kind<>'worker_tool'
       OR parameter_authority.id=discovery_authority.id
       OR parameter_authority.stage_run_unit_id IS NULL
       OR parameter_authority.stage_run_unit_id IS DISTINCT FROM discovery_authority.stage_run_unit_id
       OR NOT EXISTS (
            SELECT 1 FROM stage_worker_runs worker
            JOIN tool_calls call ON call.id=parameter_authority.source_tool_call_id
            WHERE worker.id=parameter_authority.worker_run_id
              AND worker.attempt_epoch=parameter_authority.worker_attempt_epoch
              AND worker.lease_token=parameter_authority.lease_token
              AND worker.status IN ('running','waiting_background')
              AND worker.lease_expires_at>statement_timestamp()
              AND call.worker_run_id=worker.id
              AND call.attempt_epoch=worker.attempt_epoch
              AND call.lease_token=worker.lease_token
            FOR SHARE OF worker,call
       ) THEN
        RAISE EXCEPTION 'enumeration_parameter_worker_authority_mismatch'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM capability_execution_receipt_inputs i
        JOIN coverage_denominators parameter_denominator
          ON parameter_denominator.id=NEW.denominator_id
         AND parameter_denominator.execution_authority_id=NEW.execution_authority_id
        JOIN enumeration_endpoint_occurrences occurrence ON occurrence.id=NEW.occurrence_id
        JOIN web_origins source_origin ON source_origin.id=occurrence.source_web_origin_id
        JOIN coverage_denominators parameter_root
          ON parameter_root.id=parameter_denominator.parent_denominator_id
         AND parameter_root.execution_authority_id=NEW.execution_authority_id
         AND parameter_root.denominator_kind='root'
         AND parameter_root.sealed_at IS NOT NULL
        JOIN coverage_denominator_items root_item
          ON root_item.id=parameter_denominator.parent_denominator_item_id
         AND root_item.denominator_id=parameter_root.id
         AND root_item.execution_authority_id=NEW.execution_authority_id
         WHERE i.id=NEW.terminal_receipt_input_id AND i.receipt_id=NEW.terminal_receipt_id
           AND i.denominator_item_id=NEW.denominator_item_id
           AND i.execution_authority_id=NEW.execution_authority_id
           AND i.sealed_at IS NOT NULL
           AND parameter_denominator.denominator_kind='derived_child'
           AND parameter_denominator.sealed_at IS NOT NULL
           AND enumeration_denominator_has_worker_root(
               parameter_denominator.id,parameter_denominator.execution_authority_id
           )
           AND root_item.target_id=occurrence.source_target_id
           AND root_item.exact_asset=source_origin.origin
           AND root_item.technique='GOLISH-ENUM-PARAM'
           AND root_item.expected_capability='enum.collect_browser_surface'
           AND EXISTS (
               SELECT 1 FROM enumeration_receipt_input_census_seals census
                WHERE census.receipt_id=NEW.terminal_receipt_id
                  AND census.denominator_id=NEW.denominator_id
                  AND census.execution_authority_id=NEW.execution_authority_id
           )
           AND (
               (NEW.parameter_outcome='found'
                   AND i.attempt_state='succeeded' AND i.landing_state='committed'
                   AND i.observation_state='found' AND i.coverage_extent='complete'
                   AND i.coverage_gap_reason='none')
               OR (NEW.parameter_outcome='checked_empty'
                   AND i.attempt_state='succeeded' AND i.landing_state='committed'
                   AND i.observation_state='no_match' AND i.coverage_extent='complete'
                   AND i.coverage_gap_reason='none')
               OR (NEW.parameter_outcome='not_applicable'
                   AND i.attempt_state='succeeded' AND i.landing_state='committed'
                   AND i.observation_state='not_applicable' AND i.coverage_extent='complete'
                   AND i.coverage_gap_reason='none')
               OR (NEW.parameter_outcome='unresolved'
                   AND i.attempt_state IN ('failed','outcome_unknown','exhausted')
                   AND i.coverage_extent IN ('none','partial','sampled','template_only')
                   AND i.coverage_gap_reason<>'none')
           )
         FOR SHARE OF i,parameter_denominator,parameter_root,root_item,occurrence,source_origin
    ) THEN
        RAISE EXCEPTION 'enumeration_parameter_terminal_receipt_required' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_parameter_assessments_validate
BEFORE INSERT ON enumeration_endpoint_parameter_assessments
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_parameter_assessment();
CREATE INDEX enumeration_endpoint_parameter_assessments_occurrence_idx
    ON enumeration_endpoint_parameter_assessments(occurrence_id,parameter_outcome);

CREATE TABLE enumeration_endpoint_occurrence_parameters (
    id UUID PRIMARY KEY,
    assessment_id UUID NOT NULL,
    assessment_execution_authority_id UUID NOT NULL,
    name TEXT NOT NULL CHECK (BTRIM(name)<>''),
    location TEXT NOT NULL CHECK (
        location IN ('query','body_or_form','body','form','path','header','graphql_variable','unknown')
    ),
    value_type TEXT NOT NULL DEFAULT 'unknown' CHECK (BTRIM(value_type)<>''),
    requirement TEXT NOT NULL CHECK (requirement IN ('required','optional','unknown')),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    source_anchor TEXT NOT NULL CHECK (BTRIM(source_anchor)<>''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(assessment_id,location,name),
    FOREIGN KEY(assessment_id,assessment_execution_authority_id)
        REFERENCES enumeration_endpoint_parameter_assessments(id,execution_authority_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_occurrence_parameters_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrence_parameters
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_occurrence_parameter_immutable');

CREATE FUNCTION enumeration_validate_parameter_assessment_shape()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE parameter_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO parameter_count
      FROM enumeration_endpoint_occurrence_parameters
     WHERE assessment_id=NEW.id;
    IF (NEW.parameter_outcome='found' AND parameter_count=0)
       OR (NEW.parameter_outcome<>'found' AND parameter_count<>0)
       OR NOT EXISTS (
            SELECT 1
              FROM enumeration_endpoint_occurrence_evidence evidence
             WHERE evidence.occurrence_id=NEW.occurrence_id
               AND evidence.evidence_role='parameter'
               AND evidence.parameter_assessment_id=NEW.id
               AND evidence.parameter_assessment_execution_authority_id=NEW.execution_authority_id
               AND evidence.evidence_execution_authority_id=NEW.execution_authority_id
       ) THEN
        RAISE EXCEPTION 'enumeration_parameter_assessment_terminal_shape_invalid' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER enumeration_endpoint_parameter_assessment_shape
AFTER INSERT ON enumeration_endpoint_parameter_assessments
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_parameter_assessment_shape();

-- ---------------------------------------------------------------------------
-- Server-derived groups and legacy projection links.
-- ---------------------------------------------------------------------------

CREATE TABLE enumeration_endpoint_groups (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    resolved_target_id UUID NOT NULL,
    resolved_web_origin_id UUID NOT NULL,
    protocol TEXT NOT NULL CHECK (protocol IN ('http','https','websocket','graphql')),
    method TEXT NOT NULL,
    route_kind TEXT NOT NULL CHECK (route_kind IN ('exact','template')),
    route_template TEXT NOT NULL CHECK (BTRIM(route_template)<>''),
    graphql_operation_name TEXT NOT NULL DEFAULT '',
    group_identity_hash TEXT NOT NULL CHECK (group_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,resolved_web_origin_id,protocol,method,route_kind,route_template,graphql_operation_name),
    UNIQUE(id,operation_id),
    CONSTRAINT enumeration_endpoint_groups_identity UNIQUE(id,operation_id,scope_snapshot_id,organization_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,operation_id,project_scope_id,project_path_at_freeze)
        REFERENCES operation_org_scope_snapshots(
            id,operation_id,project_scope_id,project_path_at_freeze) ON DELETE RESTRICT,
    FOREIGN KEY(resolved_target_id,organization_id,project_path_at_freeze)
        REFERENCES targets(id,organization_id,project_path) ON DELETE RESTRICT,
    FOREIGN KEY(resolved_web_origin_id,organization_id,project_path_at_freeze)
        REFERENCES web_origins(id,organization_id,project_path) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_groups_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_groups
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_group_immutable');

CREATE FUNCTION enumeration_validate_endpoint_group()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_hash TEXT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM operation_state operation
         WHERE operation.operation_id=NEW.operation_id
           AND operation.project_scope_id=NEW.project_scope_id
           AND operation.enumeration_analysis_contract='agent_team_v2'
           AND operation.tool_truth_contract='receipt_v1'
         FOR SHARE
    ) OR NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrences occurrence
         WHERE occurrence.operation_id=NEW.operation_id
           AND occurrence.scope_snapshot_id=NEW.scope_snapshot_id
           AND occurrence.organization_id=NEW.organization_id
           AND occurrence.resolved_target_id=NEW.resolved_target_id
           AND occurrence.resolved_web_origin_id=NEW.resolved_web_origin_id
           AND occurrence.protocol=NEW.protocol
           AND upper(occurrence.method)=NEW.method
           AND occurrence.route_kind=NEW.route_kind
           AND CASE WHEN occurrence.route_kind='exact'
                    THEN occurrence.canonical_request_url ELSE occurrence.route_template END
               =NEW.route_template
           AND COALESCE(occurrence.graphql_operation_name,'')=NEW.graphql_operation_name
           AND occurrence.resolution_status='resolved'
           AND occurrence.scope_decision='in_scope'
           AND occurrence.candidate_classification='endpoint'
           AND occurrence.inference_level IN ('observed','deterministic')
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_endpoint_group_not_projectable' USING ERRCODE='23514';
    END IF;
    expected_hash := tool_truth_sha256(jsonb_build_object(
        'operation_id',NEW.operation_id,'origin',NEW.resolved_web_origin_id,
        'protocol',NEW.protocol,'method',NEW.method,'route_kind',NEW.route_kind,
        'route',NEW.route_template,
        'graphql_operation_name',NEW.graphql_operation_name
    )::TEXT);
    NEW.group_identity_hash := expected_hash;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_groups_validate
BEFORE INSERT ON enumeration_endpoint_groups
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_endpoint_group();

CREATE TABLE enumeration_endpoint_occurrence_group_links (
    occurrence_id UUID NOT NULL,
    group_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    match_kind TEXT NOT NULL CHECK (match_kind IN ('exact','unique_template')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(occurrence_id,group_id),
    FOREIGN KEY(occurrence_id,operation_id,scope_snapshot_id,organization_id)
        REFERENCES enumeration_endpoint_occurrences(
            id,operation_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(group_id,operation_id,scope_snapshot_id,organization_id)
        REFERENCES enumeration_endpoint_groups(
            id,operation_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_occurrence_group_links_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrence_group_links
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_occurrence_group_link_immutable');

CREATE FUNCTION enumeration_validate_occurrence_group_link()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE occurrence enumeration_endpoint_occurrences%ROWTYPE;
DECLARE endpoint_group enumeration_endpoint_groups%ROWTYPE;
DECLARE template_match_count BIGINT;
BEGIN
    SELECT * INTO occurrence FROM enumeration_endpoint_occurrences
     WHERE id=NEW.occurrence_id FOR SHARE;
    SELECT * INTO endpoint_group FROM enumeration_endpoint_groups
     WHERE id=NEW.group_id FOR SHARE;
    IF occurrence.operation_id<>endpoint_group.operation_id
       OR occurrence.scope_snapshot_id<>endpoint_group.scope_snapshot_id
       OR occurrence.organization_id<>endpoint_group.organization_id
       OR occurrence.resolved_web_origin_id IS DISTINCT FROM endpoint_group.resolved_web_origin_id
       OR occurrence.protocol<>endpoint_group.protocol
       OR upper(occurrence.method)<>endpoint_group.method
       OR COALESCE(occurrence.graphql_operation_name,'')<>endpoint_group.graphql_operation_name THEN
        RAISE EXCEPTION 'enumeration_occurrence_group_authority_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.match_kind='exact' AND NOT (
        occurrence.route_kind='exact' AND endpoint_group.route_kind='exact'
        AND occurrence.canonical_request_url=endpoint_group.route_template
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_group_exact_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.match_kind='unique_template' THEN
        IF occurrence.route_kind='template' THEN
            IF endpoint_group.route_kind<>'template'
               OR occurrence.route_template<>endpoint_group.route_template THEN
                RAISE EXCEPTION 'enumeration_occurrence_group_template_mismatch' USING ERRCODE='23514';
            END IF;
        ELSE
            SELECT COUNT(*) INTO template_match_count
              FROM enumeration_endpoint_groups candidate
             WHERE candidate.operation_id=occurrence.operation_id
               AND candidate.resolved_web_origin_id=occurrence.resolved_web_origin_id
               AND candidate.protocol=occurrence.protocol
               AND candidate.method=upper(occurrence.method)
               AND candidate.route_kind='template'
               AND candidate.graphql_operation_name=COALESCE(occurrence.graphql_operation_name,'')
               AND enumeration_route_template_matches(
                   candidate.route_template,occurrence.runtime_sample_url
               );
            IF endpoint_group.route_kind<>'template' OR template_match_count<>1
               OR occurrence.runtime_sample_url IS NULL
               OR NOT enumeration_route_template_matches(
                   endpoint_group.route_template,occurrence.runtime_sample_url
               ) THEN
                RAISE EXCEPTION 'enumeration_occurrence_group_template_ambiguous' USING ERRCODE='23514';
            END IF;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_occurrence_group_links_validate
BEFORE INSERT ON enumeration_endpoint_occurrence_group_links
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_occurrence_group_link();

CREATE TABLE enumeration_endpoint_group_api_links (
    group_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    endpoint_id UUID NOT NULL REFERENCES api_endpoints(id) ON DELETE RESTRICT,
    endpoint_observation_id UUID NOT NULL REFERENCES enumeration_endpoint_observations(id) ON DELETE RESTRICT,
    projection_source TEXT NOT NULL DEFAULT 'occurrence_v2_aggregate'
        CHECK (projection_source='occurrence_v2_aggregate'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(group_id,operation_id)
        REFERENCES enumeration_endpoint_groups(id,operation_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_group_api_links_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_group_api_links
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_group_api_link_immutable');

CREATE FUNCTION enumeration_validate_group_api_link()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_groups endpoint_group
        JOIN operation_state operation ON operation.operation_id=endpoint_group.operation_id
        JOIN api_endpoints endpoint ON endpoint.id=NEW.endpoint_id
        JOIN enumeration_endpoint_observations observation
          ON observation.id=NEW.endpoint_observation_id
         AND observation.endpoint_id=endpoint.id
         AND observation.operation_id=endpoint_group.operation_id
         AND observation.organization_id=endpoint_group.organization_id
         AND observation.target_id=endpoint_group.resolved_target_id
         AND observation.web_origin_id=endpoint_group.resolved_web_origin_id
        WHERE endpoint_group.id=NEW.group_id AND endpoint_group.operation_id=NEW.operation_id
          AND operation.enumeration_analysis_contract='agent_team_v2'
          AND operation.tool_truth_contract='receipt_v1'
          AND endpoint_group.protocol IN ('http','https')
          AND endpoint.target_id=endpoint_group.resolved_target_id
          AND endpoint.method=endpoint_group.method
          AND endpoint.source='occurrence_v2_aggregate'
          AND observation.source='occurrence_v2_aggregate'
          AND (
              (endpoint_group.route_kind='exact' AND endpoint.url=endpoint_group.route_template)
              OR EXISTS (
                  SELECT 1 FROM enumeration_endpoint_occurrence_group_links link
                  JOIN enumeration_endpoint_occurrences occurrence ON occurrence.id=link.occurrence_id
                  WHERE link.group_id=endpoint_group.id AND occurrence.runtime_sample_url=endpoint.url
              )
          )
        FOR SHARE OF endpoint_group,operation,endpoint,observation
    ) THEN
        RAISE EXCEPTION 'enumeration_endpoint_group_api_projection_invalid' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_group_api_links_validate
BEFORE INSERT ON enumeration_endpoint_group_api_links
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_group_api_link();

CREATE TABLE enumeration_endpoint_occurrence_evidence (
    occurrence_id UUID NOT NULL,
    occurrence_execution_authority_id UUID NOT NULL,
    evidence_execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    tool_truth_evidence_authority_id UUID NOT NULL,
    authority_hash TEXT NOT NULL CHECK (authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_role TEXT NOT NULL CHECK (evidence_role IN ('discovery','resolution','parameter')),
    parameter_assessment_id UUID,
    parameter_assessment_execution_authority_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(occurrence_id,tool_truth_evidence_authority_id,evidence_role),
    CHECK ((evidence_role='parameter')=(parameter_assessment_id IS NOT NULL)),
    CHECK ((parameter_assessment_id IS NULL)
        =(parameter_assessment_execution_authority_id IS NULL)),
    FOREIGN KEY(occurrence_id,operation_id,project_scope_id,scope_snapshot_id,
                organization_id,occurrence_execution_authority_id)
        REFERENCES enumeration_endpoint_occurrences(
            id,operation_id,project_scope_id,scope_snapshot_id,organization_id,
            execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_evidence_authority_id,evidence_execution_authority_id,authority_hash)
        REFERENCES tool_truth_evidence_authorities(id,execution_authority_id,authority_hash) ON DELETE RESTRICT
    ,FOREIGN KEY(parameter_assessment_id,parameter_assessment_execution_authority_id,
                 occurrence_id,occurrence_execution_authority_id,operation_id,project_scope_id,
                 scope_snapshot_id,organization_id)
        REFERENCES enumeration_endpoint_parameter_assessments(
            id,execution_authority_id,occurrence_id,occurrence_execution_authority_id,
            operation_id,project_scope_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT
);
CREATE TRIGGER enumeration_endpoint_occurrence_evidence_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrence_evidence
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable('enumeration_endpoint_occurrence_evidence_immutable');

CREATE FUNCTION enumeration_validate_occurrence_evidence()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE occurrence_authority tool_truth_execution_authorities%ROWTYPE;
DECLARE evidence_authority tool_truth_execution_authorities%ROWTYPE;
BEGIN
    SELECT * INTO occurrence_authority FROM tool_truth_execution_authorities
     WHERE id=NEW.occurrence_execution_authority_id FOR SHARE;
    SELECT * INTO evidence_authority FROM tool_truth_execution_authorities
     WHERE id=NEW.evidence_execution_authority_id FOR SHARE;
    IF occurrence_authority.id IS NULL OR evidence_authority.id IS NULL
       OR occurrence_authority.operation_id<>NEW.operation_id
       OR evidence_authority.operation_id<>NEW.operation_id
       OR occurrence_authority.project_scope_id<>NEW.project_scope_id
       OR evidence_authority.project_scope_id<>NEW.project_scope_id
       OR occurrence_authority.scope_snapshot_id<>NEW.scope_snapshot_id
       OR evidence_authority.scope_snapshot_id<>NEW.scope_snapshot_id
       OR occurrence_authority.organization_id<>NEW.organization_id
       OR evidence_authority.organization_id<>NEW.organization_id
       OR occurrence_authority.stage_execution_id<>evidence_authority.stage_execution_id
       OR occurrence_authority.stage_run_unit_id IS DISTINCT FROM evidence_authority.stage_run_unit_id
       OR (NEW.evidence_role='discovery'
           AND NEW.evidence_execution_authority_id<>NEW.occurrence_execution_authority_id)
       OR (NEW.evidence_role='parameter'
           AND NEW.evidence_execution_authority_id
               <>NEW.parameter_assessment_execution_authority_id) THEN
        RAISE EXCEPTION 'enumeration_occurrence_evidence_authority_mismatch'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_occurrence_evidence_validate
BEFORE INSERT ON enumeration_endpoint_occurrence_evidence
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_occurrence_evidence();

CREATE FUNCTION enumeration_validate_occurrence_evidence_shape()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrence_evidence evidence
         WHERE evidence.occurrence_id=NEW.id AND evidence.evidence_role='discovery'
    ) OR (NEW.resolution_status IN ('resolved','ambiguous','unresolved') AND NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrence_evidence evidence
         WHERE evidence.occurrence_id=NEW.id AND evidence.evidence_role='resolution'
    )) THEN
        RAISE EXCEPTION 'enumeration_occurrence_normalized_evidence_incomplete' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER enumeration_endpoint_occurrence_evidence_shape
AFTER INSERT ON enumeration_endpoint_occurrences
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_occurrence_evidence_shape();

CREATE TABLE enumeration_endpoint_candidate_closure_receipts (
    id UUID PRIMARY KEY,
    stable_closure_request_id UUID NOT NULL UNIQUE,
    candidate_input_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL,
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_input_id UUID NOT NULL,
    resolution_execution_authority_id UUID,
    resolution_terminal_receipt_id UUID,
    resolution_terminal_receipt_input_id UUID,
    terminal_disposition TEXT NOT NULL CHECK (terminal_disposition IN (
        'resolved','resolved_non_promotable','scope_excluded','noise',
        'unresolved_exhausted','ambiguous_exhausted','not_applicable'
    )),
    occurrence_count BIGINT NOT NULL CHECK (occurrence_count>0),
    occurrence_set_hash TEXT NOT NULL CHECK (occurrence_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((resolution_execution_authority_id IS NULL)
        =(resolution_terminal_receipt_id IS NULL)),
    CHECK ((resolution_terminal_receipt_id IS NULL)
        =(resolution_terminal_receipt_input_id IS NULL)),
    FOREIGN KEY(candidate_input_id,execution_authority_id,terminal_receipt_input_id)
        REFERENCES enumeration_endpoint_candidate_inputs(
            id,execution_authority_id,terminal_receipt_input_id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
            id,receipt_id,execution_authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(resolution_terminal_receipt_input_id,resolution_terminal_receipt_id,
                resolution_execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(id,receipt_id,execution_authority_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_candidate_closure_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_count BIGINT;
DECLARE expected_hash TEXT;
DECLARE expected_disposition TEXT;
DECLARE candidate_authority tool_truth_execution_authorities%ROWTYPE;
DECLARE resolution_authority tool_truth_execution_authorities%ROWTYPE;
BEGIN
    PERFORM 1 FROM enumeration_endpoint_candidate_inputs candidate
     WHERE candidate.id=NEW.candidate_input_id
       AND candidate.execution_authority_id=NEW.execution_authority_id
       AND candidate.terminal_receipt_id=NEW.terminal_receipt_id
       AND candidate.terminal_receipt_input_id=NEW.terminal_receipt_input_id
     FOR UPDATE;
    SELECT authority.* INTO candidate_authority
      FROM enumeration_endpoint_candidate_inputs candidate
      JOIN tool_truth_execution_authorities authority
        ON authority.id=candidate.execution_authority_id
     WHERE candidate.id=NEW.candidate_input_id
       AND candidate.execution_authority_id=NEW.execution_authority_id
       AND candidate.terminal_receipt_id=NEW.terminal_receipt_id
       AND candidate.terminal_receipt_input_id=NEW.terminal_receipt_input_id
     FOR SHARE OF authority;
    PERFORM 1 FROM enumeration_endpoint_occurrences occurrence
     WHERE occurrence.candidate_input_id=NEW.candidate_input_id
       AND occurrence.execution_authority_id=NEW.execution_authority_id
     ORDER BY occurrence.id FOR SHARE;
    SELECT COUNT(*)::BIGINT,
           tool_truth_sha256(COALESCE(jsonb_agg(jsonb_build_object(
               'occurrence_id',occurrence.id,
               'parent_occurrence_id',occurrence.parent_occurrence_id,
               'observation_kind',occurrence.observation_kind,
               'inference_level',occurrence.inference_level,
               'resolution_status',occurrence.resolution_status,
               'scope_decision',occurrence.scope_decision,
               'candidate_classification',occurrence.candidate_classification,
               'route_kind',occurrence.route_kind,
               'evidence',(
                   SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'role',evidence.evidence_role,
                       'authority_id',evidence.tool_truth_evidence_authority_id,
                       'authority_hash',evidence.authority_hash
                   ) ORDER BY evidence.evidence_role,evidence.tool_truth_evidence_authority_id),
                   '[]'::JSONB)
                     FROM enumeration_endpoint_occurrence_evidence evidence
                    WHERE evidence.occurrence_id=occurrence.id
               ),
               'capture_events',(
                   SELECT COALESCE(jsonb_agg(link.capture_event_id ORDER BY link.capture_event_id),
                                   '[]'::JSONB)
                     FROM enumeration_endpoint_occurrence_capture_events link
                    WHERE link.occurrence_id=occurrence.id
               )
           ) ORDER BY occurrence.id),'[]'::JSONB)::TEXT),
           CASE
             WHEN BOOL_OR(occurrence.promotion_eligible) THEN 'resolved'
             WHEN BOOL_AND(occurrence.scope_decision='scope_excluded') THEN 'scope_excluded'
             WHEN BOOL_AND(occurrence.candidate_classification='noise') THEN 'noise'
             WHEN BOOL_OR(occurrence.resolution_status='ambiguous') THEN 'ambiguous_exhausted'
             WHEN BOOL_OR(occurrence.resolution_status='unresolved') THEN 'unresolved_exhausted'
             WHEN BOOL_OR(occurrence.resolution_status='resolved') THEN 'resolved_non_promotable'
             ELSE 'not_applicable'
           END
      INTO expected_count,expected_hash,expected_disposition
      FROM enumeration_endpoint_occurrences occurrence
     WHERE occurrence.candidate_input_id=NEW.candidate_input_id
       AND occurrence.execution_authority_id=NEW.execution_authority_id
    ;
    IF candidate_authority.id IS NULL OR expected_count=0 THEN
        RAISE EXCEPTION 'enumeration_candidate_closure_missing_occurrence'
            USING ERRCODE='23514';
    END IF;
    IF expected_disposition IN (
        'resolved_non_promotable','unresolved_exhausted','ambiguous_exhausted'
    ) THEN
        SELECT * INTO resolution_authority FROM tool_truth_execution_authorities
         WHERE id=NEW.resolution_execution_authority_id FOR SHARE;
        IF resolution_authority.id IS NULL
           OR resolution_authority.operation_id<>candidate_authority.operation_id
           OR resolution_authority.project_scope_id<>candidate_authority.project_scope_id
           OR resolution_authority.scope_snapshot_id<>candidate_authority.scope_snapshot_id
           OR resolution_authority.organization_id<>candidate_authority.organization_id
           OR resolution_authority.stage_execution_id<>candidate_authority.stage_execution_id
           OR resolution_authority.stage_run_unit_id IS DISTINCT FROM candidate_authority.stage_run_unit_id
           OR NOT EXISTS (
               SELECT 1 FROM capability_execution_receipt_inputs input
                WHERE input.id=NEW.resolution_terminal_receipt_input_id
                  AND input.receipt_id=NEW.resolution_terminal_receipt_id
                  AND input.execution_authority_id=NEW.resolution_execution_authority_id
                  AND input.sealed_at IS NOT NULL
                  AND input.attempt_state IN ('failed','outcome_unknown','exhausted')
                  AND input.coverage_gap_reason IN (
                      'transport','tool_failure','parser_reject','budget_exhausted',
                      'unsupported','policy_blocked','source_unavailable'
                  )
                FOR SHARE
           ) THEN
            RAISE EXCEPTION 'enumeration_candidate_resolution_terminal_receipt_required'
                USING ERRCODE='23514';
        END IF;
    ELSIF NEW.resolution_terminal_receipt_input_id IS NOT NULL THEN
        RAISE EXCEPTION 'enumeration_candidate_resolution_receipt_not_applicable'
            USING ERRCODE='23514';
    END IF;
    NEW.occurrence_count := expected_count;
    NEW.occurrence_set_hash := expected_hash;
    NEW.terminal_disposition := expected_disposition;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_candidate_closure_receipts_validate
BEFORE INSERT ON enumeration_endpoint_candidate_closure_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_candidate_closure_receipt();
CREATE TRIGGER enumeration_endpoint_candidate_closure_receipts_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_candidate_closure_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_endpoint_candidate_closure_receipt_immutable'
);

CREATE TABLE enumeration_endpoint_candidate_denominator_closure_receipts (
    id UUID PRIMARY KEY,
    stable_closure_request_id UUID NOT NULL UNIQUE,
    denominator_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK (member_count>=0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(denominator_id,execution_authority_id)
        REFERENCES coverage_denominators(id,execution_authority_id) ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_candidate_denominator_closure()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE denominator_count BIGINT;
DECLARE candidate_count BIGINT;
DECLARE closure_count BIGINT;
DECLARE expected_hash TEXT;
BEGIN
    SELECT denominator.member_count INTO denominator_count
      FROM coverage_denominators denominator
     WHERE denominator.id=NEW.denominator_id
       AND denominator.execution_authority_id=NEW.execution_authority_id
       AND denominator.sealed_at IS NOT NULL
     FOR UPDATE;
    PERFORM 1 FROM enumeration_endpoint_candidate_inputs candidate
     WHERE candidate.denominator_id=NEW.denominator_id
       AND candidate.execution_authority_id=NEW.execution_authority_id
     ORDER BY candidate.denominator_item_id FOR SHARE;
    PERFORM 1 FROM enumeration_endpoint_candidate_closure_receipts closure
      JOIN enumeration_endpoint_candidate_inputs candidate
        ON candidate.id=closure.candidate_input_id
     WHERE candidate.denominator_id=NEW.denominator_id
       AND candidate.execution_authority_id=NEW.execution_authority_id
     ORDER BY closure.id FOR SHARE OF closure;
    SELECT COUNT(*)::BIGINT,COUNT(closure.id)::BIGINT,
           tool_truth_sha256(COALESCE(jsonb_agg(jsonb_build_object(
               'denominator_item_id',candidate.denominator_item_id,
               'candidate_input_id',candidate.id,
               'logical_input_key',candidate.logical_input_key,
               'duplicate_ordinal',candidate.duplicate_ordinal,
               'closure_id',closure.id,
               'closure_hash',closure.occurrence_set_hash,
               'terminal_disposition',closure.terminal_disposition
           ) ORDER BY candidate.denominator_item_id),'[]'::JSONB)::TEXT)
      INTO candidate_count,closure_count,expected_hash
      FROM enumeration_endpoint_candidate_inputs candidate
      LEFT JOIN enumeration_endpoint_candidate_closure_receipts closure
        ON closure.candidate_input_id=candidate.id
     WHERE candidate.denominator_id=NEW.denominator_id
       AND candidate.execution_authority_id=NEW.execution_authority_id
    ;
    IF denominator_count IS NULL
       OR candidate_count<>denominator_count OR closure_count<>denominator_count THEN
        RAISE EXCEPTION 'enumeration_candidate_denominator_exact_closure_incomplete'
            USING ERRCODE='23514';
    END IF;
    NEW.member_count := denominator_count;
    NEW.member_set_hash := expected_hash;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_candidate_denominator_closure_validate
BEFORE INSERT ON enumeration_endpoint_candidate_denominator_closure_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_candidate_denominator_closure();
CREATE TRIGGER enumeration_endpoint_candidate_denominator_closure_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_candidate_denominator_closure_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_endpoint_candidate_denominator_closure_immutable'
);

CREATE FUNCTION enumeration_guard_closed_candidate_children()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE candidate_id UUID;
BEGIN
    IF TG_TABLE_NAME='enumeration_endpoint_occurrences' THEN
        candidate_id := NEW.candidate_input_id;
    ELSIF TG_TABLE_NAME='enumeration_endpoint_occurrence_evidence' THEN
        -- Parameter evidence belongs to the later, separately fenced
        -- Parameter lane.  Candidate closure freezes discovery/resolution
        -- children only; the Parameter lane receipt freezes this row below.
        IF NEW.evidence_role='parameter' THEN
            RETURN NEW;
        END IF;
        SELECT occurrence.candidate_input_id INTO candidate_id
          FROM enumeration_endpoint_occurrences occurrence
         WHERE occurrence.id=NEW.occurrence_id;
    ELSE
        candidate_id := NEW.candidate_input_id;
    END IF;
    PERFORM 1 FROM enumeration_endpoint_candidate_inputs candidate
     WHERE candidate.id=candidate_id FOR SHARE;
    IF EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_closure_receipts closure
         WHERE closure.candidate_input_id=candidate_id FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_candidate_terminal_children_immutable'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER zz_enumeration_endpoint_occurrences_closed_guard
BEFORE INSERT ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_candidate_children();
CREATE TRIGGER zz_enumeration_endpoint_occurrence_evidence_closed_guard
BEFORE INSERT ON enumeration_endpoint_occurrence_evidence
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_candidate_children();
CREATE TRIGGER zz_enumeration_endpoint_candidate_capture_events_closed_guard
BEFORE INSERT ON enumeration_endpoint_candidate_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_candidate_children();
CREATE TRIGGER zz_enumeration_endpoint_occurrence_capture_events_closed_guard
BEFORE INSERT ON enumeration_endpoint_occurrence_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_candidate_children();

CREATE FUNCTION enumeration_guard_closed_parameter_children()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE parameter_authority_id UUID;
BEGIN
    IF TG_TABLE_NAME='enumeration_endpoint_parameter_assessments' THEN
        parameter_authority_id := NEW.execution_authority_id;
    ELSIF TG_TABLE_NAME='enumeration_endpoint_occurrence_parameters' THEN
        SELECT assessment.execution_authority_id INTO parameter_authority_id
          FROM enumeration_endpoint_parameter_assessments assessment
         WHERE assessment.id=NEW.assessment_id
         FOR SHARE;
    ELSE
        IF NEW.evidence_role<>'parameter' THEN
            RETURN NEW;
        END IF;
        parameter_authority_id := NEW.parameter_assessment_execution_authority_id;
    END IF;
    IF parameter_authority_id IS NULL OR EXISTS (
        SELECT 1 FROM enumeration_lane_commit_receipts receipt
         WHERE receipt.execution_authority_id=parameter_authority_id
           AND receipt.lane='parameter'
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_parameter_terminal_children_immutable'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER zzz_enumeration_parameter_assessments_closed_guard
BEFORE INSERT ON enumeration_endpoint_parameter_assessments
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_parameter_children();
CREATE TRIGGER zzz_enumeration_occurrence_parameters_closed_guard
BEFORE INSERT ON enumeration_endpoint_occurrence_parameters
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_parameter_children();
CREATE TRIGGER zzz_enumeration_parameter_evidence_closed_guard
BEFORE INSERT ON enumeration_endpoint_occurrence_evidence
FOR EACH ROW EXECUTE FUNCTION enumeration_guard_closed_parameter_children();

-- ---------------------------------------------------------------------------
-- Extend the closed Tool Truth business-reference vocabulary.
-- ---------------------------------------------------------------------------

ALTER TABLE tool_truth_business_ref_authorities
    DROP CONSTRAINT tool_truth_business_ref_kind_check,
    ADD CONSTRAINT tool_truth_business_ref_kind_check CHECK (ref_kind IN (
        'target_asset','dns_record','web_origin_observation','network_endpoint',
        'enumeration_endpoint_observation','enumeration_endpoint_occurrence',
        'enumeration_endpoint_group'
    ));

CREATE OR REPLACE FUNCTION tool_truth_validate_business_ref()
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
                   'asset_type',a.asset_type,'value',a.value,'port',a.port,'protocol',a.protocol),a.discovered_at
          INTO expected_snapshot,expected_observed_at FROM target_assets a JOIN targets t ON t.id=a.target_id
         WHERE a.id=NEW.ref_uuid AND t.organization_id=authority.organization_id
           AND a.project_path=authority.project_path_at_freeze AND t.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='dns_record' THEN
        SELECT jsonb_build_object('kind','dns_record','id',d.id,'target_id',d.target_id,
                   'record_type',d.record_type,'name',d.name,'value',d.value,'source',d.source),d.created_at
          INTO expected_snapshot,expected_observed_at FROM dns_records d JOIN targets t ON t.id=d.target_id
         WHERE d.id=NEW.ref_bigint AND t.organization_id=authority.organization_id
           AND d.project_path=authority.project_path_at_freeze AND t.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='web_origin_observation' THEN
        SELECT jsonb_build_object('kind','web_origin_observation','id',o.id,'web_origin_id',o.web_origin_id,
                   'network_endpoint_id',o.network_endpoint_id,'target_id',o.target_id,
                   'status_code',o.status_code,'source',o.source),o.observed_at
          INTO expected_snapshot,expected_observed_at FROM web_origin_observations o
          JOIN web_origins w ON w.id=o.web_origin_id LEFT JOIN network_endpoints n ON n.id=o.network_endpoint_id
          LEFT JOIN targets t ON t.id=o.target_id
         WHERE o.id=NEW.ref_uuid AND o.organization_id=authority.organization_id
           AND o.project_path=authority.project_path_at_freeze AND w.organization_id=authority.organization_id
           AND w.project_path=authority.project_path_at_freeze
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
          INTO expected_snapshot,expected_observed_at FROM enumeration_endpoint_observations o
          JOIN targets t ON t.id=o.target_id JOIN web_origins w ON w.id=o.web_origin_id
          JOIN api_endpoints e ON e.id=o.endpoint_id
         WHERE o.id=NEW.ref_uuid AND o.operation_id=authority.operation_id
           AND o.organization_id=authority.organization_id AND o.project_path=authority.project_path_at_freeze
           AND t.organization_id=authority.organization_id AND t.project_path=authority.project_path_at_freeze
           AND w.organization_id=authority.organization_id AND w.project_path=authority.project_path_at_freeze
           AND e.target_id=t.id AND e.project_path=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='enumeration_endpoint_occurrence' THEN
        SELECT jsonb_build_object('kind','enumeration_endpoint_occurrence','id',o.id,
                   'source_target_id',o.source_target_id,'source_web_origin_id',o.source_web_origin_id,
                   'resolved_target_id',o.resolved_target_id,'resolved_web_origin_id',o.resolved_web_origin_id,
                   'protocol',o.protocol,'method',o.method,'route_kind',o.route_kind,
                   'route_template',o.route_template,'graphql_operation_name',o.graphql_operation_name),o.created_at
          INTO expected_snapshot,expected_observed_at FROM enumeration_endpoint_occurrences o
         WHERE o.id=NEW.ref_uuid AND o.execution_authority_id=authority.id
           AND o.operation_id=authority.operation_id AND o.organization_id=authority.organization_id
           AND o.project_path_at_freeze=authority.project_path_at_freeze;
    ELSIF NEW.ref_kind='enumeration_endpoint_group' THEN
        SELECT jsonb_build_object('kind','enumeration_endpoint_group','id',g.id,
                   'resolved_target_id',g.resolved_target_id,'resolved_web_origin_id',g.resolved_web_origin_id,
                   'protocol',g.protocol,'method',g.method,'route_kind',g.route_kind,
                   'route_template',g.route_template,'graphql_operation_name',g.graphql_operation_name),g.created_at
          INTO expected_snapshot,expected_observed_at FROM enumeration_endpoint_groups g
         WHERE g.id=NEW.ref_uuid AND g.operation_id=authority.operation_id
           AND g.organization_id=authority.organization_id
           AND g.project_path_at_freeze=authority.project_path_at_freeze;
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

-- ---------------------------------------------------------------------------
-- Exact per-lane commit receipt. Later lanes carry explicitly named receipt
-- ids; there is deliberately no "latest receipt" selector. Zero entity
-- counts are valid only because the receipt itself records checked-empty.
-- ---------------------------------------------------------------------------

CREATE TABLE enumeration_lane_commit_receipts (
    id UUID PRIMARY KEY,
    stable_commit_request_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL UNIQUE,
    lane TEXT NOT NULL CHECK (lane IN ('browser','js_api','parameter','resolution','coverage')),
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    target_id UUID NOT NULL,
    exact_origin TEXT NOT NULL CHECK (
        BTRIM(exact_origin)<>'' AND POSITION('?' IN exact_origin)=0
        AND POSITION('#' IN exact_origin)=0
    ),
    artifact_sha256 TEXT NOT NULL CHECK (artifact_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    dependency_receipt_ids UUID[] NOT NULL DEFAULT '{}'::UUID[] CHECK (
        array_position(dependency_receipt_ids,NULL) IS NULL
    ),
    evidence_audit_ids BIGINT[] NOT NULL CHECK (
        CARDINALITY(evidence_audit_ids)>0 AND array_position(evidence_audit_ids,NULL) IS NULL
    ),
    script_denominator_id UUID,
    candidate_denominator_ids UUID[] NOT NULL DEFAULT '{}'::UUID[] CHECK (
        array_position(candidate_denominator_ids,NULL) IS NULL
    ),
    parameter_denominator_ids UUID[] NOT NULL DEFAULT '{}'::UUID[] CHECK (
        array_position(parameter_denominator_ids,NULL) IS NULL
    ),
    resolution_occurrence_id UUID,
    resolution_terminal_receipt_id UUID,
    resolution_terminal_receipt_input_id UUID,
    terminal_disposition TEXT NOT NULL CHECK (
        terminal_disposition IN ('found','checked_empty','terminal_with_residual')
    ),
    entity_set_sha256 TEXT NOT NULL CHECK (entity_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    denominator_set_sha256 TEXT NOT NULL CHECK (
        denominator_set_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    receipt_set_sha256 TEXT NOT NULL CHECK (receipt_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    script_count BIGINT NOT NULL CHECK (script_count>=0),
    candidate_count BIGINT NOT NULL CHECK (candidate_count>=0),
    occurrence_count BIGINT NOT NULL CHECK (occurrence_count>=0),
    parameter_assessment_count BIGINT NOT NULL CHECK (parameter_assessment_count>=0),
    parameter_fact_count BIGINT NOT NULL CHECK (parameter_fact_count>=0),
    unresolved_count BIGINT NOT NULL CHECK (unresolved_count>=0),
    missing BIGINT NOT NULL CHECK (missing=0),
    group_count BIGINT NOT NULL CHECK (group_count>=0),
    occurrence_link_count BIGINT NOT NULL CHECK (occurrence_link_count>=0),
    api_link_count BIGINT NOT NULL CHECK (api_link_count>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id,execution_authority_id),
    UNIQUE(resolution_occurrence_id),
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
                execution_authority_hash)
        REFERENCES tool_truth_execution_authorities(
                id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id,stage_kind,authority_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,scope_snapshot_id,
                organization_id,stage_kind)
        REFERENCES stage_run_units(id,operation_id,stage_execution_id,scope_snapshot_id,
                organization_id,stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY(target_id) REFERENCES targets(id) ON DELETE RESTRICT
    ,FOREIGN KEY(script_denominator_id,execution_authority_id)
        REFERENCES coverage_denominators(id,execution_authority_id) ON DELETE RESTRICT
    ,FOREIGN KEY(resolution_occurrence_id)
        REFERENCES enumeration_endpoint_occurrences(id) ON DELETE RESTRICT
    ,FOREIGN KEY(resolution_terminal_receipt_input_id,resolution_terminal_receipt_id,
                 execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
            id,receipt_id,execution_authority_id
        ) ON DELETE RESTRICT
    ,CHECK (
        (lane IN ('browser','js_api') AND script_denominator_id IS NOT NULL
            AND CARDINALITY(candidate_denominator_ids)>0
            AND CARDINALITY(parameter_denominator_ids)=0
            AND resolution_occurrence_id IS NULL
            AND resolution_terminal_receipt_id IS NULL
            AND resolution_terminal_receipt_input_id IS NULL)
        OR (lane='parameter' AND script_denominator_id IS NULL
            AND CARDINALITY(candidate_denominator_ids)=0
            AND resolution_occurrence_id IS NULL
            AND resolution_terminal_receipt_id IS NULL
            AND resolution_terminal_receipt_input_id IS NULL)
        OR (lane='resolution' AND script_denominator_id IS NULL
            AND CARDINALITY(candidate_denominator_ids)=0
            AND CARDINALITY(parameter_denominator_ids)=0
            AND resolution_occurrence_id IS NOT NULL
            AND resolution_terminal_receipt_id IS NOT NULL
            AND resolution_terminal_receipt_input_id IS NOT NULL)
        OR (lane='coverage' AND script_denominator_id IS NULL
            AND CARDINALITY(candidate_denominator_ids)=0
            AND CARDINALITY(parameter_denominator_ids)=0
            AND resolution_occurrence_id IS NULL
            AND resolution_terminal_receipt_id IS NULL
            AND resolution_terminal_receipt_input_id IS NULL)
    )
);

CREATE FUNCTION enumeration_validate_lane_commit_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE authority tool_truth_execution_authorities%ROWTYPE;
DECLARE normalized_evidence_count BIGINT;
DECLARE dependency_count BIGINT;
DECLARE dependency_lane_set TEXT[];
DECLARE expected_technique TEXT;
DECLARE expected_capability TEXT;
DECLARE root_denominator_id UUID;
DECLARE root_denominator_hash TEXT;
DECLARE root_item_id UUID;
DECLARE root_item_hash TEXT;
DECLARE source_origin_id UUID;
DECLARE producer_authority_ids UUID[] := '{}'::UUID[];
DECLARE producer_candidate_denominator_ids UUID[] := '{}'::UUID[];
DECLARE dependency_occurrence_ids UUID[] := '{}'::UUID[];
DECLARE assessment_occurrence_ids UUID[] := '{}'::UUID[];
DECLARE resolution_occurrence_ids UUID[] := '{}'::UUID[];
DECLARE exact_denominator_ids UUID[] := '{}'::UUID[];
DECLARE expected_denominator_count BIGINT := 0;
DECLARE exact_denominator_count BIGINT := 0;
DECLARE computed_script_count BIGINT := 0;
DECLARE computed_candidate_count BIGINT := 0;
DECLARE computed_occurrence_count BIGINT := 0;
DECLARE computed_parameter_assessment_count BIGINT := 0;
DECLARE computed_parameter_fact_count BIGINT := 0;
DECLARE computed_unresolved_count BIGINT := 0;
DECLARE computed_promotable_count BIGINT := 0;
DECLARE computed_group_count BIGINT := 0;
DECLARE computed_occurrence_link_count BIGINT := 0;
DECLARE computed_api_link_count BIGINT := 0;
DECLARE entity_material JSONB;
DECLARE denominator_material JSONB;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'enumeration-authority:'||NEW.execution_authority_id::TEXT,0
    ));
    SELECT * INTO authority FROM tool_truth_execution_authorities
     WHERE id=NEW.execution_authority_id FOR SHARE;
    IF NOT FOUND OR authority.execution_owner_kind<>'worker_tool'
       OR authority.stage_kind<>'enumeration'
       OR authority.operation_id<>NEW.operation_id
       OR authority.project_scope_id<>NEW.project_scope_id
       OR authority.project_path_at_freeze<>NEW.project_path_at_freeze
       OR authority.scope_snapshot_id<>NEW.scope_snapshot_id
       OR authority.organization_id<>NEW.organization_id
       OR authority.stage_execution_id<>NEW.stage_execution_id
       OR authority.stage_run_unit_id<>NEW.stage_run_unit_id
       OR authority.authority_hash<>NEW.execution_authority_hash THEN
        RAISE EXCEPTION 'enumeration_lane_commit_authority_mismatch' USING ERRCODE='23514';
    END IF;

    SELECT root.worker_root_denominator_id,root.worker_denominator_hash,origin.id
      INTO root_denominator_id,root_denominator_hash,source_origin_id
      FROM enumeration_worker_authority_roots root
      JOIN web_origins origin
        ON origin.organization_id=root.organization_id
       AND origin.project_path=root.project_path_at_freeze
       AND origin.origin=NEW.exact_origin
     WHERE root.worker_execution_authority_id=NEW.execution_authority_id
       AND enumeration_worker_root_has_exact_origin(
           NEW.execution_authority_id,NEW.target_id,origin.id
       )
     FOR SHARE OF root,origin;
    IF root_denominator_id IS NULL THEN
        RAISE EXCEPTION 'enumeration_lane_commit_subject_not_in_frozen_root'
            USING ERRCODE='23514';
    END IF;

    expected_technique := CASE NEW.lane
        WHEN 'browser' THEN 'GOLISH-ENUM-JS'
        WHEN 'js_api' THEN 'GOLISH-ENUM-JSAPI'
        WHEN 'parameter' THEN 'GOLISH-ENUM-PARAM'
        WHEN 'resolution' THEN 'GOLISH-ENUM-JSAPI'
        ELSE NULL
    END;
    expected_capability := CASE NEW.lane
        WHEN 'browser' THEN 'enum.collect_browser_surface'
        WHEN 'js_api' THEN 'enum.extract_js_apis'
        WHEN 'parameter' THEN 'enum.collect_browser_surface'
        WHEN 'resolution' THEN 'enum.extract_js_apis'
        ELSE NULL
    END;
    IF expected_technique IS NOT NULL THEN
        SELECT item.id,item.member_hash INTO root_item_id,root_item_hash
          FROM coverage_denominator_items item
         WHERE item.denominator_id=root_denominator_id
           AND item.execution_authority_id=NEW.execution_authority_id
           AND item.target_id=NEW.target_id
           AND item.exact_asset=NEW.exact_origin
           AND item.technique=expected_technique
           AND item.expected_capability=expected_capability
         FOR SHARE;
        IF root_item_id IS NULL THEN
            RAISE EXCEPTION 'enumeration_lane_commit_axis_root_missing'
                USING ERRCODE='23514';
        END IF;
    END IF;

    IF CARDINALITY(NEW.dependency_receipt_ids)<>(
        SELECT COUNT(DISTINCT receipt_id) FROM unnest(NEW.dependency_receipt_ids) receipt_id
    ) OR NEW.dependency_receipt_ids<>COALESCE((
        SELECT array_agg(receipt_id ORDER BY receipt_id)
          FROM unnest(NEW.dependency_receipt_ids) receipt_id
    ),'{}'::UUID[]) OR CARDINALITY(NEW.evidence_audit_ids)<>(
        SELECT COUNT(DISTINCT audit_id) FROM unnest(NEW.evidence_audit_ids) audit_id
    ) OR NEW.evidence_audit_ids<>(
        SELECT array_agg(audit_id ORDER BY audit_id)
          FROM unnest(NEW.evidence_audit_ids) audit_id
    ) OR CARDINALITY(NEW.candidate_denominator_ids)<>(
        SELECT COUNT(DISTINCT denominator_id)
          FROM unnest(NEW.candidate_denominator_ids) denominator_id
    ) OR NEW.candidate_denominator_ids<>COALESCE((
        SELECT array_agg(denominator_id ORDER BY denominator_id)
          FROM unnest(NEW.candidate_denominator_ids) denominator_id
    ),'{}'::UUID[]) OR CARDINALITY(NEW.parameter_denominator_ids)<>(
        SELECT COUNT(DISTINCT denominator_id)
          FROM unnest(NEW.parameter_denominator_ids) denominator_id
    ) OR NEW.parameter_denominator_ids<>COALESCE((
        SELECT array_agg(denominator_id ORDER BY denominator_id)
          FROM unnest(NEW.parameter_denominator_ids) denominator_id
    ),'{}'::UUID[])
    THEN
        RAISE EXCEPTION 'enumeration_lane_commit_manifest_not_canonical' USING ERRCODE='23514';
    END IF;
    PERFORM 1 FROM enumeration_lane_commit_receipts dependency
     WHERE dependency.id=ANY(NEW.dependency_receipt_ids) FOR SHARE;
    SELECT COUNT(*),COALESCE(array_agg(dependency.lane ORDER BY dependency.lane),'{}'::TEXT[])
      INTO dependency_count,dependency_lane_set
      FROM enumeration_lane_commit_receipts dependency
     WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
       AND dependency.operation_id=NEW.operation_id
       AND dependency.organization_id=NEW.organization_id
       AND dependency.stage_execution_id=NEW.stage_execution_id
       AND dependency.stage_run_unit_id=NEW.stage_run_unit_id
       AND dependency.target_id=NEW.target_id
       AND dependency.exact_origin=NEW.exact_origin;
    IF dependency_count<>CARDINALITY(NEW.dependency_receipt_ids)
       OR (NEW.lane='browser' AND dependency_lane_set<>'{}'::TEXT[])
       OR (NEW.lane='js_api' AND dependency_lane_set<>ARRAY['browser']::TEXT[])
       OR (NEW.lane='parameter' AND dependency_lane_set<>ARRAY['browser','js_api']::TEXT[])
       OR (NEW.lane='resolution' AND (
           dependency_count<>1
           OR dependency_lane_set[1] NOT IN ('browser','js_api')
       ))
       OR (NEW.lane='coverage' AND (
           NOT dependency_lane_set @> ARRAY['browser','js_api','parameter']::TEXT[]
           OR NOT (dependency_lane_set <@ ARRAY['browser','js_api','parameter','resolution']::TEXT[])
           OR (SELECT COUNT(*) FROM unnest(dependency_lane_set) lane WHERE lane='browser')<>1
           OR (SELECT COUNT(*) FROM unnest(dependency_lane_set) lane WHERE lane='js_api')<>1
           OR (SELECT COUNT(*) FROM unnest(dependency_lane_set) lane WHERE lane='parameter')<>1
       )) THEN
        RAISE EXCEPTION 'enumeration_lane_commit_dependency_mismatch' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*) INTO normalized_evidence_count
      FROM tool_truth_evidence_authorities evidence
     WHERE evidence.execution_authority_id=NEW.execution_authority_id
       AND evidence.evidence_audit_id=ANY(NEW.evidence_audit_ids);
    IF normalized_evidence_count<>CARDINALITY(NEW.evidence_audit_ids) THEN
        RAISE EXCEPTION 'enumeration_lane_commit_evidence_mismatch' USING ERRCODE='23514';
    END IF;

    IF NEW.lane IN ('browser','js_api') THEN
        producer_authority_ids := ARRAY[NEW.execution_authority_id];
        producer_candidate_denominator_ids := NEW.candidate_denominator_ids;
    ELSE
        SELECT COALESCE(array_agg(DISTINCT dependency.execution_authority_id
                                  ORDER BY dependency.execution_authority_id),'{}'::UUID[]),
               COALESCE(array_agg(DISTINCT candidate_denominator_id
                                  ORDER BY candidate_denominator_id)
                   FILTER (WHERE candidate_denominator_id IS NOT NULL),'{}'::UUID[])
          INTO producer_authority_ids,producer_candidate_denominator_ids
          FROM enumeration_lane_commit_receipts dependency
          LEFT JOIN LATERAL unnest(dependency.candidate_denominator_ids)
               candidate_denominator_id ON TRUE
         WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
           AND dependency.lane IN ('browser','js_api');
    END IF;

    SELECT COALESCE(array_agg(occurrence.id ORDER BY occurrence.id),'{}'::UUID[]),
           COUNT(*)::BIGINT,
           COUNT(*) FILTER (
               WHERE occurrence.resolution_status IN ('ambiguous','unresolved')
                 AND occurrence.scope_decision='in_scope'
                 AND occurrence.candidate_classification='endpoint'
           )::BIGINT,
           COUNT(*) FILTER (WHERE occurrence.promotion_eligible)::BIGINT
      INTO dependency_occurrence_ids,computed_occurrence_count,
           computed_unresolved_count,computed_promotable_count
      FROM enumeration_endpoint_occurrences occurrence
      JOIN enumeration_endpoint_candidate_inputs candidate
        ON candidate.id=occurrence.candidate_input_id
       AND candidate.execution_authority_id=occurrence.execution_authority_id
     WHERE occurrence.execution_authority_id=ANY(producer_authority_ids)
       AND candidate.denominator_id=ANY(producer_candidate_denominator_ids)
       AND occurrence.source_target_id=NEW.target_id
       AND occurrence.source_web_origin_id=source_origin_id;

    IF NEW.lane IN ('browser','js_api') THEN
        IF NOT EXISTS (
            SELECT 1 FROM coverage_denominators denominator
             WHERE denominator.id=NEW.script_denominator_id
               AND denominator.execution_authority_id=NEW.execution_authority_id
               AND denominator.denominator_kind='derived_child'
               AND denominator.parent_denominator_id=root_denominator_id
               AND denominator.parent_denominator_item_id=root_item_id
               AND denominator.sealed_at IS NOT NULL
             FOR SHARE
        ) OR EXISTS (
            SELECT 1 FROM coverage_denominators denominator
             WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
               AND (
                   denominator.execution_authority_id<>NEW.execution_authority_id
                   OR denominator.denominator_kind<>'derived_child'
                   OR denominator.sealed_at IS NULL
                   OR NOT enumeration_denominator_has_worker_root(
                       denominator.id,NEW.execution_authority_id
                   )
               )
        ) OR (
            SELECT COUNT(*) FROM coverage_denominators denominator
             WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
               AND denominator.execution_authority_id=NEW.execution_authority_id
        )<>CARDINALITY(NEW.candidate_denominator_ids) THEN
            RAISE EXCEPTION 'enumeration_lane_commit_denominator_mismatch'
                USING ERRCODE='23514';
        END IF;
        SELECT COUNT(*)::BIGINT INTO computed_script_count
          FROM enumeration_js_analysis_items descriptor
         WHERE descriptor.execution_authority_id=NEW.execution_authority_id
           AND descriptor.denominator_id=NEW.script_denominator_id
           AND descriptor.terminal_receipt_input_id IS NOT NULL
           AND descriptor.terminal_bound_at IS NOT NULL;
        IF computed_script_count<>(
            SELECT member_count FROM coverage_denominators
             WHERE id=NEW.script_denominator_id
        ) OR EXISTS (
            SELECT 1 FROM enumeration_js_analysis_items descriptor
             WHERE descriptor.execution_authority_id=NEW.execution_authority_id
               AND descriptor.denominator_id<>NEW.script_denominator_id
        ) THEN
            RAISE EXCEPTION 'enumeration_lane_commit_script_exact_set_incomplete'
                USING ERRCODE='23514';
        END IF;
        SELECT COUNT(*)::BIGINT INTO computed_candidate_count
          FROM enumeration_endpoint_candidate_inputs candidate
         WHERE candidate.execution_authority_id=NEW.execution_authority_id
           AND candidate.denominator_id=ANY(NEW.candidate_denominator_ids);
        IF computed_candidate_count<>(
            SELECT COALESCE(SUM(denominator.member_count),0)::BIGINT
              FROM coverage_denominators denominator
             WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
        ) OR EXISTS (
            SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
             WHERE candidate.execution_authority_id=NEW.execution_authority_id
               AND NOT (candidate.denominator_id=ANY(NEW.candidate_denominator_ids))
        ) OR EXISTS (
            SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
            LEFT JOIN enumeration_endpoint_occurrences occurrence
              ON occurrence.candidate_input_id=candidate.id
             AND occurrence.execution_authority_id=candidate.execution_authority_id
             WHERE candidate.execution_authority_id=NEW.execution_authority_id
               AND candidate.denominator_id=ANY(NEW.candidate_denominator_ids)
             GROUP BY candidate.id
            HAVING COUNT(occurrence.id)=0
        ) OR EXISTS (
            SELECT 1 FROM coverage_denominators denominator
            LEFT JOIN enumeration_endpoint_candidate_denominator_closure_receipts closure
              ON closure.denominator_id=denominator.id
             AND closure.execution_authority_id=denominator.execution_authority_id
             WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
               AND denominator.member_count=0 AND closure.id IS NULL
        ) OR EXISTS (
            SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
            JOIN enumeration_endpoint_occurrences occurrence
              ON occurrence.candidate_input_id=candidate.id
             AND occurrence.execution_authority_id=candidate.execution_authority_id
            LEFT JOIN enumeration_endpoint_candidate_closure_receipts closure
              ON closure.candidate_input_id=candidate.id
             WHERE candidate.execution_authority_id=NEW.execution_authority_id
               AND candidate.denominator_id=ANY(NEW.candidate_denominator_ids)
             GROUP BY candidate.id,closure.id
            HAVING BOOL_AND(NOT (
                       occurrence.resolution_status IN ('ambiguous','unresolved')
                       AND occurrence.scope_decision='in_scope'
                   )) AND closure.id IS NULL
        ) THEN
            RAISE EXCEPTION 'enumeration_lane_commit_candidate_exact_set_incomplete'
                USING ERRCODE='23514';
        END IF;
    END IF;

    IF NEW.lane IN ('parameter','coverage') THEN
        SELECT dependency.script_count INTO computed_script_count
          FROM enumeration_lane_commit_receipts dependency
         WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
           AND dependency.lane='browser';
        SELECT COALESCE(SUM(dependency.candidate_count),0)::BIGINT
          INTO computed_candidate_count
          FROM enumeration_lane_commit_receipts dependency
         WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
           AND dependency.lane IN ('browser','js_api');
        SELECT COUNT(DISTINCT assessment.id),COUNT(parameter.id)
          INTO computed_parameter_assessment_count,computed_parameter_fact_count
          FROM enumeration_endpoint_parameter_assessments assessment
          LEFT JOIN enumeration_endpoint_occurrence_parameters parameter
            ON parameter.assessment_id=assessment.id
         WHERE assessment.execution_authority_id=CASE
             WHEN NEW.lane='parameter' THEN NEW.execution_authority_id
             ELSE (SELECT dependency.execution_authority_id
                     FROM enumeration_lane_commit_receipts dependency
                    WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
                      AND dependency.lane='parameter')
         END;
        SELECT COALESCE(array_agg(assessment.occurrence_id ORDER BY assessment.occurrence_id),
                        '{}'::UUID[])
          INTO assessment_occurrence_ids
          FROM enumeration_endpoint_parameter_assessments assessment
         WHERE assessment.execution_authority_id=CASE
             WHEN NEW.lane='parameter' THEN NEW.execution_authority_id
             ELSE (SELECT dependency.execution_authority_id
                     FROM enumeration_lane_commit_receipts dependency
                    WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
                      AND dependency.lane='parameter')
         END;
        SELECT COUNT(DISTINCT link.group_id),COUNT(DISTINCT (link.occurrence_id,link.group_id)),
               COUNT(DISTINCT api.group_id)
          INTO computed_group_count,computed_occurrence_link_count,computed_api_link_count
          FROM enumeration_endpoint_occurrences occurrence
          LEFT JOIN enumeration_endpoint_occurrence_group_links link
            ON link.occurrence_id=occurrence.id
          LEFT JOIN enumeration_endpoint_group_api_links api ON api.group_id=link.group_id
         WHERE occurrence.id=ANY(dependency_occurrence_ids);
    END IF;

    IF NEW.lane='parameter' THEN
        IF assessment_occurrence_ids<>dependency_occurrence_ids
           OR computed_parameter_assessment_count<>computed_occurrence_count
           OR CARDINALITY(NEW.parameter_denominator_ids)<>computed_occurrence_count
           OR EXISTS (
                SELECT 1 FROM coverage_denominators denominator
                 WHERE denominator.id=ANY(NEW.parameter_denominator_ids)
                   AND (
                       denominator.execution_authority_id<>NEW.execution_authority_id
                       OR denominator.denominator_kind<>'derived_child'
                       OR denominator.parent_denominator_id<>root_denominator_id
                       OR denominator.parent_denominator_item_id<>root_item_id
                       OR denominator.sealed_at IS NULL
                       OR denominator.member_count<>1
                   )
           ) OR (
                SELECT COUNT(*) FROM coverage_denominators denominator
                 WHERE denominator.id=ANY(NEW.parameter_denominator_ids)
                   AND denominator.execution_authority_id=NEW.execution_authority_id
           )<>CARDINALITY(NEW.parameter_denominator_ids)
           OR EXISTS (
                SELECT 1 FROM enumeration_endpoint_parameter_assessments assessment
                 WHERE assessment.execution_authority_id=NEW.execution_authority_id
                   AND NOT (assessment.denominator_id=ANY(NEW.parameter_denominator_ids))
           ) THEN
            RAISE EXCEPTION 'enumeration_parameter_exact_occurrence_set_incomplete'
                USING ERRCODE='23514';
        END IF;
    ELSIF NEW.lane='resolution' THEN
        IF NOT (NEW.resolution_occurrence_id=ANY(dependency_occurrence_ids))
           OR NOT EXISTS (
                SELECT 1 FROM enumeration_endpoint_occurrences occurrence
                 WHERE occurrence.id=NEW.resolution_occurrence_id
                   AND occurrence.resolution_status IN ('ambiguous','unresolved')
                   AND occurrence.scope_decision='in_scope'
                   AND occurrence.candidate_classification='endpoint'
                 FOR SHARE
           ) OR NOT EXISTS (
                SELECT 1 FROM capability_execution_receipt_inputs input
                JOIN coverage_denominators denominator
                  ON denominator.id=input.denominator_id
                 AND denominator.execution_authority_id=input.execution_authority_id
                JOIN enumeration_receipt_input_census_seals census
                  ON census.receipt_id=input.receipt_id
                 AND census.denominator_id=input.denominator_id
                 AND census.execution_authority_id=input.execution_authority_id
                 WHERE input.id=NEW.resolution_terminal_receipt_input_id
                   AND input.receipt_id=NEW.resolution_terminal_receipt_id
                   AND input.execution_authority_id=NEW.execution_authority_id
                   AND input.sealed_at IS NOT NULL
                   AND input.attempt_state IN ('failed','outcome_unknown','exhausted')
                   AND input.coverage_gap_reason IN (
                       'tool_failure','budget_exhausted','unsupported',
                       'policy_blocked','source_unavailable'
                   )
                   AND denominator.denominator_kind='derived_child'
                   AND denominator.parent_denominator_id=root_denominator_id
                   AND denominator.parent_denominator_item_id=root_item_id
                   AND denominator.member_count=1
                 FOR SHARE OF input,denominator,census
           ) OR NOT EXISTS (
                SELECT 1 FROM enumeration_endpoint_candidate_closure_receipts closure
                JOIN enumeration_endpoint_occurrences occurrence
                  ON occurrence.candidate_input_id=closure.candidate_input_id
                 WHERE occurrence.id=NEW.resolution_occurrence_id
                   AND closure.resolution_execution_authority_id=NEW.execution_authority_id
                   AND closure.resolution_terminal_receipt_id=NEW.resolution_terminal_receipt_id
                   AND closure.resolution_terminal_receipt_input_id=
                       NEW.resolution_terminal_receipt_input_id
                 FOR SHARE OF closure,occurrence
           ) THEN
            RAISE EXCEPTION 'enumeration_resolution_exact_closeout_incomplete'
                USING ERRCODE='23514';
        END IF;
        computed_script_count := 0;
        computed_candidate_count := 1;
        computed_occurrence_count := 1;
        computed_unresolved_count := 1;
        computed_promotable_count := 0;
    END IF;

    IF NEW.lane='coverage' AND NOT EXISTS (
        SELECT 1 FROM enumeration_lane_commit_receipts parameter
         WHERE parameter.id=ANY(NEW.dependency_receipt_ids) AND parameter.lane='parameter'
           AND parameter.occurrence_count=computed_occurrence_count
           AND parameter.parameter_assessment_count=computed_parameter_assessment_count
           AND parameter.missing=0
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_coverage_parameter_receipt_drift' USING ERRCODE='23514';
    END IF;
    IF NEW.lane='coverage' THEN
        SELECT COALESCE(array_agg(receipt.resolution_occurrence_id
                                  ORDER BY receipt.resolution_occurrence_id),'{}'::UUID[])
          INTO resolution_occurrence_ids
          FROM enumeration_lane_commit_receipts receipt
         WHERE receipt.id=ANY(NEW.dependency_receipt_ids)
           AND receipt.lane='resolution';
        IF resolution_occurrence_ids<>(
            SELECT COALESCE(array_agg(occurrence.id ORDER BY occurrence.id),'{}'::UUID[])
              FROM enumeration_endpoint_occurrences occurrence
             WHERE occurrence.id=ANY(dependency_occurrence_ids)
               AND occurrence.resolution_status IN ('ambiguous','unresolved')
               AND occurrence.scope_decision='in_scope'
               AND occurrence.candidate_classification='endpoint'
        ) OR EXISTS (
            SELECT 1 FROM unnest(producer_candidate_denominator_ids) denominator_id
            LEFT JOIN enumeration_endpoint_candidate_denominator_closure_receipts closure
              ON closure.denominator_id=denominator_id
             WHERE closure.id IS NULL
        ) OR computed_promotable_count<>(
            SELECT COUNT(DISTINCT link.occurrence_id)
              FROM enumeration_endpoint_occurrence_group_links link
             WHERE link.occurrence_id=ANY(dependency_occurrence_ids)
        ) OR (
            SELECT COUNT(*) FROM enumeration_endpoint_occurrences occurrence
             WHERE occurrence.id=ANY(dependency_occurrence_ids)
               AND occurrence.promotion_eligible
               AND occurrence.protocol IN ('http','https')
               AND NOT EXISTS (
                   SELECT 1 FROM enumeration_endpoint_occurrence_group_links link
                   JOIN enumeration_endpoint_group_api_links api ON api.group_id=link.group_id
                    WHERE link.occurrence_id=occurrence.id
               )
        )<>0 THEN
            RAISE EXCEPTION 'enumeration_coverage_exact_receipt_graph_incomplete'
                USING ERRCODE='23514';
        END IF;
    END IF;

    exact_denominator_ids := CASE NEW.lane
        WHEN 'browser' THEN ARRAY[NEW.script_denominator_id]||NEW.candidate_denominator_ids
        WHEN 'js_api' THEN ARRAY[NEW.script_denominator_id]||NEW.candidate_denominator_ids
        WHEN 'parameter' THEN NEW.parameter_denominator_ids
        WHEN 'resolution' THEN ARRAY[(
            SELECT denominator_id FROM capability_execution_receipt_inputs
             WHERE id=NEW.resolution_terminal_receipt_input_id
        )]
        ELSE '{}'::UUID[]
    END;
    SELECT COUNT(*)::BIGINT INTO exact_denominator_count
      FROM coverage_denominators denominator
     WHERE denominator.id=ANY(exact_denominator_ids);
    expected_denominator_count := CARDINALITY(exact_denominator_ids);
    IF exact_denominator_count<>expected_denominator_count THEN
        RAISE EXCEPTION 'enumeration_lane_commit_denominator_set_incomplete'
            USING ERRCODE='23514';
    END IF;

    SELECT COALESCE(jsonb_agg(jsonb_build_object(
               'id',denominator.id,
               'denominator_hash',denominator.denominator_hash,
               'parent_denominator_id',denominator.parent_denominator_id,
               'parent_denominator_item_id',denominator.parent_denominator_item_id,
               'member_count',denominator.member_count,
               'member_set_hash',denominator.member_set_hash,
               'sealed_empty',denominator.sealed_empty,
               'receipt_census',(
                   SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'receipt_id',receipt.id,
                       'receipt_authority_hash',receipt.receipt_authority_hash,
                       'census_id',census.id,
                       'input_count',census.input_count,
                       'input_set_hash',census.input_set_hash
                   ) ORDER BY receipt.id),'[]'::JSONB)
                     FROM capability_execution_receipts receipt
                     LEFT JOIN enumeration_receipt_input_census_seals census
                       ON census.receipt_id=receipt.id
                    WHERE receipt.denominator_id=denominator.id
               )
           ) ORDER BY denominator.id),'[]'::JSONB)
      INTO denominator_material
      FROM coverage_denominators denominator
     WHERE denominator.id=ANY(exact_denominator_ids);

    entity_material := jsonb_build_object(
        'descriptors',(
            SELECT COALESCE(jsonb_agg(to_jsonb(descriptor)-ARRAY['created_at','terminal_bound_at']
                                      ORDER BY descriptor.id),'[]'::JSONB)
              FROM enumeration_js_analysis_items descriptor
             WHERE descriptor.execution_authority_id=ANY(producer_authority_ids)
               AND (NEW.lane NOT IN ('browser','js_api')
                    OR descriptor.denominator_id=NEW.script_denominator_id)
        ),
        'candidates',(
            SELECT COALESCE(jsonb_agg(to_jsonb(candidate)-'created_at'
                                      ORDER BY candidate.id),'[]'::JSONB)
              FROM enumeration_endpoint_candidate_inputs candidate
             WHERE candidate.denominator_id=ANY(producer_candidate_denominator_ids)
               AND candidate.execution_authority_id=ANY(producer_authority_ids)
        ),
        'occurrences',(
            SELECT COALESCE(jsonb_agg(to_jsonb(occurrence)-ARRAY['created_at']
                                      ORDER BY occurrence.id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrences occurrence
             WHERE occurrence.id=ANY(CASE WHEN NEW.lane='resolution'
                                          THEN ARRAY[NEW.resolution_occurrence_id]
                                          ELSE dependency_occurrence_ids END)
        ),
        'occurrence_evidence',(
            SELECT COALESCE(jsonb_agg(to_jsonb(evidence)-'created_at'
                                      ORDER BY evidence.occurrence_id,evidence.evidence_role,
                                               evidence.tool_truth_evidence_authority_id),
                            '[]'::JSONB)
              FROM enumeration_endpoint_occurrence_evidence evidence
             WHERE evidence.occurrence_id=ANY(CASE WHEN NEW.lane='resolution'
                                                   THEN ARRAY[NEW.resolution_occurrence_id]
                                                   ELSE dependency_occurrence_ids END)
               AND (NEW.lane NOT IN ('browser','js_api')
                    OR evidence.occurrence_execution_authority_id=NEW.execution_authority_id)
        ),
        'assessments',(
            SELECT COALESCE(jsonb_agg(to_jsonb(assessment)-'created_at'
                                      ORDER BY assessment.id),'[]'::JSONB)
              FROM enumeration_endpoint_parameter_assessments assessment
             WHERE assessment.occurrence_id=ANY(dependency_occurrence_ids)
               AND (NEW.lane NOT IN ('parameter','coverage') OR
                    assessment.execution_authority_id=CASE
                      WHEN NEW.lane='parameter' THEN NEW.execution_authority_id
                      ELSE (SELECT dependency.execution_authority_id
                              FROM enumeration_lane_commit_receipts dependency
                             WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
                               AND dependency.lane='parameter') END)
        ),
        'parameters',(
            SELECT COALESCE(jsonb_agg(to_jsonb(parameter)-'created_at'
                                      ORDER BY parameter.id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_parameters parameter
              JOIN enumeration_endpoint_parameter_assessments assessment
                ON assessment.id=parameter.assessment_id
             WHERE assessment.occurrence_id=ANY(dependency_occurrence_ids)
        ),
        'groups',(
            SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'link',to_jsonb(link)-'created_at',
                       'group',to_jsonb(group_row)-'created_at',
                       'api',to_jsonb(api)-'created_at'
                   ) ORDER BY link.occurrence_id,link.group_id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_group_links link
              JOIN enumeration_endpoint_groups group_row ON group_row.id=link.group_id
              LEFT JOIN enumeration_endpoint_group_api_links api ON api.group_id=link.group_id
             WHERE link.occurrence_id=ANY(dependency_occurrence_ids)
        ),
        'resolution_terminal_input',(
            SELECT to_jsonb(input)-ARRAY['created_at','sealed_at']
              FROM capability_execution_receipt_inputs input
             WHERE input.id=NEW.resolution_terminal_receipt_input_id
        ),
        'resolution_suggestions',(
            SELECT COALESCE(jsonb_agg(to_jsonb(suggestion)-'created_at'
                                      ORDER BY suggestion.id),'[]'::JSONB)
              FROM enumeration_js_resolution_suggestions suggestion
             WHERE suggestion.parent_occurrence_id=NEW.resolution_occurrence_id
        )
    );

    NEW.entity_set_sha256 := tool_truth_sha256(entity_material::TEXT);
    NEW.denominator_set_sha256 := tool_truth_sha256(jsonb_build_object(
        'root_denominator_id',root_denominator_id,
        'root_denominator_hash',root_denominator_hash,
        'root_item_id',root_item_id,
        'root_item_hash',root_item_hash,
        'denominators',denominator_material,
        'dependency_denominator_hashes',(
            SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'id',dependency.id,
                       'denominator_set_sha256',dependency.denominator_set_sha256
                   ) ORDER BY dependency.id),'[]'::JSONB)
              FROM enumeration_lane_commit_receipts dependency
             WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
        )
    )::TEXT);
    NEW.script_count := computed_script_count;
    NEW.candidate_count := computed_candidate_count;
    NEW.occurrence_count := computed_occurrence_count;
    NEW.parameter_assessment_count := computed_parameter_assessment_count;
    NEW.parameter_fact_count := computed_parameter_fact_count;
    NEW.unresolved_count := computed_unresolved_count;
    NEW.group_count := computed_group_count;
    NEW.occurrence_link_count := computed_occurrence_link_count;
    NEW.api_link_count := computed_api_link_count;
    NEW.missing := 0;
    NEW.terminal_disposition := CASE
        WHEN NEW.lane='resolution' OR computed_unresolved_count>0 THEN 'terminal_with_residual'
        WHEN NEW.lane='browser' AND computed_script_count=0
             AND computed_occurrence_count=0 THEN 'checked_empty'
        WHEN NEW.lane='js_api' AND computed_promotable_count=0 THEN 'checked_empty'
        WHEN NEW.lane='parameter' AND computed_parameter_fact_count=0 THEN 'checked_empty'
        WHEN NEW.lane='coverage' AND computed_script_count=0
             AND computed_occurrence_count=0 THEN 'checked_empty'
        ELSE 'found'
    END;
    NEW.receipt_set_sha256 := tool_truth_sha256(jsonb_build_object(
        'api_link_count',NEW.api_link_count,
        'api_link_count',NEW.api_link_count,
        'artifact_sha256',NEW.artifact_sha256,
        'candidate_count',NEW.candidate_count,
        'dependency_receipt_ids',NEW.dependency_receipt_ids,
        'dependency_receipt_hashes',(
            SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'id',dependency.id,
                       'receipt_set_sha256',dependency.receipt_set_sha256,
                       'entity_set_sha256',dependency.entity_set_sha256,
                       'denominator_set_sha256',dependency.denominator_set_sha256
                   ) ORDER BY dependency.id),'[]'::JSONB)
              FROM enumeration_lane_commit_receipts dependency
             WHERE dependency.id=ANY(NEW.dependency_receipt_ids)
        ),
        'denominator_set_sha256',NEW.denominator_set_sha256,
        'entity_set_sha256',NEW.entity_set_sha256,
        'evidence_audit_ids',NEW.evidence_audit_ids,
        'execution_authority_id',NEW.execution_authority_id,
        'group_count',NEW.group_count,
        'lane',NEW.lane,
        'missing',NEW.missing,
        'occurrence_count',NEW.occurrence_count,
        'occurrence_link_count',NEW.occurrence_link_count,
        'parameter_assessment_count',NEW.parameter_assessment_count,
        'parameter_fact_count',NEW.parameter_fact_count,
        'script_denominator_id',NEW.script_denominator_id,
        'candidate_denominator_ids',NEW.candidate_denominator_ids,
        'parameter_denominator_ids',NEW.parameter_denominator_ids,
        'resolution_occurrence_id',NEW.resolution_occurrence_id,
        'resolution_terminal_receipt_id',NEW.resolution_terminal_receipt_id,
        'resolution_terminal_receipt_input_id',NEW.resolution_terminal_receipt_input_id,
        'script_count',NEW.script_count,
        'target_id',NEW.target_id,
        'terminal_disposition',NEW.terminal_disposition,
        'unresolved_count',NEW.unresolved_count,
        'exact_origin',NEW.exact_origin
    )::TEXT);
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_lane_commit_receipt_validate
BEFORE INSERT ON enumeration_lane_commit_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_lane_commit_receipt();
CREATE TRIGGER enumeration_lane_commit_receipt_immutable
BEFORE UPDATE OR DELETE ON enumeration_lane_commit_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_lane_commit_receipt_immutable'
);
