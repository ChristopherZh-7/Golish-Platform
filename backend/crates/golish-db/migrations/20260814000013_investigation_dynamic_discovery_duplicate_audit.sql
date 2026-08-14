-- Preserve each dynamic resolution's immutable proposal audit even when two
-- independent resolutions produce the same asset-local semantic hypothesis.
-- Duplicate classification belongs to the server-owned consumption step;
-- rejecting the second source row here made that route unreachable.
DO $$
DECLARE
    duplicate_constraint_name TEXT;
    duplicate_constraint_count BIGINT;
BEGIN
    SELECT MIN(candidate.conname::TEXT),COUNT(*)
      INTO duplicate_constraint_name,duplicate_constraint_count
      FROM (
        SELECT constraint_row.conname
          FROM pg_constraint constraint_row
         WHERE constraint_row.conrelid=
                 'investigation_pending_hypothesis_discoveries'::REGCLASS
           AND constraint_row.contype='u'
           AND ARRAY(
                 SELECT attribute.attname::TEXT
                   FROM unnest(constraint_row.conkey) WITH ORDINALITY key(attnum,ordinal)
                   JOIN pg_attribute attribute
                     ON attribute.attrelid=constraint_row.conrelid
                    AND attribute.attnum=key.attnum
                  ORDER BY key.ordinal
               )=ARRAY[
                 'asset_lane_id',
                 'semantic_key_sha256',
                 'structured_claim_sha256'
               ]::TEXT[]
      ) candidate;
    IF duplicate_constraint_count<>1 THEN
        RAISE EXCEPTION
            'INVESTIGATION_PENDING_DISCOVERY_DUPLICATE_CONSTRAINT_CENSUS_MISMATCH';
    END IF;
    EXECUTE format(
        'ALTER TABLE investigation_pending_hypothesis_discoveries DROP CONSTRAINT %I',
        duplicate_constraint_name
    );
END;
$$;

CREATE INDEX investigation_pending_discovery_semantic_audit
    ON investigation_pending_hypothesis_discoveries(
        asset_lane_id,
        semantic_key_sha256,
        created_at,
        discovery_authority_id
    );
