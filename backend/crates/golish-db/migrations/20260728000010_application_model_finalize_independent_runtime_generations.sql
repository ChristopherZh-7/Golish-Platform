-- Unit generation identifies a replacement runtime shell. Worker generation
-- identifies the WorkerRun ordinal for one WorkItem. They are independent
-- counters and must not be equated by final-seal or publication validation.

DO $$
DECLARE
    definition TEXT;
    corrected TEXT;
BEGIN
    SELECT pg_get_functiondef(
               'application_model_validate_current_revision()'::REGPROCEDURE
           )
      INTO definition;
    corrected := replace(
        definition,
        'OR worker.worker_generation <> unit.generation',
        ''
    );
    IF corrected = definition THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_GENERATION_CHECK_NOT_FOUND';
    END IF;
    EXECUTE corrected;
END;
$$;
