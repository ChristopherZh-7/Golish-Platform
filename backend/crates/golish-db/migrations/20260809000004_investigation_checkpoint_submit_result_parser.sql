-- Accept the two canonical submit_result transport shapes emitted by the
-- agent runtime: a JSON object, or a JSON string containing that object.
-- Malformed strings, duplicate submit_result calls, non-object payloads and
-- non-v1 payloads remain fail closed.

CREATE FUNCTION unified_investigation_submit_result_v1(checkpoint JSONB)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
DECLARE
    matches JSONB[];
    raw_result JSONB;
    parsed_result JSONB;
BEGIN
    IF jsonb_typeof(checkpoint)<>'array' THEN
        RETURN NULL;
    END IF;

    SELECT array_agg(result_row.value)
      INTO matches
      FROM jsonb_path_query(
               checkpoint,
               'strict $.** ? (@.name == "submit_result")'
           ) AS result_row(value);
    IF COALESCE(cardinality(matches),0)<>1 THEN
        RETURN NULL;
    END IF;

    raw_result := matches[1] #> '{arguments,result}';
    IF jsonb_typeof(raw_result)='object' THEN
        parsed_result := raw_result;
    ELSIF jsonb_typeof(raw_result)='string' THEN
        BEGIN
            parsed_result := (raw_result #>> '{}')::JSONB;
        EXCEPTION WHEN invalid_text_representation THEN
            RETURN NULL;
        END;
    ELSE
        RETURN NULL;
    END IF;

    IF jsonb_typeof(parsed_result)<>'object'
       OR parsed_result->'schema_version'<>'1'::JSONB
    THEN
        RETURN NULL;
    END IF;
    RETURN parsed_result;
END;
$$;

-- 20260809000002 and 20260809000003 are already installed in retained entity
-- databases. Preserve their migration checksums and replace only the audited
-- object-only predicate in each installed function definition.
DO $$
DECLARE
    function_name TEXT;
    checkpoint_expression TEXT;
    definition TEXT;
    object_only_predicate TEXT;
    typed_predicate TEXT;
BEGIN
    FOR function_name,checkpoint_expression IN
        VALUES
          ('unified_investigation_post_synthesis_analysis_rearm_allowed',
           'recovery_worker.checkpoint'),
          ('unified_investigation_primary_post_synthesis_analysis_rearm_allowed',
           'primary_worker.checkpoint')
    LOOP
        SELECT pg_get_functiondef(function_name::REGPROC)
          INTO STRICT definition;
        object_only_predicate := format(
$predicate$AND jsonb_path_exists(
                    %s,
                    'strict $.** ? (@.name == "submit_result" && @.arguments.result.schema_version == 1)'
                  )$predicate$,
            checkpoint_expression
        );
        typed_predicate := format(
            'AND unified_investigation_submit_result_v1(%s) IS NOT NULL',
            checkpoint_expression
        );
        IF strpos(definition,object_only_predicate)=0 THEN
            RAISE EXCEPTION
                'INVESTIGATION_CHECKPOINT_PARSER_MIGRATION_SOURCE_DRIFT: %',
                function_name;
        END IF;
        definition := replace(
            definition,
            object_only_predicate,
            typed_predicate
        );
        IF strpos(definition,object_only_predicate)<>0
           OR strpos(definition,typed_predicate)=0
        THEN
            RAISE EXCEPTION
                'INVESTIGATION_CHECKPOINT_PARSER_MIGRATION_REWRITE_FAILED: %',
                function_name;
        END IF;
        EXECUTE definition;
    END LOOP;
END;
$$;
