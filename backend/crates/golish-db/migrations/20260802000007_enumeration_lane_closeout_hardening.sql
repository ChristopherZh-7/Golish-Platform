-- Enumeration v2 closeout hardening.
--
-- This migration is forward-only. It adds the missing typed Resolution
-- closeout authority and rejects lane receipts whose evidence or denominator
-- manifests do not name the exact subject-owned sets.

CREATE TABLE enumeration_resolution_closeout_receipts (
    id UUID PRIMARY KEY,
    stable_closeout_request_id UUID NOT NULL UNIQUE,
    execution_authority_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    stage_kind TEXT NOT NULL DEFAULT 'enumeration' CHECK (stage_kind='enumeration'),
    assigned_work_item_id UUID NOT NULL,
    worker_run_id UUID NOT NULL,
    source_tool_call_id UUID NOT NULL,
    worker_attempt_epoch BIGINT NOT NULL CHECK (worker_attempt_epoch>=0),
    lease_token UUID NOT NULL,
    parent_occurrence_id UUID NOT NULL UNIQUE,
    producer_lane_receipt_id UUID NOT NULL,
    terminal_state TEXT NOT NULL CHECK (
        terminal_state IN ('advisory_residual','budget_exhausted','unsupported')
    ),
    reason_code TEXT NOT NULL CHECK (
        BTRIM(reason_code)<>''
        AND reason_code !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    suggestion_ids UUID[] NOT NULL DEFAULT '{}'::UUID[] CHECK (
        array_position(suggestion_ids,NULL) IS NULL
    ),
    terminal_receipt_id UUID NOT NULL,
    terminal_receipt_input_id UUID NOT NULL,
    evidence_set_sha256 TEXT NOT NULL CHECK (
        evidence_set_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    closeout_sha256 TEXT NOT NULL CHECK (
        closeout_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id,execution_authority_id),
    FOREIGN KEY(execution_authority_id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        REFERENCES tool_truth_execution_authorities(
                id,operation_id,project_scope_id,project_path_at_freeze,
                scope_snapshot_id,organization_id,stage_execution_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,scope_snapshot_id,
                organization_id,stage_kind)
        REFERENCES stage_run_units(
                id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
                stage_kind)
        ON DELETE RESTRICT,
    FOREIGN KEY(assigned_work_item_id,operation_id,stage_execution_id,stage_run_unit_id,
                organization_id)
        REFERENCES stage_work_items(
                id,operation_id,stage_execution_id,stage_run_unit_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(worker_run_id,assigned_work_item_id,operation_id,stage_execution_id,
                stage_run_unit_id,organization_id)
        REFERENCES stage_worker_runs(
                id,work_item_id,operation_id,stage_execution_id,stage_run_unit_id,
                organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(parent_occurrence_id,operation_id,organization_id)
        REFERENCES enumeration_endpoint_occurrences(id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(producer_lane_receipt_id)
        REFERENCES enumeration_lane_commit_receipts(id) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_receipt_input_id,terminal_receipt_id,execution_authority_id)
        REFERENCES capability_execution_receipt_inputs(
                id,receipt_id,execution_authority_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION enumeration_validate_resolution_closeout()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_suggestion_ids UUID[];
DECLARE expected_gap_reason TEXT;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'enumeration-subject:'||NEW.operation_id::TEXT||':'||
        NEW.organization_id::TEXT||':'||NEW.stage_execution_id::TEXT||':'||
        NEW.stage_run_unit_id::TEXT||':'||(
            SELECT receipt.target_id::TEXT
              FROM enumeration_lane_commit_receipts receipt
             WHERE receipt.id=NEW.producer_lane_receipt_id
        )||':'||(
            SELECT receipt.exact_origin
              FROM enumeration_lane_commit_receipts receipt
             WHERE receipt.id=NEW.producer_lane_receipt_id
        ),0
    ));

    IF NOT EXISTS (
        SELECT 1
          FROM tool_truth_execution_authorities authority
          JOIN stage_worker_runs worker
            ON worker.id=NEW.worker_run_id
           AND worker.work_item_id=NEW.assigned_work_item_id
           AND worker.operation_id=NEW.operation_id
           AND worker.stage_execution_id=NEW.stage_execution_id
           AND worker.stage_run_unit_id=NEW.stage_run_unit_id
           AND worker.organization_id=NEW.organization_id
          JOIN stage_work_items item
            ON item.id=worker.work_item_id
           AND item.operation_id=worker.operation_id
           AND item.stage_execution_id=worker.stage_execution_id
           AND item.stage_run_unit_id=worker.stage_run_unit_id
           AND item.organization_id=worker.organization_id
          JOIN stage_worker_requests request
            ON request.accepted_work_item_id=item.id
           AND request.operation_id=item.operation_id
           AND request.stage_execution_id=item.stage_execution_id
           AND request.stage_run_unit_id=item.stage_run_unit_id
           AND request.organization_id=item.organization_id
          JOIN tool_calls call
            ON call.id=NEW.source_tool_call_id
           AND call.worker_run_id=worker.id
           AND call.attempt_epoch=worker.attempt_epoch
           AND call.lease_token=worker.lease_token
         WHERE authority.id=NEW.execution_authority_id
           AND authority.operation_id=NEW.operation_id
           AND authority.project_scope_id=NEW.project_scope_id
           AND authority.project_path_at_freeze=NEW.project_path_at_freeze
           AND authority.scope_snapshot_id=NEW.scope_snapshot_id
           AND authority.organization_id=NEW.organization_id
           AND authority.stage_execution_id=NEW.stage_execution_id
           AND authority.stage_run_unit_id=NEW.stage_run_unit_id
           AND authority.execution_owner_kind='worker_tool'
           AND authority.worker_run_id=worker.id
           AND authority.worker_attempt_epoch=NEW.worker_attempt_epoch
           AND authority.lease_token=NEW.lease_token
           AND authority.source_tool_call_id=call.id
           AND worker.attempt_epoch=NEW.worker_attempt_epoch
           AND worker.lease_token=NEW.lease_token
           AND worker.active_tool_call_id=call.id
           AND worker.status IN ('running','waiting_background')
           AND worker.lease_expires_at>statement_timestamp()
           AND item.kind='enumeration_resolution'
           AND item.role='resolution_analyst'
           AND request.status='accepted'
           AND request.request_kind='enumeration_resolution'
           AND request.requested_role='resolution_analyst'
           AND ((request.reason_code::JSONB->>'objective')::JSONB
                    ->>'unresolved_cluster_id')=NEW.parent_occurrence_id::TEXT
         FOR SHARE OF authority,worker,item,request,call
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_closeout_worker_fence_mismatch'
            USING ERRCODE='23514';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM enumeration_lane_commit_receipts producer
          JOIN enumeration_endpoint_occurrences occurrence
            ON occurrence.id=NEW.parent_occurrence_id
           AND occurrence.execution_authority_id=producer.execution_authority_id
          JOIN web_origins origin
            ON origin.id=occurrence.source_web_origin_id
         WHERE producer.id=NEW.producer_lane_receipt_id
           AND producer.lane IN ('browser','js_api')
           AND producer.operation_id=NEW.operation_id
           AND producer.organization_id=NEW.organization_id
           AND producer.stage_execution_id=NEW.stage_execution_id
           AND producer.stage_run_unit_id=NEW.stage_run_unit_id
           AND producer.target_id=occurrence.source_target_id
           AND producer.exact_origin=origin.origin
           AND occurrence.resolution_status IN ('ambiguous','unresolved')
           AND occurrence.scope_decision='in_scope'
           AND occurrence.candidate_classification='endpoint'
         FOR SHARE OF producer,occurrence,origin
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_closeout_producer_mismatch'
            USING ERRCODE='23514';
    END IF;

    expected_gap_reason := CASE NEW.terminal_state
        WHEN 'budget_exhausted' THEN 'budget_exhausted'
        WHEN 'unsupported' THEN 'unsupported'
        ELSE 'unsupported'
    END;
    IF NOT EXISTS (
        SELECT 1
          FROM capability_execution_receipt_inputs input
          JOIN enumeration_receipt_input_census_seals census
            ON census.receipt_id=input.receipt_id
           AND census.denominator_id=input.denominator_id
           AND census.execution_authority_id=input.execution_authority_id
         WHERE input.id=NEW.terminal_receipt_input_id
           AND input.receipt_id=NEW.terminal_receipt_id
           AND input.execution_authority_id=NEW.execution_authority_id
           AND input.sealed_at IS NOT NULL
           AND input.attempt_state IN ('failed','outcome_unknown','exhausted')
           AND input.coverage_gap_reason=expected_gap_reason
         FOR SHARE OF input,census
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_closeout_terminal_input_mismatch'
            USING ERRCODE='23514';
    END IF;

    SELECT COALESCE(array_agg(suggestion.id ORDER BY suggestion.id),'{}'::UUID[])
      INTO expected_suggestion_ids
      FROM enumeration_js_resolution_suggestions suggestion
     WHERE suggestion.assigned_work_item_id=NEW.assigned_work_item_id
       AND suggestion.assigned_cluster_id=NEW.parent_occurrence_id
       AND suggestion.parent_occurrence_id=NEW.parent_occurrence_id
       AND suggestion.worker_run_id=NEW.worker_run_id;
    IF NEW.suggestion_ids<>COALESCE((
           SELECT array_agg(id ORDER BY id) FROM unnest(NEW.suggestion_ids) id
       ),'{}'::UUID[])
       OR NEW.suggestion_ids<>expected_suggestion_ids
       OR (NEW.terminal_state='advisory_residual'
           AND CARDINALITY(NEW.suggestion_ids)=0) THEN
        RAISE EXCEPTION 'enumeration_resolution_closeout_suggestion_set_mismatch'
            USING ERRCODE='23514';
    END IF;

    NEW.evidence_set_sha256 := tool_truth_sha256(jsonb_build_object(
        'execution_authority_id',NEW.execution_authority_id,
        'evidence',(
            SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'id',evidence.id,
                       'evidence_audit_id',evidence.evidence_audit_id,
                       'authority_hash',evidence.authority_hash
                   ) ORDER BY evidence.id),'[]'::JSONB)
              FROM tool_truth_evidence_authorities evidence
             WHERE evidence.execution_authority_id=NEW.execution_authority_id
        )
    )::TEXT);
    NEW.closeout_sha256 := tool_truth_sha256(jsonb_build_object(
        'execution_authority_id',NEW.execution_authority_id,
        'parent_occurrence_id',NEW.parent_occurrence_id,
        'producer_lane_receipt_id',NEW.producer_lane_receipt_id,
        'terminal_state',NEW.terminal_state,
        'reason_code',NEW.reason_code,
        'suggestion_ids',NEW.suggestion_ids,
        'terminal_receipt_id',NEW.terminal_receipt_id,
        'terminal_receipt_input_id',NEW.terminal_receipt_input_id,
        'evidence_set_sha256',NEW.evidence_set_sha256
    )::TEXT);
    RETURN NEW;
END;
$$;

CREATE TRIGGER enumeration_resolution_closeout_validate
BEFORE INSERT ON enumeration_resolution_closeout_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_resolution_closeout();

CREATE TRIGGER enumeration_resolution_closeout_immutable
BEFORE UPDATE OR DELETE ON enumeration_resolution_closeout_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_resolution_closeout_immutable'
);

CREATE FUNCTION enumeration_validate_lane_receipt_hardening()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE root_denominator_id UUID;
DECLARE root_item_id UUID;
DECLARE actual_evidence_ids BIGINT[];
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'enumeration-subject:'||NEW.operation_id::TEXT||':'||
        NEW.organization_id::TEXT||':'||NEW.stage_execution_id::TEXT||':'||
        NEW.stage_run_unit_id::TEXT||':'||NEW.target_id::TEXT||':'||NEW.exact_origin,0
    ));

    SELECT root.worker_root_denominator_id,item.id
      INTO root_denominator_id,root_item_id
      FROM enumeration_worker_authority_roots root
      JOIN coverage_denominator_items item
        ON item.denominator_id=root.worker_root_denominator_id
       AND item.execution_authority_id=root.worker_execution_authority_id
       AND item.target_id=NEW.target_id
       AND item.exact_asset=NEW.exact_origin
       AND item.technique=CASE NEW.lane
           WHEN 'browser' THEN 'GOLISH-ENUM-JS'
           WHEN 'parameter' THEN 'GOLISH-ENUM-PARAM'
           ELSE 'GOLISH-ENUM-JSAPI'
       END
     WHERE root.worker_execution_authority_id=NEW.execution_authority_id
     FOR SHARE OF root,item;

    SELECT COALESCE(array_agg(evidence.evidence_audit_id
                              ORDER BY evidence.evidence_audit_id),'{}'::BIGINT[])
      INTO actual_evidence_ids
      FROM tool_truth_evidence_authorities evidence
     WHERE evidence.execution_authority_id=NEW.execution_authority_id;
    IF NEW.evidence_audit_ids<>actual_evidence_ids THEN
        RAISE EXCEPTION 'enumeration_lane_commit_evidence_exact_set_mismatch'
            USING ERRCODE='23514';
    END IF;

    IF NEW.lane='browser' AND EXISTS (
        SELECT 1 FROM coverage_denominators denominator
         WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
           AND NOT (
               denominator.execution_authority_id=NEW.execution_authority_id
               AND denominator.parent_denominator_id=root_denominator_id
               AND denominator.parent_denominator_item_id=root_item_id
           )
    ) THEN
        RAISE EXCEPTION 'enumeration_browser_candidate_denominator_origin_mismatch'
            USING ERRCODE='23514';
    ELSIF NEW.lane='js_api' AND EXISTS (
        SELECT 1 FROM coverage_denominators denominator
         WHERE denominator.id=ANY(NEW.candidate_denominator_ids)
           AND NOT (
               denominator.execution_authority_id=NEW.execution_authority_id
               AND (
                   (
                       denominator.parent_denominator_id=NEW.script_denominator_id
                       AND EXISTS (
                           SELECT 1 FROM coverage_denominator_items script_item
                            WHERE script_item.id=denominator.parent_denominator_item_id
                              AND script_item.denominator_id=NEW.script_denominator_id
                              AND script_item.execution_authority_id=NEW.execution_authority_id
                       )
                   )
                   OR (
                       denominator.parent_denominator_id=root_denominator_id
                       AND denominator.parent_denominator_item_id=root_item_id
                       AND (SELECT member_count FROM coverage_denominators
                             WHERE id=NEW.script_denominator_id)=0
                   )
               )
           )
    ) THEN
        RAISE EXCEPTION 'enumeration_js_api_candidate_denominator_origin_mismatch'
            USING ERRCODE='23514';
    END IF;

    IF NEW.lane='resolution' AND NOT EXISTS (
        SELECT 1 FROM enumeration_resolution_closeout_receipts closeout
         WHERE closeout.execution_authority_id=NEW.execution_authority_id
           AND closeout.parent_occurrence_id=NEW.resolution_occurrence_id
           AND closeout.producer_lane_receipt_id=NEW.dependency_receipt_ids[1]
           AND closeout.terminal_receipt_id=NEW.resolution_terminal_receipt_id
           AND closeout.terminal_receipt_input_id=
               NEW.resolution_terminal_receipt_input_id
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_typed_closeout_required'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER enumeration_lane_commit_receipt_hardening
BEFORE INSERT ON enumeration_lane_commit_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_lane_receipt_hardening();

CREATE FUNCTION enumeration_reject_authority_write_after_lane_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE row_value JSONB;
DECLARE owner_authority_id UUID;
BEGIN
    row_value := CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    owner_authority_id := (row_value->>TG_ARGV[0])::UUID;
    IF owner_authority_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM enumeration_lane_commit_receipts receipt
         WHERE receipt.execution_authority_id=owner_authority_id
    ) THEN
        RAISE EXCEPTION 'enumeration_lane_entity_write_after_seal'
            USING ERRCODE='23514';
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$;

CREATE TRIGGER enumeration_js_analysis_items_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_js_analysis_items
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_candidate_inputs_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_candidate_inputs
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_candidate_events_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_candidate_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_occurrences_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_parameter_assessments_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_parameter_assessments
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_occurrence_parameters_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_occurrence_parameters
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'assessment_execution_authority_id'
);
CREATE TRIGGER enumeration_receipt_census_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_receipt_input_census_seals
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);

CREATE FUNCTION enumeration_reject_suggestion_after_resolution_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM enumeration_lane_commit_receipts receipt
         WHERE receipt.lane='resolution'
           AND receipt.resolution_occurrence_id=NEW.parent_occurrence_id
    ) THEN
        RAISE EXCEPTION 'enumeration_resolution_suggestion_after_closeout'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_resolution_suggestion_lane_freeze
BEFORE INSERT ON enumeration_js_resolution_suggestions
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_suggestion_after_resolution_seal();
