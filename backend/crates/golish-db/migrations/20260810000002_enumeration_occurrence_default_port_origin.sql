-- A canonical endpoint URL is value-free and origin-bound, but URL
-- serializers legitimately omit a default :80/:443 port. Keep the exact
-- frozen WebOrigin identity explicit in the authority rows while accepting
-- both equivalent URL spellings at the occurrence boundary.

CREATE OR REPLACE FUNCTION enumeration_validate_occurrence()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE contract TEXT;
DECLARE tool_contract TEXT;
DECLARE owner_kind TEXT;
BEGIN
    SELECT o.enumeration_analysis_contract,o.tool_truth_contract,a.execution_owner_kind
      INTO contract,tool_contract,owner_kind
      FROM operation_state o
      JOIN tool_truth_execution_authorities a
        ON a.id=NEW.execution_authority_id AND a.operation_id=o.operation_id
     WHERE o.operation_id=NEW.operation_id
       AND a.project_scope_id=NEW.project_scope_id
       AND a.project_path_at_freeze=NEW.project_path_at_freeze
       AND a.scope_snapshot_id=NEW.scope_snapshot_id
       AND a.organization_id=NEW.organization_id
       AND a.stage_execution_id=NEW.stage_execution_id
     FOR SHARE OF o,a;
    IF contract IS NULL THEN
        RAISE EXCEPTION 'enumeration_occurrence_authority_mismatch' USING ERRCODE='23514';
    END IF;
    IF contract='legacy_v1' THEN
        RAISE EXCEPTION 'enumeration_v2_writer_disabled' USING ERRCODE='23514';
    END IF;
    IF contract='agent_team_v2' AND tool_contract<>'receipt_v1' THEN
        RAISE EXCEPTION 'enumeration_receipt_v1_required' USING ERRCODE='23514';
    END IF;
    IF contract='agent_team_v2_shadow' AND tool_contract NOT IN ('shadow_v1','receipt_v1') THEN
        RAISE EXCEPTION 'enumeration_shadow_tool_truth_incompatible' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
         WHERE candidate.id=NEW.candidate_input_id
           AND candidate.execution_authority_id=NEW.execution_authority_id
           AND candidate.terminal_receipt_id=NEW.terminal_receipt_id
           AND candidate.terminal_receipt_input_id=NEW.terminal_receipt_input_id
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_candidate_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_candidate_capture_events capture
         WHERE capture.capture_event_id=NEW.initial_capture_event_id
           AND capture.candidate_input_id=NEW.candidate_input_id
           AND capture.execution_authority_id=NEW.execution_authority_id
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_capture_event_mismatch' USING ERRCODE='23514';
    END IF;
    IF owner_kind<>'worker_tool' OR NOT EXISTS (
        SELECT 1 FROM tool_truth_execution_authorities a
        JOIN stage_worker_runs w ON w.id=a.worker_run_id
        JOIN tool_calls t ON t.id=a.source_tool_call_id
        WHERE a.id=NEW.execution_authority_id
          AND w.attempt_epoch=a.worker_attempt_epoch AND w.lease_token=a.lease_token
          AND t.worker_run_id=w.id AND t.attempt_epoch=w.attempt_epoch AND t.lease_token=w.lease_token
          AND enumeration_denominator_has_worker_root(
              (SELECT denominator_id FROM enumeration_endpoint_candidate_inputs
                WHERE id=NEW.candidate_input_id),a.id
          )
        FOR SHARE OF w,t
    ) THEN
        RAISE EXCEPTION 'tool_truth_worker_fence_mismatch' USING ERRCODE='23514';
    END IF;
    IF NOT enumeration_worker_root_has_exact_origin(
        NEW.execution_authority_id,NEW.source_target_id,NEW.source_web_origin_id
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_source_origin_not_in_frozen_root'
            USING ERRCODE='23514';
    END IF;
    IF NEW.scope_decision='scope_excluded' AND NEW.resolved_web_origin_id IS NOT NULL THEN
        RAISE EXCEPTION 'enumeration_scope_excluded_resolved_target_forbidden' USING ERRCODE='23514';
    END IF;
    IF NEW.resolved_web_origin_id IS NOT NULL AND NOT enumeration_worker_root_has_exact_origin(
        NEW.execution_authority_id,NEW.resolved_target_id,NEW.resolved_web_origin_id
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_resolved_origin_not_in_frozen_root'
            USING ERRCODE='23514';
    END IF;
    IF NOT enumeration_url_matches_web_origin(NEW.source_url,NEW.source_web_origin_id) THEN
        RAISE EXCEPTION 'enumeration_occurrence_source_url_origin_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.resolved_web_origin_id IS NOT NULL AND (
        (NEW.canonical_request_url IS NOT NULL AND NOT enumeration_url_matches_web_origin(
            NEW.canonical_request_url,NEW.resolved_web_origin_id
        ))
        OR (NEW.runtime_sample_url IS NOT NULL AND NOT enumeration_url_matches_web_origin(
            NEW.runtime_sample_url,NEW.resolved_web_origin_id
        ))
        OR (NEW.route_template IS NOT NULL AND NEW.route_template ~ '^[A-Za-z][A-Za-z0-9+.-]*://'
            AND NOT enumeration_url_matches_web_origin(
                NEW.route_template,NEW.resolved_web_origin_id
            ))
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_resolved_url_origin_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.canonical_request_url IS NOT NULL AND NOT enumeration_url_matches_web_origin(
        NEW.canonical_request_url,NEW.resolved_web_origin_id
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_canonical_url_not_exact_origin'
            USING ERRCODE='23514';
    END IF;
    IF NEW.parent_occurrence_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_occurrences p
         WHERE p.id=NEW.parent_occurrence_id AND p.operation_id=NEW.operation_id
           AND p.project_scope_id=NEW.project_scope_id AND p.scope_snapshot_id=NEW.scope_snapshot_id
           AND p.organization_id=NEW.organization_id
           AND p.execution_authority_id=NEW.execution_authority_id FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_occurrence_parent_authority_mismatch' USING ERRCODE='23514';
    END IF;
    IF NEW.method<>UPPER(NEW.method) THEN
        RAISE EXCEPTION 'enumeration_occurrence_method_not_canonical' USING ERRCODE='23514';
    END IF;
    NEW.request_schema_hash := tool_truth_sha256(NEW.request_schema::TEXT);
    RETURN NEW;
END;
$$;
