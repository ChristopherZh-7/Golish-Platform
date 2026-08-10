-- The historical Registry writer and the unified Investigation compiler use
-- different domain-separated exact-set envelopes for the same immutable
-- hypothesis generation membership.  Closure must validate the exact member
-- rows against the envelope that was actually sealed; it must not mistake the
-- unified compiler's valid seal for an incomplete admission set.

CREATE FUNCTION unified_investigation_generation_member_seal_matches_v1(
    p_generation_id UUID,
    p_expected_member_count BIGINT,
    p_expected_member_set_hash TEXT
)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
STRICT
AS $$
    WITH actual AS (
        SELECT COUNT(*) AS member_count,
               investigation_exact_member_set_hash(
                   'hypothesis_generation_members.v1',
                   COALESCE(array_agg(member.member_hash),ARRAY[]::TEXT[])
               ) AS registry_member_set_hash,
               unified_investigation_exact_set_hash(
                   'hypothesis_generation_members.v1',
                   COALESCE(array_agg(member.member_hash ORDER BY member.member_hash),
                            ARRAY[]::TEXT[])
               ) AS unified_member_set_hash
          FROM hypothesis_generation_members member
         WHERE member.generation_id=p_generation_id
    )
    SELECT p_expected_member_count=actual.member_count
       AND p_expected_member_set_hash IN (
               actual.registry_member_set_hash,
               actual.unified_member_set_hash
           )
      FROM actual
$$;

-- Keep the full closure implementation in its original migration as the
-- single readable source.  This guarded forward patch replaces exactly one
-- obsolete predicate in the already-installed function and aborts migration
-- if the expected body is not present exactly once.
DO $migration$
DECLARE
    closure_definition TEXT;
    obsolete_predicate CONSTANT TEXT := $obsolete$
            OR generation_seal.member_count<>(
                SELECT COUNT(*) FROM hypothesis_generation_members member
                 WHERE member.generation_id=current.generation_id
            )
            OR generation_seal.member_set_hash<>(
                SELECT investigation_exact_member_set_hash(
                           'hypothesis_generation_members.v1',
                           COALESCE(array_agg(member.member_hash ORDER BY member.ordinal),ARRAY[]::TEXT[])
                       )
                  FROM hypothesis_generation_members member
                 WHERE member.generation_id=current.generation_id
            )$obsolete$;
    compatible_predicate CONSTANT TEXT := $compatible$
            OR NOT unified_investigation_generation_member_seal_matches_v1(
                       current.generation_id,
                       generation_seal.member_count,
                       generation_seal.member_set_hash
                   )$compatible$;
BEGIN
    SELECT pg_get_functiondef(
               'seal_investigation_run_closure_v1(uuid,uuid,uuid,text)'::regprocedure
           )
      INTO STRICT closure_definition;

    IF POSITION(obsolete_predicate IN closure_definition)=0
       OR POSITION(
              obsolete_predicate IN SUBSTRING(
                  closure_definition
                  FROM POSITION(obsolete_predicate IN closure_definition)
                       + LENGTH(obsolete_predicate)
              )
          )<>0
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_GENERATION_SEAL_PREDICATE_DRIFT'
            USING ERRCODE='23514';
    END IF;

    EXECUTE REPLACE(
        closure_definition,
        obsolete_predicate,
        compatible_predicate
    );
END
$migration$;
