-- Enumeration v2 subject/DAG hardening.
--
-- A base lane is a single immutable fact set for one frozen exact Origin.
-- Resolution remains one receipt per immutable unresolved occurrence.  This
-- prevents a crashed/new worker from publishing a second Browser/JsApi/
-- Parameter/Coverage truth set for the same subject.

CREATE UNIQUE INDEX enumeration_lane_commit_receipts_one_base_subject
    ON enumeration_lane_commit_receipts(
        operation_id,organization_id,stage_execution_id,stage_run_unit_id,
        target_id,exact_origin,lane
    )
    WHERE lane IN ('browser','js_api','parameter','coverage');

CREATE FUNCTION enumeration_validate_lane_receipt_exact_dag()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE browser_id UUID;
DECLARE js_api_id UUID;
DECLARE parameter_id UUID;
DECLARE expected_pair UUID[];
BEGIN
    SELECT receipt.id INTO browser_id
      FROM enumeration_lane_commit_receipts receipt
     WHERE receipt.id=ANY(NEW.dependency_receipt_ids)
       AND receipt.lane='browser';
    SELECT receipt.id INTO js_api_id
      FROM enumeration_lane_commit_receipts receipt
     WHERE receipt.id=ANY(NEW.dependency_receipt_ids)
       AND receipt.lane='js_api';
    SELECT receipt.id INTO parameter_id
      FROM enumeration_lane_commit_receipts receipt
     WHERE receipt.id=ANY(NEW.dependency_receipt_ids)
       AND receipt.lane='parameter';

    IF NEW.lane='js_api' AND (
        browser_id IS NULL
        OR NEW.dependency_receipt_ids<>ARRAY[browser_id]
    ) THEN
        RAISE EXCEPTION 'enumeration_js_api_exact_browser_dependency_mismatch'
            USING ERRCODE='23514';
    END IF;

    IF NEW.lane IN ('parameter','coverage') THEN
        IF browser_id IS NULL OR js_api_id IS NULL OR NOT EXISTS (
            SELECT 1 FROM enumeration_lane_commit_receipts js_api
             WHERE js_api.id=js_api_id
               AND js_api.dependency_receipt_ids=ARRAY[browser_id]
             FOR SHARE
        ) THEN
            RAISE EXCEPTION 'enumeration_parameter_exact_producer_dag_mismatch'
                USING ERRCODE='23514';
        END IF;
        expected_pair := ARRAY(
            SELECT id FROM unnest(ARRAY[browser_id,js_api_id]) id ORDER BY id
        );
    END IF;

    IF NEW.lane='parameter'
       AND NEW.dependency_receipt_ids<>expected_pair THEN
        RAISE EXCEPTION 'enumeration_parameter_exact_dependency_set_mismatch'
            USING ERRCODE='23514';
    END IF;

    IF NEW.lane='coverage' AND (
        parameter_id IS NULL OR NOT EXISTS (
            SELECT 1 FROM enumeration_lane_commit_receipts parameter
             WHERE parameter.id=parameter_id
               AND parameter.dependency_receipt_ids=expected_pair
             FOR SHARE
        )
    ) THEN
        RAISE EXCEPTION 'enumeration_coverage_exact_parameter_dag_mismatch'
            USING ERRCODE='23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER enumeration_lane_commit_receipt_exact_dag
BEFORE INSERT ON enumeration_lane_commit_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_lane_receipt_exact_dag();
