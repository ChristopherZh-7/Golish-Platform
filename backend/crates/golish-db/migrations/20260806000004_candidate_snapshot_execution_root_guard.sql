-- Keep candidate snapshots aligned with the three-root Tool Truth execution
-- bundle. Target Intel remains a separately validated finalized Goal authority.
CREATE OR REPLACE FUNCTION enforce_candidate_snapshot_exact_authority_bundle()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    ordinal_count BIGINT;
    root_family_count BIGINT;
    fresh_count BIGINT;
BEGIN
    SELECT COUNT(*),COUNT(DISTINCT ordinal),COUNT(DISTINCT root_family),
           COUNT(*) FILTER (WHERE member_status='consistent_fresh')
      INTO actual_count,ordinal_count,root_family_count,fresh_count
      FROM candidate_analysis_snapshot_authority_bundle_members
     WHERE snapshot_id=NEW.snapshot_id;
    IF actual_count<>3 OR ordinal_count<>3 OR root_family_count<>3 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_AUTHORITY_BUNDLE_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF NEW.snapshot_status='sealed_ready' AND fresh_count<>3 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_ALL_FRESH_AUTHORITY_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION enforce_candidate_snapshot_exact_authority_bundle() IS
    'Requires the exact three execution-receipt roots copied from the Tool Truth authority bundle.';
