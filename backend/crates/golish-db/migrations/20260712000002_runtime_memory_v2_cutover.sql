-- Runtime-memory V2 dual-write sampling enablement.
--
-- A migration cannot prove live whole-record parity.  It may therefore only
-- perform the transition that enables dual writes while retaining the legacy
-- read authority.  Later adjacent ranks require the retained runtime-memory
-- attestation/cohort gate installed by a post-foundation migration. Existing
-- operation_state rows remain untouched and keep their immutable contract.

DO $cutover$
DECLARE
    affected_rows BIGINT;
BEGIN
    UPDATE runtime_memory_rollout
       SET contract = 'dual_write_legacy_read',
           contract_rank = 1,
           row_version = row_version + 1,
           updated_at = NOW()
     WHERE singleton_id = 1
       AND contract = 'legacy_v1'
       AND contract_rank = 0
       AND row_version = 0;
    GET DIAGNOSTICS affected_rows = ROW_COUNT;
    IF affected_rows <> 1 THEN
        RAISE EXCEPTION
            'runtime memory cutover legacy_v1 step expected one singleton row, updated %',
            affected_rows;
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM runtime_memory_rollout
         WHERE singleton_id = 1
           AND contract = 'dual_write_legacy_read'
           AND contract_rank = 1
           AND row_version = 1
    ) THEN
        RAISE EXCEPTION 'runtime memory sampling enablement did not reach dual_write_legacy_read';
    END IF;
END;
$cutover$;
