-- One endpoint candidate may have several immutable occurrences. Each
-- unresolved occurrence owns its own bounded Resolution receipt, while the
-- producer-owned candidate closure is sealed only after every sibling has a
-- typed closeout. Requiring each Resolution receipt to point at the candidate
-- closure's single legacy representative input deadlocked the first sibling
-- and made 1:N occurrence sets impossible to close.
--
-- Forward-only repair: keep the legacy representative columns for compatible
-- readers, make the candidate closure prove the all-sibling closeout barrier,
-- and let each Resolution receipt prove its own exact closeout independently.
DO $migration$
DECLARE original_definition TEXT;
DECLARE repaired_definition TEXT;
DECLARE obsolete_clause TEXT := E' OR NOT EXISTS (\n                SELECT 1 FROM enumeration_endpoint_candidate_closure_receipts closure\n                JOIN enumeration_endpoint_occurrences occurrence\n                  ON occurrence.candidate_input_id=closure.candidate_input_id\n                 WHERE occurrence.id=NEW.resolution_occurrence_id\n                   AND closure.resolution_execution_authority_id=NEW.execution_authority_id\n                   AND closure.resolution_terminal_receipt_id=NEW.resolution_terminal_receipt_id\n                   AND closure.resolution_terminal_receipt_input_id=\n                       NEW.resolution_terminal_receipt_input_id\n                 FOR SHARE OF closure,occurrence\n           )';
BEGIN
    SELECT pg_get_functiondef(
        'enumeration_validate_lane_commit_receipt()'::REGPROCEDURE
    ) INTO original_definition;

    IF POSITION(obsolete_clause IN original_definition)=0 THEN
        RAISE EXCEPTION 'enumeration_multi_occurrence_lane_validator_source_drift'
            USING ERRCODE='23514';
    END IF;
    repaired_definition := REPLACE(original_definition,obsolete_clause,'');
    EXECUTE repaired_definition;
END;
$migration$;

DO $migration$
DECLARE original_definition TEXT;
DECLARE repaired_definition TEXT;
DECLARE anchor TEXT := E'           OR NOT EXISTS (\n               SELECT 1 FROM capability_execution_receipt_inputs input\n                WHERE input.id=NEW.resolution_terminal_receipt_input_id';
DECLARE closeout_barrier TEXT := E'           OR EXISTS (\n               SELECT 1\n                 FROM enumeration_endpoint_occurrences sibling\n                WHERE sibling.candidate_input_id=NEW.candidate_input_id\n                  AND sibling.execution_authority_id=NEW.execution_authority_id\n                  AND sibling.resolution_status IN (''ambiguous'',''unresolved'')\n                  AND sibling.scope_decision=''in_scope''\n                  AND sibling.candidate_classification=''endpoint''\n                  AND NOT EXISTS (\n                      SELECT 1\n                        FROM enumeration_resolution_closeout_receipts closeout\n                        JOIN enumeration_lane_commit_receipts producer\n                          ON producer.id=closeout.producer_lane_receipt_id\n                         AND producer.execution_authority_id=sibling.execution_authority_id\n                       WHERE closeout.parent_occurrence_id=sibling.id\n                  )\n           )\n';
BEGIN
    SELECT pg_get_functiondef(
        'enumeration_validate_candidate_closure_receipt()'::REGPROCEDURE
    ) INTO original_definition;

    IF POSITION(anchor IN original_definition)=0
       OR POSITION('enumeration_candidate_resolution_terminal_receipt_required'
                   IN original_definition)=0
       OR POSITION('enumeration_resolution_closeout_receipts closeout'
                   IN original_definition)>0 THEN
        RAISE EXCEPTION 'enumeration_multi_occurrence_candidate_validator_source_drift'
            USING ERRCODE='23514';
    END IF;
    repaired_definition := REPLACE(
        original_definition,
        anchor,
        closeout_barrier || anchor
    );
    EXECUTE repaired_definition;
END;
$migration$;
