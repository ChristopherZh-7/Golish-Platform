-- Candidate attack-execution V2 shadow-sampling enablement.
--
-- This migration runs before the retained shadow-attestation schema.  It may
-- therefore perform only the one transition that requires no prior sample:
-- legacy -> dual_write_read_legacy.  Later ranks are promoted by the
-- shadow/cohort authority installed after the attestation schema; advancing
-- them here would bypass the production gate.  Existing operation_state rows
-- remain untouched and keep the immutable attack contract they froze at
-- creation.

DO $cutover$
DECLARE
    affected_rows BIGINT;
BEGIN
    UPDATE attack_execution_rollout
       SET contract = 'dual_write_read_legacy',
           rank = 1,
           row_version = row_version + 1,
           updated_at = NOW()
     WHERE singleton = TRUE
       AND contract = 'legacy'
       AND rank = 0
       AND row_version = 0;
    GET DIAGNOSTICS affected_rows = ROW_COUNT;
    IF affected_rows <> 1 THEN
        RAISE EXCEPTION
            'attack execution cutover legacy step expected one singleton row, updated %',
            affected_rows;
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM attack_execution_rollout
         WHERE singleton = TRUE
           AND contract = 'dual_write_read_legacy'
           AND rank = 1
           AND row_version = 1
    ) THEN
        RAISE EXCEPTION 'attack execution sampling enablement did not reach dual_write_read_legacy';
    END IF;
END;
$cutover$;
