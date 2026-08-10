-- Enumeration v2 cryptographic closeout and provenance hardening.
--
-- Forward-only: previously applied migrations remain byte-for-byte intact.

-- Preserve every deterministic source anchor that contributed to a reduced
-- parameter fact.  The legacy scalar column remains the canonical first
-- anchor for backwards-compatible readers.
CREATE TABLE enumeration_endpoint_occurrence_parameter_source_anchors (
    parameter_id UUID NOT NULL,
    assessment_id UUID NOT NULL,
    assessment_execution_authority_id UUID NOT NULL,
    anchor_ordinal INTEGER NOT NULL CHECK (anchor_ordinal>=0),
    source_anchor TEXT NOT NULL CHECK (
        BTRIM(source_anchor)<>''
        AND source_anchor !~* '(authorization:|cookie:|password=|secret=|token=|api_key=)'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(parameter_id,anchor_ordinal),
    UNIQUE(parameter_id,source_anchor),
    FOREIGN KEY(parameter_id)
        REFERENCES enumeration_endpoint_occurrence_parameters(id) ON DELETE RESTRICT,
    FOREIGN KEY(assessment_id,assessment_execution_authority_id)
        REFERENCES enumeration_endpoint_parameter_assessments(id,execution_authority_id)
        ON DELETE RESTRICT
);

INSERT INTO enumeration_endpoint_occurrence_parameter_source_anchors(
    parameter_id,assessment_id,assessment_execution_authority_id,anchor_ordinal,source_anchor
)
SELECT id,assessment_id,assessment_execution_authority_id,0,source_anchor
  FROM enumeration_endpoint_occurrence_parameters;

CREATE FUNCTION enumeration_validate_parameter_source_anchor()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM enumeration_endpoint_occurrence_parameters parameter
         WHERE parameter.id=NEW.parameter_id
           AND parameter.assessment_id=NEW.assessment_id
           AND parameter.assessment_execution_authority_id=
               NEW.assessment_execution_authority_id
           AND (NEW.anchor_ordinal<>0 OR parameter.source_anchor=NEW.source_anchor)
         FOR SHARE
    ) THEN
        RAISE EXCEPTION 'enumeration_parameter_source_anchor_owner_mismatch'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_parameter_source_anchor_validate
BEFORE INSERT ON enumeration_endpoint_occurrence_parameter_source_anchors
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_parameter_source_anchor();
CREATE TRIGGER enumeration_parameter_source_anchor_immutable
BEFORE UPDATE OR DELETE ON enumeration_endpoint_occurrence_parameter_source_anchors
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_parameter_source_anchor_immutable'
);

CREATE FUNCTION enumeration_validate_parameter_source_anchor_set()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE anchor_count BIGINT;
DECLARE maximum_ordinal INTEGER;
DECLARE first_anchor TEXT;
BEGIN
    SELECT COUNT(*),MAX(anchor_ordinal),
           MAX(source_anchor) FILTER (WHERE anchor_ordinal=0)
      INTO anchor_count,maximum_ordinal,first_anchor
      FROM enumeration_endpoint_occurrence_parameter_source_anchors
     WHERE parameter_id=NEW.id;
    IF anchor_count=0
       OR maximum_ordinal<>anchor_count-1
       OR first_anchor IS DISTINCT FROM NEW.source_anchor THEN
        RAISE EXCEPTION 'enumeration_parameter_source_anchor_set_incomplete'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER enumeration_parameter_source_anchor_set
AFTER INSERT ON enumeration_endpoint_occurrence_parameters
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_parameter_source_anchor_set();

-- Exactly one advisory submission is permitted for a bounded Resolution
-- WorkItem/occurrence, even if a model calls the submit tool more than once.
CREATE UNIQUE INDEX enumeration_resolution_suggestion_one_per_assignment
    ON enumeration_js_resolution_suggestions(assigned_work_item_id,assigned_cluster_id);

-- Serialize every authority-owned hash input against its lane receipt.  This
-- closes the check-then-insert phantom window left by the earlier freeze
-- trigger without retroactively editing the applied migration.
CREATE OR REPLACE FUNCTION enumeration_reject_authority_write_after_lane_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE row_value JSONB;
DECLARE owner_authority_id UUID;
BEGIN
    row_value := CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    owner_authority_id := (row_value->>TG_ARGV[0])::UUID;
    IF owner_authority_id IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(hashtextextended(
            'enumeration-authority:'||owner_authority_id::TEXT,0
        ));
    END IF;
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

CREATE TRIGGER enumeration_occurrence_capture_events_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_occurrence_capture_events
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_occurrence_evidence_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_occurrence_evidence
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'evidence_execution_authority_id'
);
CREATE TRIGGER enumeration_parameter_source_anchors_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON enumeration_endpoint_occurrence_parameter_source_anchors
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'assessment_execution_authority_id'
);
CREATE TRIGGER enumeration_evidence_authorities_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON tool_truth_evidence_authorities
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_evidence_bindings_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON tool_truth_evidence_production_bindings
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_capability_receipts_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_receipts
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_capability_receipt_inputs_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_receipt_inputs
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);
CREATE TRIGGER enumeration_capability_input_evidence_lane_freeze
BEFORE INSERT OR UPDATE OR DELETE ON capability_execution_input_evidence_members
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_authority_write_after_lane_seal(
    'execution_authority_id'
);

-- Suggestions are not authority-owned rows, so derive their exact frozen
-- subject from the immutable parent occurrence + producer receipt and acquire
-- the same advisory lock as closeout/receipt writers before checking the seal.
CREATE OR REPLACE FUNCTION enumeration_reject_suggestion_after_resolution_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE subject_operation_id UUID;
DECLARE subject_organization_id UUID;
DECLARE subject_stage_execution_id UUID;
DECLARE subject_stage_run_unit_id UUID;
DECLARE subject_target_id UUID;
DECLARE subject_exact_origin TEXT;
BEGIN
    SELECT producer.operation_id,producer.organization_id,
           producer.stage_execution_id,producer.stage_run_unit_id,
           producer.target_id,producer.exact_origin
      INTO subject_operation_id,subject_organization_id,
           subject_stage_execution_id,subject_stage_run_unit_id,
           subject_target_id,subject_exact_origin
      FROM enumeration_endpoint_occurrences occurrence
      JOIN enumeration_lane_commit_receipts producer
        ON producer.execution_authority_id=occurrence.execution_authority_id
       AND producer.lane IN ('browser','js_api')
     WHERE occurrence.id=NEW.parent_occurrence_id
       AND occurrence.operation_id=NEW.operation_id
       AND occurrence.organization_id=NEW.organization_id
     FOR SHARE OF occurrence,producer;
    IF subject_operation_id IS NULL THEN
        RAISE EXCEPTION 'enumeration_resolution_suggestion_subject_missing'
            USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'enumeration-subject:'||subject_operation_id::TEXT||':'||
        subject_organization_id::TEXT||':'||subject_stage_execution_id::TEXT||':'||
        subject_stage_run_unit_id::TEXT||':'||subject_target_id::TEXT||':'||
        subject_exact_origin,0
    ));
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

CREATE FUNCTION enumeration_lock_projection_subject(
    subject_operation_id UUID,
    subject_organization_id UUID,
    subject_target_id UUID,
    subject_web_origin_id UUID
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE subject_stage_execution_id UUID;
DECLARE subject_stage_run_unit_id UUID;
DECLARE subject_exact_origin TEXT;
BEGIN
    SELECT receipt.stage_execution_id,receipt.stage_run_unit_id,origin.origin
      INTO subject_stage_execution_id,subject_stage_run_unit_id,subject_exact_origin
      FROM enumeration_lane_commit_receipts receipt
      JOIN web_origins origin
        ON origin.id=subject_web_origin_id
       AND origin.organization_id=receipt.organization_id
       AND origin.origin=receipt.exact_origin
     WHERE receipt.operation_id=subject_operation_id
       AND receipt.organization_id=subject_organization_id
       AND receipt.target_id=subject_target_id
       AND receipt.lane='browser';
    IF subject_stage_execution_id IS NULL THEN
        RAISE EXCEPTION 'enumeration_projection_browser_subject_missing'
            USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'enumeration-subject:'||subject_operation_id::TEXT||':'||
        subject_organization_id::TEXT||':'||subject_stage_execution_id::TEXT||':'||
        subject_stage_run_unit_id::TEXT||':'||subject_target_id::TEXT||':'||
        subject_exact_origin,0
    ));
    IF EXISTS (
        SELECT 1 FROM enumeration_lane_commit_receipts receipt
         WHERE receipt.operation_id=subject_operation_id
           AND receipt.organization_id=subject_organization_id
           AND receipt.stage_execution_id=subject_stage_execution_id
           AND receipt.stage_run_unit_id=subject_stage_run_unit_id
           AND receipt.target_id=subject_target_id
           AND receipt.exact_origin=subject_exact_origin
           AND receipt.lane IN ('parameter','coverage')
    ) THEN
        RAISE EXCEPTION 'enumeration_projection_write_after_parameter_seal'
            USING ERRCODE='23514';
    END IF;
END;
$$;

CREATE FUNCTION enumeration_reject_group_projection_after_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    PERFORM enumeration_lock_projection_subject(
        NEW.operation_id,NEW.organization_id,NEW.resolved_target_id,
        NEW.resolved_web_origin_id
    );
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_endpoint_groups_lane_freeze
BEFORE INSERT ON enumeration_endpoint_groups
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_group_projection_after_seal();

CREATE FUNCTION enumeration_reject_group_link_projection_after_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE endpoint_group enumeration_endpoint_groups%ROWTYPE;
BEGIN
    SELECT * INTO endpoint_group FROM enumeration_endpoint_groups
     WHERE id=NEW.group_id FOR SHARE;
    PERFORM enumeration_lock_projection_subject(
        endpoint_group.operation_id,endpoint_group.organization_id,
        endpoint_group.resolved_target_id,endpoint_group.resolved_web_origin_id
    );
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_occurrence_group_links_lane_freeze
BEFORE INSERT ON enumeration_endpoint_occurrence_group_links
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_group_link_projection_after_seal();
CREATE TRIGGER enumeration_group_api_links_lane_freeze
BEFORE INSERT ON enumeration_endpoint_group_api_links
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_group_link_projection_after_seal();

CREATE FUNCTION enumeration_reject_linked_projection_payload_after_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE row_value JSONB;
DECLARE endpoint_uuid UUID;
DECLARE observation_uuid UUID;
DECLARE subject RECORD;
BEGIN
    row_value := CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    endpoint_uuid := CASE TG_ARGV[0]
        WHEN 'endpoint' THEN (row_value->>'id')::UUID
        ELSE NULL
    END;
    observation_uuid := CASE TG_ARGV[0]
        WHEN 'observation' THEN (row_value->>'id')::UUID
        WHEN 'parameter' THEN (row_value->>'endpoint_observation_id')::UUID
        ELSE NULL
    END;
    FOR subject IN
        SELECT DISTINCT endpoint_group.operation_id,endpoint_group.organization_id,
               endpoint_group.resolved_target_id,endpoint_group.resolved_web_origin_id
          FROM enumeration_endpoint_group_api_links link
          JOIN enumeration_endpoint_groups endpoint_group ON endpoint_group.id=link.group_id
         WHERE (endpoint_uuid IS NOT NULL AND link.endpoint_id=endpoint_uuid)
            OR (observation_uuid IS NOT NULL
                AND link.endpoint_observation_id=observation_uuid)
         ORDER BY endpoint_group.operation_id,endpoint_group.organization_id,
                  endpoint_group.resolved_target_id,endpoint_group.resolved_web_origin_id
    LOOP
        PERFORM enumeration_lock_projection_subject(
            subject.operation_id,subject.organization_id,
            subject.resolved_target_id,subject.resolved_web_origin_id
        );
    END LOOP;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$;
CREATE TRIGGER enumeration_api_endpoints_lane_freeze
BEFORE UPDATE OR DELETE ON api_endpoints
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_linked_projection_payload_after_seal(
    'endpoint'
);
CREATE TRIGGER enumeration_endpoint_observations_lane_freeze
BEFORE UPDATE OR DELETE ON enumeration_endpoint_observations
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_linked_projection_payload_after_seal(
    'observation'
);
CREATE TRIGGER enumeration_endpoint_parameters_lane_freeze
BEFORE UPDATE OR DELETE ON enumeration_endpoint_parameters
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_linked_projection_payload_after_seal(
    'parameter'
);

-- A second immutable digest seals the closeout graph that the original lane
-- receipt hash could not include: capture events, typed Resolution closeout,
-- candidate/denominator closure, complete parameter anchors, and the concrete
-- legacy projection payload. Downstream lane artifacts bind this digest.
CREATE TABLE enumeration_lane_closure_graph_seals (
    lane_receipt_id UUID PRIMARY KEY
        REFERENCES enumeration_lane_commit_receipts(id) ON DELETE RESTRICT,
    graph_version TEXT NOT NULL DEFAULT 'enumeration_lane_closure_graph.v1'
        CHECK (graph_version='enumeration_lane_closure_graph.v1'),
    closure_graph_sha256 TEXT NOT NULL CHECK (
        closure_graph_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);
CREATE TRIGGER enumeration_lane_closure_graph_seal_immutable
BEFORE UPDATE OR DELETE ON enumeration_lane_closure_graph_seals
FOR EACH ROW EXECUTE FUNCTION enumeration_reject_immutable(
    'enumeration_lane_closure_graph_seal_immutable'
);

CREATE FUNCTION enumeration_lane_closure_graph_material(receipt_id UUID)
RETURNS JSONB LANGUAGE plpgsql STABLE AS $$
DECLARE receipt enumeration_lane_commit_receipts%ROWTYPE;
DECLARE producer_authority_ids UUID[] := '{}'::UUID[];
DECLARE producer_occurrence_ids UUID[] := '{}'::UUID[];
DECLARE resolution_occurrence_ids UUID[] := '{}'::UUID[];
DECLARE parameter_authority_id UUID;
BEGIN
    SELECT * INTO receipt
      FROM enumeration_lane_commit_receipts lane_receipt
     WHERE lane_receipt.id=receipt_id;
    IF receipt.id IS NULL THEN
        RAISE EXCEPTION 'enumeration_lane_closure_graph_receipt_missing'
            USING ERRCODE='23514';
    END IF;

    IF receipt.lane IN ('browser','js_api') THEN
        producer_authority_ids := ARRAY[receipt.execution_authority_id];
    ELSE
        SELECT COALESCE(array_agg(DISTINCT dependency.execution_authority_id
                                  ORDER BY dependency.execution_authority_id),'{}'::UUID[])
          INTO producer_authority_ids
          FROM enumeration_lane_commit_receipts dependency
         WHERE dependency.id=ANY(receipt.dependency_receipt_ids)
           AND dependency.lane IN ('browser','js_api');
    END IF;
    SELECT COALESCE(array_agg(occurrence.id ORDER BY occurrence.id),'{}'::UUID[])
      INTO producer_occurrence_ids
      FROM enumeration_endpoint_occurrences occurrence
     WHERE occurrence.execution_authority_id=ANY(producer_authority_ids)
       AND occurrence.source_target_id=receipt.target_id
       AND EXISTS (
           SELECT 1 FROM web_origins origin
            WHERE origin.id=occurrence.source_web_origin_id
              AND origin.origin=receipt.exact_origin
       );
    resolution_occurrence_ids := CASE receipt.lane
        WHEN 'resolution' THEN ARRAY[receipt.resolution_occurrence_id]
        WHEN 'coverage' THEN COALESCE((
            SELECT array_agg(dependency.resolution_occurrence_id
                             ORDER BY dependency.resolution_occurrence_id)
              FROM enumeration_lane_commit_receipts dependency
             WHERE dependency.id=ANY(receipt.dependency_receipt_ids)
               AND dependency.lane='resolution'
        ),'{}'::UUID[])
        ELSE '{}'::UUID[]
    END;
    parameter_authority_id := CASE receipt.lane
        WHEN 'parameter' THEN receipt.execution_authority_id
        WHEN 'coverage' THEN (
            SELECT dependency.execution_authority_id
              FROM enumeration_lane_commit_receipts dependency
             WHERE dependency.id=ANY(receipt.dependency_receipt_ids)
               AND dependency.lane='parameter'
        )
        ELSE NULL
    END;

    RETURN jsonb_build_object(
        'graph_version','enumeration_lane_closure_graph.v1',
        'lane_receipt_id',receipt.id,
        'lane',receipt.lane,
        'receipt_set_sha256',receipt.receipt_set_sha256,
        'dependencies',(
            SELECT COALESCE(jsonb_agg(jsonb_build_object(
                       'receipt_id',dependency.id,
                       'receipt_set_sha256',dependency.receipt_set_sha256,
                       'closure_graph_sha256',seal.closure_graph_sha256
                   ) ORDER BY dependency.id),'[]'::JSONB)
              FROM enumeration_lane_commit_receipts dependency
              JOIN enumeration_lane_closure_graph_seals seal
                ON seal.lane_receipt_id=dependency.id
             WHERE dependency.id=ANY(receipt.dependency_receipt_ids)
        ),
        'authority_evidence',(
            SELECT COALESCE(jsonb_agg(to_jsonb(evidence)-'created_at'
                                      ORDER BY evidence.id),'[]'::JSONB)
              FROM tool_truth_evidence_authorities evidence
             WHERE evidence.execution_authority_id=receipt.execution_authority_id
        ),
        'authority_evidence_bindings',(
            SELECT COALESCE(jsonb_agg(to_jsonb(binding)-'created_at'
                                      ORDER BY binding.id),'[]'::JSONB)
              FROM tool_truth_evidence_production_bindings binding
             WHERE binding.execution_authority_id=receipt.execution_authority_id
        ),
        'capability_receipts',(
            SELECT COALESCE(jsonb_agg(to_jsonb(capability_receipt)-'created_at'
                                      ORDER BY capability_receipt.id),'[]'::JSONB)
              FROM capability_execution_receipts capability_receipt
             WHERE capability_receipt.execution_authority_id=receipt.execution_authority_id
        ),
        'capability_inputs',(
            SELECT COALESCE(jsonb_agg(to_jsonb(input)-ARRAY['created_at','updated_at']
                                      ORDER BY input.id),'[]'::JSONB)
              FROM capability_execution_receipt_inputs input
             WHERE input.execution_authority_id=receipt.execution_authority_id
        ),
        'capability_input_evidence',(
            SELECT COALESCE(jsonb_agg(to_jsonb(member) ORDER BY member.id),'[]'::JSONB)
              FROM capability_execution_input_evidence_members member
             WHERE member.execution_authority_id=receipt.execution_authority_id
        ),
        'candidate_capture_events',(
            SELECT COALESCE(jsonb_agg(to_jsonb(event)-'created_at'
                                      ORDER BY event.capture_event_id),'[]'::JSONB)
              FROM enumeration_endpoint_candidate_capture_events event
             WHERE event.execution_authority_id=ANY(producer_authority_ids)
        ),
        'occurrence_capture_events',(
            SELECT COALESCE(jsonb_agg(to_jsonb(event)-'linked_at'
                                      ORDER BY event.occurrence_id,event.capture_event_id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_capture_events event
             WHERE event.occurrence_id=ANY(producer_occurrence_ids)
        ),
        'candidate_closures',(
            SELECT COALESCE(jsonb_agg(to_jsonb(closure)-'created_at'
                                      ORDER BY closure.id),'[]'::JSONB)
              FROM enumeration_endpoint_candidate_closure_receipts closure
             WHERE closure.execution_authority_id=ANY(producer_authority_ids)
        ),
        'candidate_denominator_closures',(
            SELECT COALESCE(jsonb_agg(to_jsonb(closure)-'created_at'
                                      ORDER BY closure.id),'[]'::JSONB)
              FROM enumeration_endpoint_candidate_denominator_closure_receipts closure
             WHERE closure.execution_authority_id=ANY(producer_authority_ids)
        ),
        'resolution_suggestions',(
            SELECT COALESCE(jsonb_agg(to_jsonb(suggestion)-'created_at'
                                      ORDER BY suggestion.id),'[]'::JSONB)
              FROM enumeration_js_resolution_suggestions suggestion
             WHERE suggestion.parent_occurrence_id=ANY(resolution_occurrence_ids)
        ),
        'resolution_closeouts',(
            SELECT COALESCE(jsonb_agg(to_jsonb(closeout)-'created_at'
                                      ORDER BY closeout.id),'[]'::JSONB)
              FROM enumeration_resolution_closeout_receipts closeout
             WHERE closeout.parent_occurrence_id=ANY(resolution_occurrence_ids)
        ),
        'parameter_assessments',(
            SELECT COALESCE(jsonb_agg(to_jsonb(assessment)-'created_at'
                                      ORDER BY assessment.id),'[]'::JSONB)
              FROM enumeration_endpoint_parameter_assessments assessment
             WHERE assessment.execution_authority_id=parameter_authority_id
        ),
        'parameter_facts',(
            SELECT COALESCE(jsonb_agg(to_jsonb(parameter)-'created_at'
                                      ORDER BY parameter.id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_parameters parameter
              JOIN enumeration_endpoint_parameter_assessments assessment
                ON assessment.id=parameter.assessment_id
             WHERE assessment.execution_authority_id=parameter_authority_id
        ),
        'parameter_source_anchors',(
            SELECT COALESCE(jsonb_agg(to_jsonb(anchor)-'created_at'
                                      ORDER BY anchor.parameter_id,anchor.anchor_ordinal),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_parameter_source_anchors anchor
             WHERE anchor.assessment_execution_authority_id=parameter_authority_id
        ),
        'groups',(
            SELECT COALESCE(jsonb_agg(to_jsonb(endpoint_group)-'created_at'
                                      ORDER BY endpoint_group.id),'[]'::JSONB)
              FROM enumeration_endpoint_groups endpoint_group
             WHERE EXISTS (
                 SELECT 1 FROM enumeration_endpoint_occurrence_group_links link
                  WHERE link.group_id=endpoint_group.id
                    AND link.occurrence_id=ANY(producer_occurrence_ids)
             )
        ),
        'occurrence_group_links',(
            SELECT COALESCE(jsonb_agg(to_jsonb(link)-'created_at'
                                      ORDER BY link.occurrence_id,link.group_id),'[]'::JSONB)
              FROM enumeration_endpoint_occurrence_group_links link
             WHERE link.occurrence_id=ANY(producer_occurrence_ids)
        ),
        'group_api_links',(
            SELECT COALESCE(jsonb_agg(to_jsonb(link)-'created_at'
                                      ORDER BY link.group_id),'[]'::JSONB)
              FROM enumeration_endpoint_group_api_links link
             WHERE EXISTS (
                 SELECT 1 FROM enumeration_endpoint_occurrence_group_links occurrence_link
                  WHERE occurrence_link.group_id=link.group_id
                    AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)
             )
        ),
        'api_endpoints',(
            SELECT COALESCE(jsonb_agg(to_jsonb(endpoint)-ARRAY['discovered_at','updated_at']
                                      ORDER BY endpoint.id),'[]'::JSONB)
              FROM api_endpoints endpoint
             WHERE EXISTS (
                 SELECT 1 FROM enumeration_endpoint_group_api_links link
                 JOIN enumeration_endpoint_occurrence_group_links occurrence_link
                   ON occurrence_link.group_id=link.group_id
                  WHERE link.endpoint_id=endpoint.id
                    AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)
             )
        ),
        'endpoint_observations',(
            SELECT COALESCE(jsonb_agg(to_jsonb(observation)-ARRAY['created_at','updated_at']
                                      ORDER BY observation.id),'[]'::JSONB)
              FROM enumeration_endpoint_observations observation
             WHERE EXISTS (
                 SELECT 1 FROM enumeration_endpoint_group_api_links link
                 JOIN enumeration_endpoint_occurrence_group_links occurrence_link
                   ON occurrence_link.group_id=link.group_id
                  WHERE link.endpoint_observation_id=observation.id
                    AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)
             )
        ),
        'endpoint_parameters',(
            SELECT COALESCE(jsonb_agg(to_jsonb(parameter)-ARRAY['created_at','updated_at']
                                      ORDER BY parameter.id),'[]'::JSONB)
              FROM enumeration_endpoint_parameters parameter
             WHERE EXISTS (
                 SELECT 1 FROM enumeration_endpoint_group_api_links link
                 JOIN enumeration_endpoint_occurrence_group_links occurrence_link
                   ON occurrence_link.group_id=link.group_id
                  WHERE link.endpoint_observation_id=parameter.endpoint_observation_id
                    AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)
             )
        )
    );
END;
$$;

CREATE FUNCTION enumeration_compute_lane_closure_graph_sha256(receipt_id UUID)
RETURNS TEXT LANGUAGE SQL STABLE AS $$
    SELECT tool_truth_sha256(enumeration_lane_closure_graph_material(receipt_id)::TEXT)
$$;

-- Backfill historical WIP rows in dependency order without mutating the
-- immutable lane receipts themselves.
INSERT INTO enumeration_lane_closure_graph_seals(lane_receipt_id,closure_graph_sha256)
SELECT id,enumeration_compute_lane_closure_graph_sha256(id)
  FROM enumeration_lane_commit_receipts
 WHERE lane='browser' ORDER BY id;
INSERT INTO enumeration_lane_closure_graph_seals(lane_receipt_id,closure_graph_sha256)
SELECT id,enumeration_compute_lane_closure_graph_sha256(id)
  FROM enumeration_lane_commit_receipts
 WHERE lane='js_api' ORDER BY id;
INSERT INTO enumeration_lane_closure_graph_seals(lane_receipt_id,closure_graph_sha256)
SELECT id,enumeration_compute_lane_closure_graph_sha256(id)
  FROM enumeration_lane_commit_receipts
 WHERE lane IN ('parameter','resolution')
 ORDER BY CASE lane WHEN 'parameter' THEN 0 ELSE 1 END,id;
INSERT INTO enumeration_lane_closure_graph_seals(lane_receipt_id,closure_graph_sha256)
SELECT id,enumeration_compute_lane_closure_graph_sha256(id)
  FROM enumeration_lane_commit_receipts
 WHERE lane='coverage' ORDER BY id;

CREATE FUNCTION enumeration_validate_lane_closure_graph_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected_sha256 TEXT;
BEGIN
    expected_sha256 := enumeration_compute_lane_closure_graph_sha256(NEW.lane_receipt_id);
    IF NEW.closure_graph_sha256<>expected_sha256 THEN
        RAISE EXCEPTION 'enumeration_lane_closure_graph_hash_mismatch'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER enumeration_lane_closure_graph_seal_validate
BEFORE INSERT ON enumeration_lane_closure_graph_seals
FOR EACH ROW EXECUTE FUNCTION enumeration_validate_lane_closure_graph_seal();

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM enumeration_lane_closure_graph_seals seal
         WHERE seal.closure_graph_sha256<>
               enumeration_compute_lane_closure_graph_sha256(seal.lane_receipt_id)
    ) THEN
        RAISE EXCEPTION 'enumeration_lane_closure_graph_backfill_drift'
            USING ERRCODE='23514';
    END IF;
END;
$$;

CREATE FUNCTION enumeration_require_lane_closure_graph_seal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_lane_closure_graph_seals seal
         WHERE seal.lane_receipt_id=NEW.id
    ) THEN
        RAISE EXCEPTION 'enumeration_lane_closure_graph_seal_missing'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER enumeration_lane_closure_graph_seal_required
AFTER INSERT ON enumeration_lane_commit_receipts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enumeration_require_lane_closure_graph_seal();
