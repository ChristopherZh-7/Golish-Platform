-- Permit the Application Model current-authority trigger to attest a large,
-- already validated relational proposal through a bounded content-hash receipt.
-- Legacy full-payload receipts remain valid for deterministic replay.
DO $migration$
DECLARE
    function_definition TEXT;
    old_guard TEXT := $old_guard$
           OR submission.payload IS DISTINCT FROM jsonb_build_object(
               'stage_id', 'application_understanding',
               'stage_run_id', manifest.stage_execution_id,
               'schema_version', 1,
               'manifest_id', manifest.id,
               'structured_model', revision.structured_model,
               'decisions', revision_material -> 'decisions',
               'items', revision_material -> 'items'
           )
$old_guard$;
    new_guard TEXT := $new_guard$
           OR NOT (
               submission.payload IS NOT DISTINCT FROM jsonb_build_object(
                   'stage_id', 'application_understanding',
                   'stage_run_id', manifest.stage_execution_id,
                   'schema_version', 1,
                   'manifest_id', manifest.id,
                   'structured_model', revision.structured_model,
                   'decisions', revision_material -> 'decisions',
                   'items', revision_material -> 'items'
               )
               OR submission.payload IS NOT DISTINCT FROM jsonb_build_object(
                   'stage_id', 'application_understanding',
                   'stage_run_id', manifest.stage_execution_id,
                   'schema_version', 1,
                   'manifest_id', manifest.id,
                   'authority_kind', 'model',
                   'proposal_material_hash', application_model_sha256_jsonb(
                       jsonb_build_object(
                           'schema_version', 'application_model_proposal_content.v1',
                           'manifest_id', manifest.id,
                           'structured_model', revision.structured_model,
                           'decisions', revision_material -> 'decisions',
                           'items', revision_material -> 'items'
                       )
                   ),
                   'decision_count', jsonb_array_length(revision_material -> 'decisions'),
                   'item_count', jsonb_array_length(revision_material -> 'items')
               )
           )
$new_guard$;
    source_matches INTEGER;
BEGIN
    SELECT pg_get_functiondef(
               'application_model_validate_current_revision()'::REGPROCEDURE
           )
      INTO STRICT function_definition;

    source_matches := (
        length(function_definition) -
        length(replace(function_definition, old_guard, ''))
    ) / length(old_guard);
    IF source_matches <> 1 THEN
        RAISE EXCEPTION
            'APPLICATION_MODEL_COMPACT_SUBMISSION_GUARD_SOURCE_MISMATCH: expected 1, found %',
            source_matches;
    END IF;

    function_definition := replace(function_definition, old_guard, new_guard);
    IF position(new_guard IN function_definition) = 0 THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_COMPACT_SUBMISSION_GUARD_REPLACEMENT_FAILED';
    END IF;
    EXECUTE function_definition;
END;
$migration$;
