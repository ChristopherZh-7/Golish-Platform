-- Select the accepted full-chain contracts for a genuinely fresh Golish
-- installation. Existing deployments keep their frozen rollout state and use
-- the evidence-gated promotion path; this migration never rewrites an
-- operation or changes defaults after the first operation exists.

CREATE TABLE fresh_install_full_chain_bootstrap_receipts (
    bootstrap_id UUID PRIMARY KEY,
    singleton BOOLEAN NOT NULL UNIQUE CHECK (singleton),
    schema_version TEXT NOT NULL CHECK (schema_version='fresh_install_full_chain_bootstrap.v1'),
    bootstrap_mode TEXT NOT NULL CHECK (bootstrap_mode IN ('selected','verified_existing')),
    source_contracts JSONB NOT NULL CHECK (jsonb_typeof(source_contracts)='object'),
    target_contracts JSONB NOT NULL CHECK (jsonb_typeof(target_contracts)='object'),
    receipt_sha256 TEXT NOT NULL UNIQUE CHECK (receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    applied_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

DO $$
DECLARE
    operation_count BIGINT;
    runtime_row runtime_memory_rollout%ROWTYPE;
    attack_row attack_execution_rollout%ROWTYPE;
    enumeration_row enumeration_analysis_rollout%ROWTYPE;
    tool_row tool_truth_rollout%ROWTYPE;
    investigation_row investigation_rollout%ROWTYPE;
    source_contracts JSONB;
    target_contracts CONSTANT JSONB := jsonb_build_object(
        'runtime_memory_contract','v2_only',
        'runtime_memory_rank',3,
        'runtime_memory_row_version',3,
        'attack_execution_contract','v2_only',
        'attack_execution_rank',3,
        'attack_execution_row_version',3,
        'enumeration_analysis_contract','agent_team_v2',
        'enumeration_analysis_generation',2,
        'tool_truth_contract','receipt_v1',
        'tool_truth_row_version',1,
        'investigation_contract_version','hypothesis_registry_v1',
        'investigation_rollout_mode','new_only',
        'investigation_mode_rank',4,
        'investigation_row_version',1,
        'stage_topology_contract','unified_investigation_v1'
    );
    bootstrap_mode TEXT;
BEGIN
    LOCK TABLE operation_state IN SHARE ROW EXCLUSIVE MODE;
    SELECT COUNT(*) INTO operation_count FROM operation_state;
    IF operation_count<>0 THEN
        RETURN;
    END IF;

    SELECT * INTO STRICT runtime_row
      FROM runtime_memory_rollout WHERE singleton_id=1 FOR UPDATE;
    SELECT * INTO STRICT attack_row
      FROM attack_execution_rollout WHERE singleton=TRUE FOR UPDATE;
    SELECT * INTO STRICT enumeration_row
      FROM enumeration_analysis_rollout WHERE singleton=TRUE FOR UPDATE;
    SELECT * INTO STRICT tool_row
      FROM tool_truth_rollout WHERE singleton=TRUE FOR UPDATE;
    SELECT * INTO STRICT investigation_row
      FROM investigation_rollout WHERE singleton=TRUE FOR UPDATE;

    source_contracts := jsonb_build_object(
        'runtime_memory_contract',runtime_row.contract,
        'runtime_memory_rank',runtime_row.contract_rank,
        'runtime_memory_row_version',runtime_row.row_version,
        'attack_execution_contract',attack_row.contract,
        'attack_execution_rank',attack_row.rank,
        'attack_execution_row_version',attack_row.row_version,
        'enumeration_analysis_contract',enumeration_row.new_operation_contract,
        'enumeration_analysis_generation',enumeration_row.generation,
        'tool_truth_contract',tool_row.new_operation_contract,
        'tool_truth_row_version',tool_row.row_version,
        'investigation_contract_version',investigation_row.contract_version,
        'investigation_rollout_mode',investigation_row.rollout_mode,
        'investigation_mode_rank',investigation_row.mode_rank,
        'investigation_row_version',investigation_row.row_version
    );

    IF source_contracts=target_contracts-'stage_topology_contract' THEN
        bootstrap_mode := 'verified_existing';
    ELSIF source_contracts=jsonb_build_object(
        'runtime_memory_contract','dual_write_legacy_read',
        'runtime_memory_rank',1,
        'runtime_memory_row_version',1,
        'attack_execution_contract','dual_write_read_legacy',
        'attack_execution_rank',1,
        'attack_execution_row_version',1,
        'enumeration_analysis_contract','legacy_v1',
        'enumeration_analysis_generation',0,
        'tool_truth_contract','legacy_v1',
        'tool_truth_row_version',0,
        'investigation_contract_version','legacy_candidate_v1',
        'investigation_rollout_mode','legacy_only',
        'investigation_mode_rank',0,
        'investigation_row_version',0
    ) THEN
        bootstrap_mode := 'selected';

        EXECUTE 'ALTER TABLE runtime_memory_rollout DISABLE TRIGGER runtime_memory_rollout_forward_only';
        EXECUTE 'ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate';
        EXECUTE 'ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt';
        EXECUTE 'ALTER TABLE attack_execution_rollout DISABLE TRIGGER attack_execution_rollout_forward_only';
        EXECUTE 'ALTER TABLE attack_execution_rollout DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt';
        EXECUTE 'ALTER TABLE enumeration_analysis_rollout DISABLE TRIGGER enumeration_analysis_rollout_mutation_guard';
        EXECUTE 'ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard';
        EXECUTE 'ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard';

        UPDATE runtime_memory_rollout
           SET contract='v2_only',contract_rank=3,row_version=3,
               updated_at=statement_timestamp()
         WHERE singleton_id=1 AND contract='dual_write_legacy_read'
           AND contract_rank=1 AND row_version=1;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'FRESH_INSTALL_RUNTIME_DEFAULT_CAS_CHANGED' USING ERRCODE='23514';
        END IF;

        UPDATE attack_execution_rollout
           SET contract='v2_only',rank=3,row_version=3,updated_at=statement_timestamp()
         WHERE singleton=TRUE AND contract='dual_write_read_legacy'
           AND rank=1 AND row_version=1;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'FRESH_INSTALL_ATTACK_DEFAULT_CAS_CHANGED' USING ERRCODE='23514';
        END IF;

        UPDATE enumeration_analysis_rollout
           SET new_operation_contract='agent_team_v2',generation=2,
               updated_at=statement_timestamp()
         WHERE singleton=TRUE AND new_operation_contract='legacy_v1' AND generation=0;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'FRESH_INSTALL_ENUMERATION_DEFAULT_CAS_CHANGED' USING ERRCODE='23514';
        END IF;

        UPDATE tool_truth_rollout
           SET new_operation_contract='receipt_v1',row_version=1,
               updated_at=statement_timestamp()
         WHERE singleton=TRUE AND new_operation_contract='legacy_v1' AND row_version=0;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'FRESH_INSTALL_TOOL_TRUTH_DEFAULT_CAS_CHANGED' USING ERRCODE='23514';
        END IF;

        UPDATE investigation_rollout
           SET contract_version='hypothesis_registry_v1',rollout_mode='new_only',
               mode_rank=4,row_version=1,updated_at=statement_timestamp()
         WHERE singleton=TRUE AND contract_version='legacy_candidate_v1'
           AND rollout_mode='legacy_only' AND mode_rank=0 AND row_version=0;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'FRESH_INSTALL_INVESTIGATION_DEFAULT_CAS_CHANGED' USING ERRCODE='23514';
        END IF;

        EXECUTE 'ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard';
        EXECUTE 'ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard';
        EXECUTE 'ALTER TABLE enumeration_analysis_rollout ENABLE TRIGGER enumeration_analysis_rollout_mutation_guard';
        EXECUTE 'ALTER TABLE attack_execution_rollout ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt';
        EXECUTE 'ALTER TABLE attack_execution_rollout ENABLE TRIGGER attack_execution_rollout_forward_only';
        EXECUTE 'ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt';
        EXECUTE 'ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate';
        EXECUTE 'ALTER TABLE runtime_memory_rollout ENABLE TRIGGER runtime_memory_rollout_forward_only';
    ELSE
        RAISE EXCEPTION 'FRESH_INSTALL_FULL_CHAIN_DEFAULT_SOURCE_DRIFT' USING ERRCODE='23514';
    END IF;

    INSERT INTO fresh_install_full_chain_bootstrap_receipts(
        bootstrap_id,singleton,schema_version,bootstrap_mode,
        source_contracts,target_contracts,receipt_sha256
    ) VALUES(
        '73c8bd4e-73a0-5d89-bd59-bd1791636cab',TRUE,
        'fresh_install_full_chain_bootstrap.v1',bootstrap_mode,
        source_contracts,target_contracts,
        tool_truth_sha256(jsonb_build_object(
            'schema_version','fresh_install_full_chain_bootstrap.v1',
            'bootstrap_mode',bootstrap_mode,
            'source_contracts',source_contracts,
            'target_contracts',target_contracts
        )::TEXT)
    );
END;
$$;

CREATE TRIGGER fresh_install_full_chain_bootstrap_receipts_append_only
BEFORE INSERT OR UPDATE OR DELETE ON fresh_install_full_chain_bootstrap_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

REVOKE INSERT,UPDATE,DELETE ON fresh_install_full_chain_bootstrap_receipts FROM PUBLIC;
