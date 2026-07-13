-- Server-authored Verification final handoff authority.
--
-- Candidate V2 Verification has no model-authored StageDeliverable and its
-- organization/verification WorkerRun is an aggregate cursor, not a leased
-- execution worker.  This retained, frozen-identity row is the typed final
-- seal for that deliberate exception.  It is never backed by a synthetic
-- tool call, deliverable submission, or lease.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION verification_canonical_jsonb(input_value JSONB)
RETURNS TEXT AS $$
DECLARE
    value_kind TEXT;
    rendered TEXT;
BEGIN
    value_kind := jsonb_typeof(input_value);
    CASE value_kind
        WHEN 'null' THEN RETURN 'null';
        WHEN 'boolean' THEN RETURN input_value::TEXT;
        WHEN 'number' THEN RETURN input_value::TEXT;
        WHEN 'string' THEN RETURN to_jsonb(input_value #>> '{}')::TEXT;
        WHEN 'array' THEN
            SELECT '[' || COALESCE(
                       STRING_AGG(
                           verification_canonical_jsonb(element.value),
                           ',' ORDER BY element.ordinal
                       ),
                       ''
                   ) || ']'
              INTO rendered
              FROM jsonb_array_elements(input_value)
                   WITH ORDINALITY AS element(value, ordinal);
            RETURN rendered;
        WHEN 'object' THEN
            SELECT '{' || COALESCE(
                       STRING_AGG(
                           to_jsonb(entry.key)::TEXT || ':' ||
                               verification_canonical_jsonb(entry.value),
                           ',' ORDER BY entry.key
                       ),
                       ''
                   ) || '}'
              INTO rendered
              FROM jsonb_each(input_value) AS entry(key, value);
            RETURN rendered;
        ELSE
            RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END CASE;
END;
$$ LANGUAGE plpgsql IMMUTABLE STRICT;

CREATE FUNCTION verification_sha256_jsonb(input_value JSONB)
RETURNS TEXT AS $$
    SELECT ENCODE(
        DIGEST(verification_canonical_jsonb(input_value), 'sha256'),
        'hex'
    );
$$ LANGUAGE sql IMMUTABLE STRICT;

CREATE FUNCTION verification_attempt_terminal_bundle_exact(
    expected_attempt_id UUID,
    expected_operation_id UUID,
    expected_scope_snapshot_id UUID,
    expected_wave_run_id UUID,
    expected_wave_unit_id UUID,
    expected_organization_id UUID
)
RETURNS BOOLEAN AS $$
    SELECT EXISTS (
        SELECT 1
          FROM candidate_attempts AS attempt
          JOIN attack_candidates AS candidate
            ON candidate.terminal_attempt_id = attempt.id
           AND candidate.candidate_id = attempt.candidate_id
           AND candidate.operation_uuid = attempt.operation_id
           AND candidate.scope_snapshot_id = attempt.scope_snapshot_id
           AND candidate.wave_run_id = attempt.wave_run_id
           AND candidate.wave_unit_id = attempt.wave_unit_id
           AND candidate.organization_id = attempt.organization_id
           AND candidate.target_identity_hash = attempt.target_identity_hash
           AND candidate.candidate_plan_hash = attempt.candidate_plan_hash
           AND candidate.disposition = attempt.status
          JOIN attack_candidate_approvals AS approval
            ON approval.id = attempt.approval_id
           AND approval.candidate_id = attempt.candidate_id
           AND approval.operation_id = attempt.operation_id
           AND approval.scope_snapshot_id = attempt.scope_snapshot_id
           AND approval.wave_run_id = attempt.wave_run_id
           AND approval.wave_unit_id = attempt.wave_unit_id
           AND approval.organization_id = attempt.organization_id
           AND approval.target_identity_hash = attempt.target_identity_hash
           AND approval.candidate_plan_hash = attempt.candidate_plan_hash
           AND approval.status <> 'rejected'
          JOIN stage_worker_runs AS worker
            ON worker.id = attempt.stage_worker_run_id
           AND worker.operation_id = attempt.operation_id
           AND worker.organization_id = attempt.organization_id
           AND worker.specialist = 'candidate_verifier'
           AND worker.work_item_kind = 'candidate_attempt'
           AND worker.work_item_key = attempt.id::TEXT
           AND worker.terminal_at IS NOT NULL
           AND worker.lease_token IS NULL
           AND worker.lease_owner IS NULL
           AND worker.lease_acquired_at IS NULL
           AND worker.lease_expires_at IS NULL
           AND worker.heartbeat_at IS NULL
           AND worker.active_tool_call_id IS NULL
           AND worker.active_tool_started_at IS NULL
          JOIN operation_state AS operation
            ON operation.operation_id = attempt.operation_id
          JOIN knowledge_outbox_events AS terminal_event
            ON terminal_event.event_id = uuid_generate_v5(
                   attempt.id,
                   'CandidateAttemptTerminal.v1'
               )
           AND terminal_event.event_name = 'CandidateAttemptTerminal.v1'
           AND terminal_event.schema_version = 1
           AND terminal_event.project_scope_id = operation.project_scope_id
           AND terminal_event.organization_id_at_time = attempt.organization_id
           AND terminal_event.source_operation_id = attempt.operation_id
           AND terminal_event.source_kind = 'candidate_attempt'
           AND terminal_event.source_id_kind = 'uuid'
           AND terminal_event.source_id_value = attempt.id::TEXT
           AND terminal_event.source_stream_key =
                   'candidate-attempt:' || attempt.id::TEXT
           AND terminal_event.source_version = attempt.row_version
           AND terminal_event.occurred_at = attempt.terminal_at
         WHERE attempt.id = expected_attempt_id
           AND attempt.operation_id = expected_operation_id
           AND attempt.scope_snapshot_id = expected_scope_snapshot_id
           AND attempt.wave_run_id = expected_wave_run_id
           AND attempt.wave_unit_id = expected_wave_unit_id
           AND attempt.organization_id = expected_organization_id
           AND attempt.status IN ('verified', 'refuted', 'blocked')
           AND attempt.terminal_at IS NOT NULL
           AND attempt.result_json IS NOT NULL
           AND jsonb_typeof(attempt.result_json) = 'object'
           AND attempt.result_hash =
                   'sha256:' || verification_sha256_jsonb(attempt.result_json)
           AND attempt.result_json ->> 'disposition' = attempt.status
           AND terminal_event.payload #>> '{structured_payload,attempt_id}' =
                   attempt.id::TEXT
           AND terminal_event.payload #>> '{structured_payload,candidate_id}' =
                   attempt.candidate_id::TEXT
           AND terminal_event.payload #>> '{structured_payload,approval_id}' =
                   attempt.approval_id::TEXT
           AND terminal_event.payload #>> '{structured_payload,disposition}' =
                   attempt.status
           AND terminal_event.payload #>> '{structured_payload,candidate_plan_hash}' =
                   attempt.candidate_plan_hash
           AND terminal_event.payload #>> '{structured_payload,result_hash}' =
                   attempt.result_hash
           AND NULLIF(
                   terminal_event.payload #>> '{structured_payload,finding_id}',
                   ''
               )::UUID IS NOT DISTINCT FROM candidate.terminal_finding_id
           AND terminal_event.payload #>> '{structured_payload,target_type_at_time}' =
                   attempt.target_type_at_time
           AND terminal_event.payload #>> '{structured_payload,target_value_at_time}' =
                   attempt.target_value_at_time
           AND terminal_event.payload #>> '{structured_payload,target_identity_hash}' =
                   attempt.target_identity_hash
           AND terminal_event.payload #> '{structured_payload,proof_evidence_ids}' =
                   to_jsonb(COALESCE(
                       ARRAY(
                           SELECT evidence.evidence_id
                             FROM candidate_attempt_evidence AS evidence
                            WHERE evidence.attempt_id = attempt.id
                              AND evidence.role = 'proof'
                            ORDER BY evidence.evidence_id
                       ),
                       '{}'::BIGINT[]
                   ))
           AND terminal_event.payload #> '{structured_payload,refutation_evidence_ids}' =
                   to_jsonb(COALESCE(
                       ARRAY(
                           SELECT evidence.evidence_id
                             FROM candidate_attempt_evidence AS evidence
                            WHERE evidence.attempt_id = attempt.id
                              AND evidence.role = 'refutation'
                            ORDER BY evidence.evidence_id
                       ),
                       '{}'::BIGINT[]
                   ))
           AND terminal_event.payload #> '{structured_payload,blocker_evidence_ids}' =
                   to_jsonb(COALESCE(
                       ARRAY(
                           SELECT evidence.evidence_id
                             FROM candidate_attempt_evidence AS evidence
                            WHERE evidence.attempt_id = attempt.id
                              AND evidence.role = 'blocker'
                            ORDER BY evidence.evidence_id
                       ),
                       '{}'::BIGINT[]
                   ))
           AND terminal_event.payload #> '{structured_payload,evidence_ids}' =
                   to_jsonb(COALESCE(
                       ARRAY(
                           SELECT DISTINCT evidence.evidence_id
                             FROM candidate_attempt_evidence AS evidence
                            WHERE evidence.attempt_id = attempt.id
                            ORDER BY evidence.evidence_id
                       ),
                       '{}'::BIGINT[]
                   ))
           AND COALESCE(
                   attempt.result_json -> 'proof_evidence_ids',
                   '[]'::JSONB
               ) = terminal_event.payload #> '{structured_payload,proof_evidence_ids}'
           AND COALESCE(
                   attempt.result_json -> 'refutation_evidence_ids',
                   '[]'::JSONB
               ) = terminal_event.payload #> '{structured_payload,refutation_evidence_ids}'
           AND COALESCE(
                   attempt.result_json -> 'blocker_evidence_ids',
                   '[]'::JSONB
               ) = terminal_event.payload #> '{structured_payload,blocker_evidence_ids}'
           AND NULLIF(
                   terminal_event.payload #>> '{structured_payload,fact_delta_count}',
                   ''
               )::BIGINT = (
                   SELECT COUNT(*)
                     FROM attack_fact_deltas AS delta
                    WHERE delta.source_attempt_id = attempt.id
                      AND delta.candidate_id = attempt.candidate_id
                      AND delta.operation_id = attempt.operation_id
                      AND delta.scope_snapshot_id = attempt.scope_snapshot_id
                      AND delta.wave_run_id = attempt.wave_run_id
                      AND delta.wave_unit_id = attempt.wave_unit_id
                      AND delta.organization_id = attempt.organization_id
               )
           AND NOT EXISTS (
                   SELECT 1
                     FROM attack_execution_lanes AS lane
                    WHERE lane.stage_worker_run_id = worker.id
               )
           AND NOT EXISTS (
                   SELECT 1
                     FROM (VALUES
                         ('assertion-promoter', 1),
                         ('document-projector', 1),
                         ('embedding-projector', 1),
                         ('graph-projector', 1)
                     ) AS projector(projector_name, projector_schema_version)
                    WHERE NOT EXISTS (
                        SELECT 1
                          FROM knowledge_projection_deliveries AS delivery
                         WHERE delivery.event_id = terminal_event.event_id
                           AND delivery.projector_name = projector.projector_name
                           AND delivery.projector_schema_version =
                                   projector.projector_schema_version
                    )
               )
           AND (
               (
                   terminal_event.payload #>>
                       '{structured_payload,blocker_reason_code}'
                       IS DISTINCT FROM 'max_attempts_total'
                   AND worker.status = 'passed'
                   AND jsonb_typeof(candidate.execution_plan -> 'actions') = 'array'
                   AND jsonb_array_length(candidate.execution_plan -> 'actions') = (
                       SELECT COUNT(*)
                         FROM candidate_attempt_actions AS action
                        WHERE action.attempt_id = attempt.id
                   )
                   AND NOT EXISTS (
                       SELECT 1
                         FROM jsonb_array_elements(
                                  candidate.execution_plan -> 'actions'
                              ) AS planned(value)
                        WHERE NOT EXISTS (
                            SELECT 1
                              FROM candidate_attempt_actions AS action
                             WHERE action.attempt_id = attempt.id
                               AND action.action_ordinal =
                                       (planned.value ->> 'ordinal')::INTEGER
                               AND action.capability_id =
                                       planned.value ->> 'capability_id'
                               AND action.action_kind =
                                       planned.value ->> 'action_kind'
                               AND action.canonical_args =
                                       planned.value -> 'canonical_args'
                               AND action.status IN ('completed', 'failed')
                               AND action.completed_at IS NOT NULL
                        )
                   )
               )
               OR
               (
                   terminal_event.payload #>>
                       '{structured_payload,blocker_reason_code}' =
                       'max_attempts_total'
                   AND attempt.status = 'blocked'
                   AND worker.status = 'failed'
                   AND EXISTS (
                       SELECT 1
                         FROM attack_residual_risks AS residual
                        WHERE residual.id = uuid_generate_v5(
                                  attempt.id,
                                  'max_attempts_total:residual'
                              )
                          AND residual.operation_id = attempt.operation_id
                          AND residual.scope_snapshot_id = attempt.scope_snapshot_id
                          AND residual.wave_run_id = attempt.wave_run_id
                          AND residual.wave_unit_id = attempt.wave_unit_id
                          AND residual.organization_id = attempt.organization_id
                          AND residual.reason_code = 'max_attempts_total'
                          AND NOT EXISTS (
                              SELECT 1
                                FROM attack_residual_risk_evidence AS risk_evidence
                               WHERE risk_evidence.residual_risk_id = residual.id
                                 AND risk_evidence.role = 'residual'
                                 AND NOT EXISTS (
                                     SELECT 1
                                       FROM candidate_attempt_evidence AS attempt_evidence
                                      WHERE attempt_evidence.attempt_id = attempt.id
                                        AND attempt_evidence.evidence_id =
                                                risk_evidence.evidence_id
                                        AND attempt_evidence.role = 'blocker'
                                 )
                          )
                   )
               )
           )
    );
$$ LANGUAGE sql STABLE;

CREATE FUNCTION verification_finding_ref_projection(
    expected_finding_id UUID,
    expected_operation_id UUID,
    expected_scope_snapshot_id UUID,
    expected_wave_run_id UUID,
    expected_wave_unit_id UUID,
    expected_organization_id UUID
)
RETURNS JSONB AS $$
    SELECT jsonb_build_object(
               'key', jsonb_build_object(
                   'kind', 'finding',
                   'finding_id', finding.id
               ),
               'organization_id', lineage.organization_id,
               'source_table', 'findings',
               'source_row_version', finding.row_version,
               'observed_at_unix_micros',
                   (EXTRACT(EPOCH FROM finding.updated_at) * 1000000)::BIGINT,
               'content_sha256', verification_sha256_jsonb(
                   (to_jsonb(finding) - 'target_id') || jsonb_build_object(
                       'finding_lineage_id', lineage.id,
                       'finding_lineage_row_version', lineage.row_version,
                       'canonical_target_snapshot', lineage.canonical_target_snapshot
                   )
               ),
               'evidence_ids', to_jsonb(COALESCE(
                   ARRAY(
                       SELECT evidence.evidence_id
                         FROM candidate_attempt_evidence AS evidence
                        WHERE evidence.attempt_id = lineage.candidate_attempt_id
                          AND evidence.role = 'proof'
                        ORDER BY evidence.evidence_id
                   ),
                   '{}'::BIGINT[]
               ))
           )
      FROM finding_lineage AS lineage
      JOIN findings AS finding ON finding.id = lineage.finding_id
      JOIN attack_candidates AS candidate
        ON candidate.terminal_attempt_id = lineage.candidate_attempt_id
       AND candidate.terminal_finding_id = lineage.finding_id
       AND candidate.candidate_id = lineage.candidate_id
       AND candidate.operation_uuid = lineage.operation_id
       AND candidate.scope_snapshot_id = lineage.scope_snapshot_id
       AND candidate.wave_run_id = lineage.wave_run_id
       AND candidate.wave_unit_id = lineage.wave_unit_id
       AND candidate.organization_id = lineage.organization_id
       AND candidate.target_identity_hash = lineage.target_identity_hash
       AND candidate.candidate_plan_hash = lineage.candidate_plan_hash
       AND candidate.disposition = 'verified'
     WHERE finding.id = expected_finding_id
       AND lineage.operation_id = expected_operation_id
       AND lineage.scope_snapshot_id = expected_scope_snapshot_id
       AND lineage.wave_run_id = expected_wave_run_id
       AND lineage.wave_unit_id = expected_wave_unit_id
       AND lineage.organization_id = expected_organization_id
       AND verification_attempt_terminal_bundle_exact(
           lineage.candidate_attempt_id,
           expected_operation_id,
           expected_scope_snapshot_id,
           expected_wave_run_id,
           expected_wave_unit_id,
           expected_organization_id
       )
       AND EXISTS (
           SELECT 1
             FROM attack_candidate_approvals AS approval
            WHERE approval.candidate_id = candidate.candidate_id
              AND approval.operation_id = candidate.operation_uuid
              AND approval.scope_snapshot_id = candidate.scope_snapshot_id
              AND approval.wave_run_id = candidate.wave_run_id
              AND approval.wave_unit_id = candidate.wave_unit_id
              AND approval.organization_id = candidate.organization_id
              AND approval.status <> 'rejected'
       );
$$ LANGUAGE sql STABLE;

CREATE TABLE verification_stage_handoffs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    wave_run_id UUID NOT NULL,
    wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    source_stage_run_unit_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL UNIQUE,
    wave_generation INTEGER NOT NULL CHECK (wave_generation >= 0),
    wave_unit_row_version_after_close BIGINT NOT NULL CHECK (
        wave_unit_row_version_after_close > 0
    ),
    from_stage_kind TEXT NOT NULL DEFAULT 'verification' CHECK (
        from_stage_kind = 'verification'
    ),
    authority_kind TEXT NOT NULL DEFAULT 'verification_wave_close' CHECK (
        authority_kind = 'verification_wave_close'
    ),
    payload JSONB NOT NULL CHECK (
        jsonb_typeof(payload) = 'object' AND PG_COLUMN_SIZE(payload) <= 262144
    ),
    payload_sha256 TEXT NOT NULL CHECK (
        payload_sha256 ~ '^[0-9a-f]{64}$'
    ),
    evidence_ids BIGINT[] NOT NULL DEFAULT '{}',
    coverage_watermark JSONB NOT NULL CHECK (
        jsonb_typeof(coverage_watermark) = 'object'
    ),
    verification_truth_hash TEXT NOT NULL CHECK (
        verification_truth_hash ~ '^[0-9a-f]{64}$'
    ),
    gate_passed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
    UNIQUE (wave_unit_id),
    UNIQUE (stage_execution_id, organization_id),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id) ON DELETE RESTRICT,
    FOREIGN KEY (
        wave_unit_id,
        wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES attack_wave_units(
        id,
        wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_stage_run_unit_id,
        operation_id,
        stage_execution_id,
        organization_id,
        from_stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        primary_worker_run_id,
        operation_id,
        stage_execution_id,
        source_stage_run_unit_id,
        organization_id
    ) REFERENCES stage_worker_runs(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT
);

CREATE FUNCTION reject_verification_stage_handoff_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_IMMUTABLE';
    END IF;
    RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER verification_stage_handoffs_immutable
BEFORE UPDATE OR DELETE ON verification_stage_handoffs
FOR EACH ROW EXECUTE FUNCTION reject_verification_stage_handoff_change();

CREATE FUNCTION reject_verification_handoff_source_evidence_change()
RETURNS trigger AS $$
DECLARE
    old_source_id UUID;
    new_source_id UUID;
    claim_kind TEXT;
    claim_identity_field TEXT;
BEGIN
    IF TG_TABLE_NAME = 'attack_candidate_work_item_evidence' THEN
        claim_kind := 'attack_no_candidate_decision';
        claim_identity_field := 'work_item_id';
        IF TG_OP <> 'INSERT' THEN
            old_source_id := OLD.work_item_id;
        END IF;
        IF TG_OP <> 'DELETE' THEN
            new_source_id := NEW.work_item_id;
        END IF;
    ELSIF TG_TABLE_NAME = 'attack_fact_delta_evidence' THEN
        claim_kind := 'attack_fact_delta_proposal';
        claim_identity_field := 'fact_delta_id';
        IF TG_OP <> 'INSERT' THEN
            old_source_id := OLD.fact_delta_id;
        END IF;
        IF TG_OP <> 'DELETE' THEN
            new_source_id := NEW.fact_delta_id;
        END IF;
    END IF;

    IF EXISTS (
        SELECT 1
          FROM verification_stage_handoffs AS handoff,
               LATERAL jsonb_array_elements(
                   handoff.payload -> 'typed_claims'
               ) AS claim(value)
         WHERE claim.value ->> 'kind' = claim_kind
           AND NULLIF(
                   claim.value -> 'payload' ->> claim_identity_field,
                   ''
               )::UUID IN (old_source_id, new_source_id)
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_SOURCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER verification_work_item_evidence_source_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_work_item_evidence
FOR EACH ROW EXECUTE FUNCTION reject_verification_handoff_source_evidence_change();

CREATE TRIGGER verification_fact_delta_evidence_source_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_fact_delta_evidence
FOR EACH ROW EXECUTE FUNCTION reject_verification_handoff_source_evidence_change();

CREATE FUNCTION reject_verification_handoff_evidence_identity_change()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM verification_stage_handoffs AS handoff
         WHERE OLD.id = ANY(handoff.evidence_ids)
    ) THEN
        IF TG_OP = 'UPDATE'
            AND OLD.target_id IS NOT NULL
            AND NEW.target_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_id)
            AND (to_jsonb(NEW) - 'target_id') = (to_jsonb(OLD) - 'target_id')
        THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_SOURCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER verification_evidence_identity_immutable
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION reject_verification_handoff_evidence_identity_change();

CREATE FUNCTION validate_verification_stage_handoff_projection()
RETURNS trigger AS $$
DECLARE
    payload_evidence_ids BIGINT[];
    projected_evidence_ids BIGINT[];
    claim_record RECORD;
    ref_record RECORD;
    expected_payload JSONB;
    expected_ref JSONB;
    expected_ordered_claims JSONB;
    expected_ordered_refs JSONB;
    expected_coverage JSONB;
    expected_truth_material JSONB;
    manifest_count INTEGER;
    manifest_frozen_at TIMESTAMPTZ;
    work_item_count BIGINT;
    approved_candidate_count BIGINT;
    terminal_attempt_count BIGINT;
    verified_finding_count BIGINT;
    no_candidate_decision_count BIGINT;
    fact_delta_proposal_count BIGINT;
    actual_count BIGINT;
    actual_distinct_count BIGINT;
BEGIN
    -- The final-seal chronology is server authority. Caller-supplied past or
    -- future timestamps are overwritten even for direct SQL inserts.
    NEW.gate_passed_at := NOW();
    IF NEW.id IS DISTINCT FROM uuid_generate_v5(
        NEW.wave_unit_id,
        'verification-stage-handoff:v1'
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    IF jsonb_typeof(NEW.payload) IS DISTINCT FROM 'object'
        OR ARRAY(
            SELECT payload_key
              FROM jsonb_object_keys(NEW.payload) AS payload_keys(payload_key)
             ORDER BY payload_key
        ) IS DISTINCT FROM ARRAY[
            'canonical_fact_refs',
            'coverage_watermark',
            'evidence_ids',
            'schema_version',
            'typed_claims',
            'verification_truth_hash'
        ]::TEXT[]
        OR NEW.payload ->> 'schema_version' IS DISTINCT FROM '1'
        OR jsonb_typeof(NEW.payload -> 'canonical_fact_refs') IS DISTINCT FROM 'array'
        OR jsonb_typeof(NEW.payload -> 'typed_claims') IS DISTINCT FROM 'array'
        OR jsonb_typeof(NEW.payload -> 'coverage_watermark') IS DISTINCT FROM 'object'
        OR jsonb_typeof(NEW.payload -> 'evidence_ids') IS DISTINCT FROM 'array'
        OR NEW.payload ->> 'verification_truth_hash'
            IS DISTINCT FROM NEW.verification_truth_hash
        OR NEW.payload -> 'coverage_watermark'
            IS DISTINCT FROM NEW.coverage_watermark
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END IF;
    IF verification_sha256_jsonb(NEW.payload) IS DISTINCT FROM NEW.payload_sha256 THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_HASH_MISMATCH';
    END IF;

    BEGIN
        SELECT COALESCE(
                   ARRAY_AGG(value::BIGINT ORDER BY ordinal),
                   '{}'::BIGINT[]
               )
          INTO payload_evidence_ids
          FROM jsonb_array_elements_text(NEW.payload -> 'evidence_ids')
               WITH ORDINALITY AS element(value, ordinal);
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END;
    IF payload_evidence_ids IS DISTINCT FROM NEW.evidence_ids
        OR NEW.evidence_ids IS DISTINCT FROM ARRAY(
            SELECT DISTINCT evidence_id
              FROM UNNEST(NEW.evidence_ids) AS evidence(evidence_id)
             ORDER BY evidence_id
        )
        OR EXISTS (
            SELECT 1 FROM UNNEST(NEW.evidence_ids) AS evidence(evidence_id)
             WHERE evidence_id <= 0
        )
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
         WHERE jsonb_typeof(value) IS DISTINCT FROM 'object'
            OR ARRAY(
                SELECT wrapper_key
                  FROM jsonb_object_keys(value) AS wrapper_keys(wrapper_key)
                 ORDER BY wrapper_key
            ) IS DISTINCT FROM ARRAY['kind', 'payload']::TEXT[]
            OR value ->> 'kind' NOT IN (
                'candidate_attempt_terminal',
                'verified_candidate_attempt',
                'attack_fact_delta_proposal',
                'attack_no_candidate_decision'
            )
            OR jsonb_typeof(value -> 'payload') IS DISTINCT FROM 'object'
            OR jsonb_typeof(value -> 'payload' -> 'evidence_ids')
                IS DISTINCT FROM 'array'
    ) OR EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'canonical_fact_refs') AS ref(value)
         WHERE jsonb_typeof(value) IS DISTINCT FROM 'object'
            OR value #>> '{key,kind}' IS DISTINCT FROM 'finding'
            OR value ->> 'source_table' IS DISTINCT FROM 'findings'
            OR COALESCE(value ->> 'content_sha256', '')
                !~ '^[0-9a-f]{64}$'
            OR jsonb_typeof(value -> 'evidence_ids') IS DISTINCT FROM 'array'
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
         WHERE ARRAY(
                   SELECT payload_key
                     FROM jsonb_object_keys(value -> 'payload')
                          AS payload_keys(payload_key)
                    ORDER BY payload_key
               ) IS DISTINCT FROM CASE value ->> 'kind'
                   WHEN 'candidate_attempt_terminal' THEN ARRAY[
                       'attempt_id',
                       'blocker_reason_code',
                       'candidate_id',
                       'candidate_plan_hash',
                       'disposition',
                       'evidence_ids',
                       'finding_id',
                       'finding_ref'
                   ]::TEXT[]
                   WHEN 'verified_candidate_attempt' THEN ARRAY[
                       'attempt_id',
                       'blocker_reason_code',
                       'candidate_id',
                       'candidate_plan_hash',
                       'disposition',
                       'evidence_ids',
                       'finding_id',
                       'finding_ref'
                   ]::TEXT[]
                   WHEN 'attack_no_candidate_decision' THEN ARRAY[
                       'decided_at_unix_micros',
                       'detail',
                       'evidence_ids',
                       'reason_code',
                       'work_item_id',
                       'work_item_key'
                   ]::TEXT[]
                   WHEN 'attack_fact_delta_proposal' THEN ARRAY[
                       'candidate_id',
                       'canonical_ref_hash',
                       'canonical_ref_id',
                       'canonical_ref_kind',
                       'canonical_ref_version',
                       'delta_kind',
                       'evidence_ids',
                       'fact_delta_id',
                       'source_attempt_id',
                       'status'
                   ]::TEXT[]
               END
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END IF;

    SELECT COALESCE(
               jsonb_agg(
                   value ORDER BY group_ordinal, identity_id, kind_ordinal
               ),
               '[]'::JSONB
           )
      INTO expected_ordered_claims
      FROM (
          SELECT claim.value,
                 CASE claim.value ->> 'kind'
                     WHEN 'candidate_attempt_terminal' THEN 0
                     WHEN 'verified_candidate_attempt' THEN 0
                     WHEN 'attack_no_candidate_decision' THEN 1
                     WHEN 'attack_fact_delta_proposal' THEN 2
                 END AS group_ordinal,
                 CASE claim.value ->> 'kind'
                     WHEN 'candidate_attempt_terminal' THEN
                         (claim.value #>> '{payload,candidate_id}')::UUID
                     WHEN 'verified_candidate_attempt' THEN
                         (claim.value #>> '{payload,candidate_id}')::UUID
                     WHEN 'attack_no_candidate_decision' THEN
                         (claim.value #>> '{payload,work_item_id}')::UUID
                     WHEN 'attack_fact_delta_proposal' THEN
                         (claim.value #>> '{payload,fact_delta_id}')::UUID
                 END AS identity_id,
                 CASE claim.value ->> 'kind'
                     WHEN 'candidate_attempt_terminal' THEN 0
                     WHEN 'verified_candidate_attempt' THEN 1
                     ELSE 0
                 END AS kind_ordinal
            FROM jsonb_array_elements(
                     NEW.payload -> 'typed_claims'
                 ) AS claim(value)
      ) AS ordered_claim;
    SELECT COALESCE(
               jsonb_agg(
                   value ORDER BY (value #>> '{key,finding_id}')::UUID
               ),
               '[]'::JSONB
           )
      INTO expected_ordered_refs
      FROM jsonb_array_elements(
               NEW.payload -> 'canonical_fact_refs'
           ) AS ref(value);
    IF NEW.payload -> 'typed_claims' IS DISTINCT FROM expected_ordered_claims
        OR NEW.payload -> 'canonical_fact_refs' IS DISTINCT FROM expected_ordered_refs
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;

    BEGIN
        SELECT COALESCE(
                   ARRAY_AGG(DISTINCT evidence_id ORDER BY evidence_id),
                   '{}'::BIGINT[]
               )
          INTO projected_evidence_ids
          FROM (
              SELECT jsonb_array_elements_text(
                         claim.value -> 'payload' -> 'evidence_ids'
                     )::BIGINT AS evidence_id
                FROM jsonb_array_elements(
                         NEW.payload -> 'typed_claims'
                     ) AS claim(value)
              UNION
              SELECT jsonb_array_elements_text(
                         ref.value -> 'evidence_ids'
                     )::BIGINT AS evidence_id
                FROM jsonb_array_elements(
                         NEW.payload -> 'canonical_fact_refs'
                     ) AS ref(value)
          ) AS projected;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID';
    END;
    IF projected_evidence_ids IS DISTINCT FROM NEW.evidence_ids THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH';
    END IF;

    SELECT wave_unit.manifest_count, wave_unit.manifest_frozen_at
      INTO manifest_count, manifest_frozen_at
      FROM attack_wave_units AS wave_unit
     WHERE wave_unit.id = NEW.wave_unit_id
       AND wave_unit.wave_run_id = NEW.wave_run_id
       AND wave_unit.operation_id = NEW.operation_id
       AND wave_unit.scope_snapshot_id = NEW.scope_snapshot_id
       AND wave_unit.organization_id = NEW.organization_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*)
      INTO work_item_count
      FROM attack_candidate_work_items AS work
     WHERE work.operation_id = NEW.operation_id
       AND work.scope_snapshot_id = NEW.scope_snapshot_id
       AND work.wave_unit_id = NEW.wave_unit_id
       AND work.organization_id = NEW.organization_id;
    IF manifest_count IS NULL
        OR manifest_frozen_at IS NULL
        OR manifest_count::BIGINT <> work_item_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM attack_candidate_work_items AS work
         WHERE work.operation_id = NEW.operation_id
           AND work.scope_snapshot_id = NEW.scope_snapshot_id
           AND work.wave_unit_id = NEW.wave_unit_id
           AND work.organization_id = NEW.organization_id
           AND (
               work.decision_kind IS NULL
               OR (
                   work.decision_kind = 'no_candidate'
                   AND NOT EXISTS (
                       SELECT 1
                         FROM attack_candidate_work_item_evidence AS evidence
                        WHERE evidence.work_item_id = work.id
                          AND evidence.role = 'decision'
                   )
               )
               OR (
                   work.decision_kind = 'candidate'
                   AND NOT EXISTS (
                       SELECT 1
                         FROM attack_candidates AS candidate
                         JOIN candidate_attempts AS attempt
                           ON attempt.id = candidate.terminal_attempt_id
                          AND attempt.candidate_id = candidate.candidate_id
                          AND attempt.operation_id = candidate.operation_uuid
                          AND attempt.scope_snapshot_id = candidate.scope_snapshot_id
                          AND attempt.wave_run_id = candidate.wave_run_id
                          AND attempt.wave_unit_id = candidate.wave_unit_id
                          AND attempt.organization_id = candidate.organization_id
                          AND attempt.candidate_plan_hash = candidate.candidate_plan_hash
                        WHERE candidate.source_work_item_id = work.id
                          AND candidate.candidate_id = work.candidate_id
                          AND candidate.operation_uuid = NEW.operation_id
                          AND candidate.scope_snapshot_id = NEW.scope_snapshot_id
                          AND candidate.wave_run_id = NEW.wave_run_id
                          AND candidate.wave_unit_id = NEW.wave_unit_id
                          AND candidate.organization_id = NEW.organization_id
                          AND candidate.disposition IN ('verified', 'refuted', 'blocked')
                          AND attempt.status = candidate.disposition
                          AND attempt.terminal_at IS NOT NULL
                          AND verification_attempt_terminal_bundle_exact(
                              attempt.id,
                              NEW.operation_id,
                              NEW.scope_snapshot_id,
                              NEW.wave_run_id,
                              NEW.wave_unit_id,
                              NEW.organization_id
                          )
                          AND EXISTS (
                              SELECT 1
                                FROM attack_candidate_approvals AS approval
                               WHERE approval.candidate_id = candidate.candidate_id
                                 AND approval.operation_id = NEW.operation_id
                                 AND approval.scope_snapshot_id = NEW.scope_snapshot_id
                                 AND approval.wave_run_id = NEW.wave_run_id
                                 AND approval.wave_unit_id = NEW.wave_unit_id
                                 AND approval.organization_id = NEW.organization_id
                                 AND approval.status <> 'rejected'
                          )
                   )
               )
           )
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;

    SELECT COUNT(DISTINCT approval.candidate_id)
      INTO approved_candidate_count
      FROM attack_candidate_approvals AS approval
     WHERE approval.operation_id = NEW.operation_id
       AND approval.scope_snapshot_id = NEW.scope_snapshot_id
       AND approval.wave_run_id = NEW.wave_run_id
       AND approval.wave_unit_id = NEW.wave_unit_id
       AND approval.organization_id = NEW.organization_id
       AND approval.status <> 'rejected';
    SELECT COUNT(*),
           COUNT(*) FILTER (WHERE candidate.disposition = 'verified')
      INTO terminal_attempt_count, verified_finding_count
      FROM attack_candidates AS candidate
      JOIN candidate_attempts AS attempt
        ON attempt.id = candidate.terminal_attempt_id
       AND attempt.candidate_id = candidate.candidate_id
       AND attempt.operation_id = candidate.operation_uuid
       AND attempt.scope_snapshot_id = candidate.scope_snapshot_id
       AND attempt.wave_run_id = candidate.wave_run_id
       AND attempt.wave_unit_id = candidate.wave_unit_id
       AND attempt.organization_id = candidate.organization_id
       AND attempt.candidate_plan_hash = candidate.candidate_plan_hash
       AND attempt.status = candidate.disposition
       AND attempt.terminal_at IS NOT NULL
     WHERE candidate.operation_uuid = NEW.operation_id
       AND candidate.scope_snapshot_id = NEW.scope_snapshot_id
       AND candidate.wave_run_id = NEW.wave_run_id
       AND candidate.wave_unit_id = NEW.wave_unit_id
       AND candidate.organization_id = NEW.organization_id
       AND candidate.disposition IN ('verified', 'refuted', 'blocked')
       AND verification_attempt_terminal_bundle_exact(
           attempt.id,
           NEW.operation_id,
           NEW.scope_snapshot_id,
           NEW.wave_run_id,
           NEW.wave_unit_id,
           NEW.organization_id
       )
       AND EXISTS (
           SELECT 1
             FROM attack_candidate_approvals AS approval
            WHERE approval.candidate_id = candidate.candidate_id
              AND approval.operation_id = NEW.operation_id
              AND approval.scope_snapshot_id = NEW.scope_snapshot_id
              AND approval.wave_run_id = NEW.wave_run_id
              AND approval.wave_unit_id = NEW.wave_unit_id
              AND approval.organization_id = NEW.organization_id
              AND approval.status <> 'rejected'
       );
    IF approved_candidate_count <> terminal_attempt_count THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*)
      INTO no_candidate_decision_count
      FROM attack_candidate_work_items AS work
     WHERE work.operation_id = NEW.operation_id
       AND work.scope_snapshot_id = NEW.scope_snapshot_id
       AND work.wave_unit_id = NEW.wave_unit_id
       AND work.organization_id = NEW.organization_id
       AND work.decision_kind = 'no_candidate';
    IF EXISTS (
        SELECT 1
          FROM attack_fact_deltas AS delta
         WHERE delta.operation_id = NEW.operation_id
           AND delta.scope_snapshot_id = NEW.scope_snapshot_id
           AND delta.wave_run_id = NEW.wave_run_id
           AND delta.wave_unit_id = NEW.wave_unit_id
           AND delta.organization_id = NEW.organization_id
           AND delta.status <> 'proposed'
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*)
      INTO fact_delta_proposal_count
      FROM attack_fact_deltas AS delta
     WHERE delta.operation_id = NEW.operation_id
       AND delta.scope_snapshot_id = NEW.scope_snapshot_id
       AND delta.wave_run_id = NEW.wave_run_id
       AND delta.wave_unit_id = NEW.wave_unit_id
       AND delta.organization_id = NEW.organization_id
       AND delta.status = 'proposed';

    FOR claim_record IN
        SELECT value ->> 'kind' AS kind, value -> 'payload' AS payload
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
    LOOP
        expected_payload := NULL;
        IF claim_record.kind IN (
            'candidate_attempt_terminal', 'verified_candidate_attempt'
        ) THEN
            SELECT jsonb_build_object(
                       'attempt_id', attempt.id,
                       'candidate_id', candidate.candidate_id,
                       'candidate_plan_hash', attempt.candidate_plan_hash,
                       'disposition', attempt.status,
                       'finding_id', candidate.terminal_finding_id,
                       'blocker_reason_code', NULLIF(
                           BTRIM(attempt.result_json ->> 'blocker_reason_code'),
                           ''
                       ),
                       'finding_ref', CASE
                           WHEN attempt.status = 'verified' THEN
                               verification_finding_ref_projection(
                                   candidate.terminal_finding_id,
                                   NEW.operation_id,
                                   NEW.scope_snapshot_id,
                                   NEW.wave_run_id,
                                   NEW.wave_unit_id,
                                   NEW.organization_id
                               )
                           ELSE 'null'::JSONB
                       END,
                       'evidence_ids', to_jsonb(COALESCE(
                           ARRAY(
                               SELECT DISTINCT evidence.evidence_id
                                 FROM candidate_attempt_evidence AS evidence
                                WHERE evidence.attempt_id = attempt.id
                                  AND evidence.role IN (
                                      'proof', 'refutation', 'blocker'
                                  )
                                ORDER BY evidence.evidence_id
                           ),
                           '{}'::BIGINT[]
                       ))
                   )
              INTO expected_payload
              FROM attack_candidates AS candidate
              JOIN candidate_attempts AS attempt
                ON attempt.id = candidate.terminal_attempt_id
               AND attempt.candidate_id = candidate.candidate_id
               AND attempt.operation_id = candidate.operation_uuid
               AND attempt.scope_snapshot_id = candidate.scope_snapshot_id
               AND attempt.wave_run_id = candidate.wave_run_id
               AND attempt.wave_unit_id = candidate.wave_unit_id
               AND attempt.organization_id = candidate.organization_id
               AND attempt.candidate_plan_hash = candidate.candidate_plan_hash
             WHERE attempt.id = (claim_record.payload ->> 'attempt_id')::UUID
               AND candidate.operation_uuid = NEW.operation_id
               AND candidate.scope_snapshot_id = NEW.scope_snapshot_id
               AND candidate.wave_run_id = NEW.wave_run_id
               AND candidate.wave_unit_id = NEW.wave_unit_id
               AND candidate.organization_id = NEW.organization_id
               AND candidate.disposition IN ('verified', 'refuted', 'blocked')
               AND attempt.status = candidate.disposition
               AND attempt.terminal_at IS NOT NULL
               AND verification_attempt_terminal_bundle_exact(
                   attempt.id,
                   NEW.operation_id,
                   NEW.scope_snapshot_id,
                   NEW.wave_run_id,
                   NEW.wave_unit_id,
                   NEW.organization_id
               )
               AND (
                   claim_record.kind = 'candidate_attempt_terminal'
                   OR attempt.status = 'verified'
               )
               AND EXISTS (
                   SELECT 1
                     FROM attack_candidate_approvals AS approval
                    WHERE approval.candidate_id = candidate.candidate_id
                      AND approval.operation_id = NEW.operation_id
                      AND approval.scope_snapshot_id = NEW.scope_snapshot_id
                      AND approval.wave_run_id = NEW.wave_run_id
                      AND approval.wave_unit_id = NEW.wave_unit_id
                      AND approval.organization_id = NEW.organization_id
                      AND approval.status <> 'rejected'
               );
        ELSIF claim_record.kind = 'attack_no_candidate_decision' THEN
            SELECT jsonb_build_object(
                       'work_item_id', work.id,
                       'work_item_key', work.work_item_key,
                       'reason_code', work.no_candidate_reason_code,
                       'detail', work.no_candidate_detail,
                       'decided_at_unix_micros',
                           (EXTRACT(EPOCH FROM work.decided_at) * 1000000)::BIGINT,
                       'evidence_ids', to_jsonb(COALESCE(
                           ARRAY(
                               SELECT evidence.evidence_id
                                 FROM attack_candidate_work_item_evidence AS evidence
                                WHERE evidence.work_item_id = work.id
                                  AND evidence.role = 'decision'
                                ORDER BY evidence.evidence_id
                           ),
                           '{}'::BIGINT[]
                       ))
                   )
              INTO expected_payload
              FROM attack_candidate_work_items AS work
             WHERE work.id = (claim_record.payload ->> 'work_item_id')::UUID
               AND work.operation_id = NEW.operation_id
               AND work.scope_snapshot_id = NEW.scope_snapshot_id
               AND work.wave_unit_id = NEW.wave_unit_id
               AND work.organization_id = NEW.organization_id
               AND work.decision_kind = 'no_candidate';
        ELSIF claim_record.kind = 'attack_fact_delta_proposal' THEN
            SELECT jsonb_build_object(
                       'fact_delta_id', delta.id,
                       'source_attempt_id', delta.source_attempt_id,
                       'candidate_id', delta.candidate_id,
                       'canonical_ref_kind', delta.canonical_ref_kind,
                       'canonical_ref_id', delta.canonical_ref_id,
                       'canonical_ref_version', delta.canonical_ref_version,
                       'canonical_ref_hash', delta.canonical_ref_hash,
                       'delta_kind', delta.delta_kind,
                       'status', delta.status,
                       'evidence_ids', to_jsonb(COALESCE(
                           ARRAY(
                               SELECT evidence.evidence_id
                                 FROM attack_fact_delta_evidence AS evidence
                                WHERE evidence.fact_delta_id = delta.id
                                ORDER BY evidence.evidence_id
                           ),
                           '{}'::BIGINT[]
                       ))
                   )
              INTO expected_payload
              FROM attack_fact_deltas AS delta
             WHERE delta.id = (claim_record.payload ->> 'fact_delta_id')::UUID
               AND delta.operation_id = NEW.operation_id
               AND delta.scope_snapshot_id = NEW.scope_snapshot_id
               AND delta.wave_run_id = NEW.wave_run_id
               AND delta.wave_unit_id = NEW.wave_unit_id
               AND delta.organization_id = NEW.organization_id
               AND delta.status = 'proposed'
               AND verification_attempt_terminal_bundle_exact(
                   delta.source_attempt_id,
                   NEW.operation_id,
                   NEW.scope_snapshot_id,
                   NEW.wave_run_id,
                   NEW.wave_unit_id,
                   NEW.organization_id
               )
               AND EXISTS (
                   SELECT 1
                     FROM attack_fact_delta_evidence AS evidence
                    WHERE evidence.fact_delta_id = delta.id
               )
               AND NOT EXISTS (
                   SELECT 1
                     FROM attack_fact_delta_evidence AS evidence
                    WHERE evidence.fact_delta_id = delta.id
                      AND NOT EXISTS (
                          SELECT 1
                            FROM candidate_attempt_evidence AS attempt_evidence
                           WHERE attempt_evidence.attempt_id = delta.source_attempt_id
                             AND attempt_evidence.evidence_id = evidence.evidence_id
                             AND attempt_evidence.role = 'fact_delta'
                      )
               );
        END IF;
        IF expected_payload IS NULL
            OR expected_payload IS DISTINCT FROM claim_record.payload
        THEN
            RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
        END IF;
    END LOOP;

    SELECT COUNT(*), COUNT(DISTINCT value #>> '{payload,attempt_id}')
      INTO actual_count, actual_distinct_count
      FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
     WHERE value ->> 'kind' = 'candidate_attempt_terminal';
    IF actual_count <> terminal_attempt_count
        OR actual_distinct_count <> terminal_attempt_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*), COUNT(DISTINCT value #>> '{payload,attempt_id}')
      INTO actual_count, actual_distinct_count
      FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
     WHERE value ->> 'kind' = 'verified_candidate_attempt';
    IF actual_count <> verified_finding_count
        OR actual_distinct_count <> verified_finding_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*), COUNT(DISTINCT value #>> '{payload,work_item_id}')
      INTO actual_count, actual_distinct_count
      FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
     WHERE value ->> 'kind' = 'attack_no_candidate_decision';
    IF actual_count <> no_candidate_decision_count
        OR actual_distinct_count <> no_candidate_decision_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;
    SELECT COUNT(*), COUNT(DISTINCT value #>> '{payload,fact_delta_id}')
      INTO actual_count, actual_distinct_count
      FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
     WHERE value ->> 'kind' = 'attack_fact_delta_proposal';
    IF actual_count <> fact_delta_proposal_count
        OR actual_distinct_count <> fact_delta_proposal_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;

    FOR ref_record IN
        SELECT value
          FROM jsonb_array_elements(NEW.payload -> 'canonical_fact_refs') AS ref(value)
    LOOP
        expected_ref := verification_finding_ref_projection(
            (ref_record.value #>> '{key,finding_id}')::UUID,
            NEW.operation_id,
            NEW.scope_snapshot_id,
            NEW.wave_run_id,
            NEW.wave_unit_id,
            NEW.organization_id
        );
        IF expected_ref IS NULL OR expected_ref IS DISTINCT FROM ref_record.value THEN
            RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
        END IF;
    END LOOP;
    SELECT COUNT(*), COUNT(DISTINCT value #>> '{key,finding_id}')
      INTO actual_count, actual_distinct_count
      FROM jsonb_array_elements(NEW.payload -> 'canonical_fact_refs') AS ref(value);
    IF actual_count <> verified_finding_count
        OR actual_distinct_count <> verified_finding_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;

    expected_coverage := jsonb_build_object(
        'approved_candidate_count', approved_candidate_count,
        'terminal_attempt_count', terminal_attempt_count,
        'verified_finding_count', verified_finding_count,
        'no_candidate_decision_count', no_candidate_decision_count,
        'fact_delta_proposal_count', fact_delta_proposal_count
    );
    IF expected_coverage IS DISTINCT FROM NEW.coverage_watermark THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM UNNEST(NEW.evidence_ids) AS evidence(evidence_id)
          LEFT JOIN audit_log AS audit ON audit.id = evidence.evidence_id
         WHERE audit.id IS NULL
            OR audit.audit_role IS DISTINCT FROM 'evidence'
            OR audit.run_id IS DISTINCT FROM NEW.operation_id
            OR NULLIF(audit.detail ->> 'organization_id', '')::UUID
                IS DISTINCT FROM NEW.organization_id
            OR NOT (
                EXISTS (
                    SELECT 1
                      FROM candidate_attempt_evidence AS link
                      JOIN candidate_attempts AS attempt
                        ON attempt.id = link.attempt_id
                       AND attempt.operation_id = NEW.operation_id
                       AND attempt.scope_snapshot_id = NEW.scope_snapshot_id
                       AND attempt.wave_run_id = NEW.wave_run_id
                       AND attempt.wave_unit_id = NEW.wave_unit_id
                       AND attempt.organization_id = NEW.organization_id
                       AND attempt.status IN ('verified', 'refuted', 'blocked')
                     WHERE link.evidence_id = evidence.evidence_id
                       AND link.role IN ('proof', 'refutation', 'blocker')
                )
                OR EXISTS (
                    SELECT 1
                      FROM attack_candidate_work_item_evidence AS link
                      JOIN attack_candidate_work_items AS work
                        ON work.id = link.work_item_id
                       AND work.operation_id = NEW.operation_id
                       AND work.scope_snapshot_id = NEW.scope_snapshot_id
                       AND work.wave_unit_id = NEW.wave_unit_id
                       AND work.organization_id = NEW.organization_id
                       AND work.decision_kind = 'no_candidate'
                     WHERE link.evidence_id = evidence.evidence_id
                       AND link.role = 'decision'
                )
                OR EXISTS (
                    SELECT 1
                      FROM attack_fact_delta_evidence AS delta_link
                      JOIN attack_fact_deltas AS delta
                        ON delta.id = delta_link.fact_delta_id
                       AND delta.operation_id = NEW.operation_id
                       AND delta.scope_snapshot_id = NEW.scope_snapshot_id
                       AND delta.wave_run_id = NEW.wave_run_id
                       AND delta.wave_unit_id = NEW.wave_unit_id
                       AND delta.organization_id = NEW.organization_id
                       AND delta.status = 'proposed'
                      JOIN candidate_attempt_evidence AS attempt_link
                        ON attempt_link.attempt_id = delta.source_attempt_id
                       AND attempt_link.evidence_id = delta_link.evidence_id
                       AND attempt_link.role = 'fact_delta'
                     WHERE delta_link.evidence_id = evidence.evidence_id
                       AND delta_link.role = 'fact_delta'
                )
            )
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH';
    END IF;
    -- Recompute each referenced membership's owner semantics at seal time.
    -- Evidence links validate these fields when they are created, but audit_log
    -- remains mutable until this handoff freezes the referenced rows.
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
          JOIN candidate_attempts AS attempt
            ON attempt.id = (claim.value #>> '{payload,attempt_id}')::UUID
          CROSS JOIN LATERAL jsonb_array_elements_text(
              claim.value -> 'payload' -> 'evidence_ids'
          ) AS evidence(value)
          JOIN audit_log AS audit ON audit.id = evidence.value::BIGINT
         WHERE claim.value ->> 'kind' IN (
                   'candidate_attempt_terminal', 'verified_candidate_attempt'
               )
           AND audit.target_id IS NOT NULL
           AND audit.target_id IS DISTINCT FROM attempt.target_live_id
    ) OR EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
          JOIN attack_candidate_work_items AS work
            ON work.id = (claim.value #>> '{payload,work_item_id}')::UUID
          CROSS JOIN LATERAL jsonb_array_elements_text(
              claim.value -> 'payload' -> 'evidence_ids'
          ) AS evidence(value)
          JOIN audit_log AS audit ON audit.id = evidence.value::BIGINT
         WHERE claim.value ->> 'kind' = 'attack_no_candidate_decision'
           AND audit.target_id IS NOT NULL
           AND audit.target_id IS DISTINCT FROM work.target_live_id
    ) OR EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.payload -> 'typed_claims') AS claim(value)
          JOIN attack_fact_deltas AS delta
            ON delta.id = (claim.value #>> '{payload,fact_delta_id}')::UUID
          JOIN candidate_attempts AS attempt ON attempt.id = delta.source_attempt_id
          CROSS JOIN LATERAL jsonb_array_elements_text(
              claim.value -> 'payload' -> 'evidence_ids'
          ) AS evidence(value)
          JOIN audit_log AS audit ON audit.id = evidence.value::BIGINT
         WHERE claim.value ->> 'kind' = 'attack_fact_delta_proposal'
           AND (
               (
                   audit.target_id IS NOT NULL
                   AND audit.target_id IS DISTINCT FROM delta.target_live_id
               )
               OR audit.created_at < attempt.created_at
               OR audit.created_at > attempt.terminal_at
           )
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH';
    END IF;
    expected_truth_material := jsonb_build_object(
        'schema_version', 1,
        'operation_id', NEW.operation_id,
        'scope_snapshot_id', NEW.scope_snapshot_id,
        'wave_run_id', NEW.wave_run_id,
        'wave_unit_id', NEW.wave_unit_id,
        'organization_id', NEW.organization_id,
        'canonical_fact_refs', NEW.payload -> 'canonical_fact_refs',
        'typed_claims', NEW.payload -> 'typed_claims',
        'coverage_watermark', NEW.payload -> 'coverage_watermark',
        'evidence_ids', NEW.payload -> 'evidence_ids'
    );
    IF verification_sha256_jsonb(expected_truth_material)
        IS DISTINCT FROM NEW.verification_truth_hash
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_HASH_MISMATCH';
    END IF;
    RETURN NEW;
EXCEPTION WHEN OTHERS THEN
    IF SQLERRM LIKE 'VERIFICATION_TYPED_HANDOFF_%' THEN
        RAISE;
    END IF;
    RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER verification_stage_handoff_projection_exact
BEFORE INSERT ON verification_stage_handoffs
FOR EACH ROW EXECUTE FUNCTION validate_verification_stage_handoff_projection();

CREATE FUNCTION require_typed_handoff_for_verification_unit_pass()
RETURNS trigger AS $$
BEGIN
    IF NEW.stage_kind = 'verification'
        AND NEW.status = 'passed'
        AND EXISTS (
            SELECT 1
              FROM operation_state AS operation
             WHERE operation.operation_id = NEW.operation_id
               AND operation.runtime_memory_contract = 'v2_only'
               AND operation.attack_execution_contract = 'v2_only'
        )
        AND NOT EXISTS (
            SELECT 1
              FROM verification_stage_handoffs AS handoff
             WHERE handoff.operation_id = NEW.operation_id
               AND handoff.scope_snapshot_id = NEW.scope_snapshot_id
               AND handoff.organization_id = NEW.organization_id
               AND handoff.stage_execution_id = NEW.stage_execution_id
               AND handoff.source_stage_run_unit_id = NEW.id
        )
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_REQUIRED';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER verification_unit_pass_requires_typed_handoff
AFTER INSERT OR UPDATE OF status ON stage_run_units
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION require_typed_handoff_for_verification_unit_pass();

CREATE FUNCTION require_typed_handoff_for_verification_primary_worker_pass()
RETURNS trigger AS $$
BEGIN
    IF NEW.specialist = 'candidate_verifier'
        AND NEW.work_item_kind = 'organization'
        AND NEW.work_item_key = 'verification'
        AND NEW.status = 'passed'
        AND EXISTS (
            SELECT 1
              FROM operation_state AS operation
             WHERE operation.operation_id = NEW.operation_id
               AND operation.runtime_memory_contract = 'v2_only'
               AND operation.attack_execution_contract = 'v2_only'
        )
        AND NOT EXISTS (
            SELECT 1
              FROM verification_stage_handoffs AS handoff
             WHERE handoff.operation_id = NEW.operation_id
               AND handoff.organization_id = NEW.organization_id
               AND handoff.stage_execution_id = NEW.stage_execution_id
               AND handoff.source_stage_run_unit_id = NEW.stage_run_unit_id
               AND handoff.primary_worker_run_id = NEW.id
        )
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_REQUIRED';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER verification_primary_worker_pass_requires_typed_handoff
AFTER INSERT OR UPDATE OF status ON stage_worker_runs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION require_typed_handoff_for_verification_primary_worker_pass();

CREATE FUNCTION require_typed_handoff_for_ready_verification_wave_unit()
RETURNS trigger AS $$
BEGIN
    IF NEW.status = 'verification'
        AND NEW.review_closed
        AND NEW.verification_closed
        AND NEW.consolidation_status = 'ready'
        AND EXISTS (
            SELECT 1
              FROM operation_state AS operation
             WHERE operation.operation_id = NEW.operation_id
               AND operation.runtime_memory_contract = 'v2_only'
               AND operation.attack_execution_contract = 'v2_only'
        )
        AND NOT EXISTS (
            SELECT 1
              FROM verification_stage_handoffs AS handoff
             WHERE handoff.operation_id = NEW.operation_id
               AND handoff.scope_snapshot_id = NEW.scope_snapshot_id
               AND handoff.wave_run_id = NEW.wave_run_id
               AND handoff.wave_unit_id = NEW.id
               AND handoff.organization_id = NEW.organization_id
        )
    THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_REQUIRED';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER verification_wave_unit_ready_requires_typed_handoff
AFTER INSERT OR UPDATE OF status, review_closed, verification_closed, consolidation_status
ON attack_wave_units
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION require_typed_handoff_for_ready_verification_wave_unit();

CREATE FUNCTION validate_verification_stage_handoff_authority()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM operation_state AS operation
          JOIN attack_wave_runs AS wave
            ON wave.id = NEW.wave_run_id
           AND wave.operation_id = NEW.operation_id
           AND wave.scope_snapshot_id = NEW.scope_snapshot_id
           AND wave.generation = NEW.wave_generation
          JOIN attack_wave_units AS wave_unit
            ON wave_unit.id = NEW.wave_unit_id
           AND wave_unit.wave_run_id = wave.id
           AND wave_unit.operation_id = wave.operation_id
           AND wave_unit.scope_snapshot_id = wave.scope_snapshot_id
           AND wave_unit.organization_id = NEW.organization_id
          JOIN stage_run_units AS stage_unit
            ON stage_unit.id = NEW.source_stage_run_unit_id
           AND stage_unit.operation_id = NEW.operation_id
           AND stage_unit.stage_execution_id = NEW.stage_execution_id
           AND stage_unit.scope_snapshot_id = NEW.scope_snapshot_id
           AND stage_unit.organization_id = NEW.organization_id
           AND stage_unit.stage_kind = 'verification'
           AND stage_unit.generation = wave.generation
          JOIN stage_worker_runs AS worker
            ON worker.id = NEW.primary_worker_run_id
           AND worker.operation_id = NEW.operation_id
           AND worker.stage_execution_id = NEW.stage_execution_id
           AND worker.stage_run_unit_id = NEW.source_stage_run_unit_id
           AND worker.organization_id = NEW.organization_id
           AND worker.worker_generation = wave.generation
           AND worker.specialist = 'candidate_verifier'
           AND worker.work_item_kind = 'organization'
           AND worker.work_item_key = 'verification'
         WHERE operation.operation_id = NEW.operation_id
           AND operation.runtime_memory_contract = 'v2_only'
           AND operation.attack_execution_contract = 'v2_only'
           AND wave.status = 'verification'
           AND wave.terminal_at IS NULL
           AND wave_unit.status = 'verification'
           AND wave_unit.review_closed
           AND wave_unit.verification_closed
           AND wave_unit.consolidation_status = 'ready'
           AND wave_unit.terminal_at IS NULL
           AND wave_unit.row_version = NEW.wave_unit_row_version_after_close
           AND stage_unit.status = 'passed'
           AND stage_unit.terminal_at IS NOT NULL
           AND worker.status = 'passed'
           AND worker.terminal_at IS NOT NULL
           AND worker.lease_token IS NULL
           AND worker.lease_owner IS NULL
           AND worker.lease_acquired_at IS NULL
           AND worker.lease_expires_at IS NULL
           AND worker.heartbeat_at IS NULL
           AND worker.active_tool_call_id IS NULL
           AND worker.active_tool_started_at IS NULL
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_AUTHORITY_MISMATCH'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER verification_stage_handoff_authority_exact
AFTER INSERT ON verification_stage_handoffs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_verification_stage_handoff_authority();

-- Reject direct or nested-trigger pre-seeding immediately. The deferred copy
-- remains necessary to catch owner-row drift later in the same transaction.
CREATE TRIGGER verification_stage_handoff_authority_ready_at_insert
AFTER INSERT ON verification_stage_handoffs
FOR EACH ROW EXECUTE FUNCTION validate_verification_stage_handoff_authority();

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM stage_run_units AS unit
          JOIN operation_state AS operation
            ON operation.operation_id = unit.operation_id
         WHERE unit.stage_kind = 'verification'
           AND unit.status = 'passed'
           AND operation.runtime_memory_contract = 'v2_only'
           AND operation.attack_execution_contract = 'v2_only'
           AND NOT EXISTS (
               SELECT 1
                 FROM verification_stage_handoffs AS handoff
                WHERE handoff.source_stage_run_unit_id = unit.id
                  AND handoff.operation_id = unit.operation_id
                  AND handoff.stage_execution_id = unit.stage_execution_id
                  AND handoff.organization_id = unit.organization_id
           )
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_TYPED_HANDOFF_REQUIRED';
    END IF;
END;
$$;
