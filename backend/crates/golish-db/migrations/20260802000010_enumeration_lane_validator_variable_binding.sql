-- Keep the existing append-only Enumeration v2 validator intact while repairing
-- two ambiguous identifiers in its stored PL/pgSQL body. Reconstructing the
-- prior definition from pg_get_functiondef avoids copying the large validator
-- into a second source of truth; every replacement is guarded so migration
-- replay fails closed if the predecessor definition ever differs.
--
-- This is a forward-only repair: existing migration files and receipt rows are
-- not rewritten.
DO $migration$
DECLARE original_definition TEXT;
DECLARE repaired_definition TEXT;
BEGIN
    SELECT pg_get_functiondef(
        'enumeration_validate_lane_commit_receipt()'::REGPROCEDURE
    ) INTO original_definition;

    IF POSITION('DECLARE expected_capability TEXT;' IN original_definition)=0
       OR POSITION('expected_capability := CASE NEW.lane' IN original_definition)=0
       OR POSITION(
           'AND item.expected_capability=expected_capability'
           IN original_definition
       )=0
       OR POSITION(
           'FROM unnest(producer_candidate_denominator_ids) denominator_id'
           IN original_definition
       )=0
       OR POSITION(
           'ON closure.denominator_id=denominator_id'
           IN original_definition
       )=0 THEN
        RAISE EXCEPTION 'enumeration_lane_validator_repair_source_drift'
            USING ERRCODE='23514';
    END IF;

    repaired_definition := REPLACE(
        original_definition,
        'DECLARE expected_capability TEXT;',
        'DECLARE lane_expected_capability TEXT;'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'expected_capability := CASE NEW.lane',
        'lane_expected_capability := CASE NEW.lane'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'AND item.expected_capability=expected_capability',
        'AND item.expected_capability=lane_expected_capability'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'FROM unnest(producer_candidate_denominator_ids) denominator_id',
        'FROM unnest(producer_candidate_denominator_ids) AS producer_denominator(denominator_id)'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'ON closure.denominator_id=denominator_id',
        'ON closure.denominator_id=producer_denominator.denominator_id'
    );

    EXECUTE repaired_definition;
END;
$migration$;
