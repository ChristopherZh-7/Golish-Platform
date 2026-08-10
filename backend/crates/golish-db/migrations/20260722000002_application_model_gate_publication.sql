-- Trusted publication seam for the dormant Application Understanding stage.
--
-- This migration does not register the stage in the runtime graph. It replaces
-- the S1 unconditional publication blockers with deferred relational checks so
-- only one complete revision/current/Handoff/runtime-final-seal bundle commits.

DROP TRIGGER application_model_current_revisions_dormant
    ON application_model_current_revisions;
DROP TRIGGER application_model_stage_handoffs_dormant
    ON stage_handoffs;

CREATE OR REPLACE FUNCTION application_model_restrict_revision_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_IMMUTABLE';
    END IF;
    IF OLD.status = 'building'
       AND NEW.status = 'proposed'
       AND NEW.row_version = OLD.row_version
       AND NEW.finalized_at IS NULL
       AND (to_jsonb(NEW) - ARRAY['status', 'row_version', 'finalized_at'])
           = (to_jsonb(OLD) - ARRAY['status', 'row_version', 'finalized_at'])
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status = 'proposed'
       AND OLD.row_version = 0
       AND OLD.finalized_at IS NULL
       AND NEW.status = 'final'
       AND NEW.row_version = 1
       AND NEW.finalized_at = transaction_timestamp()
       AND (to_jsonb(NEW) - ARRAY['status', 'row_version', 'finalized_at'])
           = (to_jsonb(OLD) - ARRAY['status', 'row_version', 'finalized_at'])
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION application_model_revision_canonical_content(target_revision_id UUID)
RETURNS JSONB AS $$
    SELECT jsonb_build_object(
        'revision', to_jsonb(revision.*),
        'manifest', to_jsonb(manifest.*),
        'inputs', (
            SELECT COALESCE(
                jsonb_agg(to_jsonb(input.*) ORDER BY input.ordinal),
                '[]'::jsonb
            )
              FROM application_model_manifest_inputs AS input
             WHERE input.manifest_id=manifest.id
        ),
        'decisions', (
            SELECT COALESCE(
                jsonb_agg(to_jsonb(decision.*) ORDER BY decision.input_key),
                '[]'::jsonb
            )
              FROM application_model_input_decisions AS decision
             WHERE decision.revision_id=revision.id
        ),
        'items', (
            SELECT COALESCE(
                jsonb_agg(to_jsonb(item.*) ORDER BY item.ordinal),
                '[]'::jsonb
            )
              FROM application_model_items AS item
             WHERE item.revision_id=revision.id
        ),
        'item_evidence', (
            SELECT COALESCE(
                jsonb_agg(
                    to_jsonb(item_evidence.*)
                    ORDER BY item_evidence.item_key,item_evidence.evidence_id
                ),
                '[]'::jsonb
            )
              FROM application_model_item_evidence AS item_evidence
             WHERE item_evidence.revision_id=revision.id
        )
    )
      FROM application_model_revisions AS revision
      JOIN application_model_manifests AS manifest
        ON manifest.id=revision.manifest_id
     WHERE revision.id=target_revision_id;
$$ LANGUAGE sql STABLE STRICT;

CREATE FUNCTION application_model_revision_evidence_ids(target_revision_id UUID)
RETURNS BIGINT[] AS $$
    SELECT COALESCE(array_agg(evidence_id ORDER BY evidence_id), '{}'::BIGINT[])
      FROM (
          SELECT unnest(input.evidence_ids) AS evidence_id
            FROM application_model_revisions AS revision
            JOIN application_model_manifest_inputs AS input
              ON input.manifest_id=revision.manifest_id
           WHERE revision.id=target_revision_id
          UNION
          SELECT item_evidence.evidence_id
            FROM application_model_item_evidence AS item_evidence
           WHERE item_evidence.revision_id=target_revision_id
      ) AS evidence;
$$ LANGUAGE sql STABLE STRICT;

CREATE FUNCTION application_model_manifest_gate_material(target_manifest_id UUID)
RETURNS JSONB AS $$
    SELECT jsonb_build_object(
        'schema_version', 'application_model_manifest.v1',
        'manifest_id', manifest.id,
        'operation_id', manifest.operation_id,
        'scope_snapshot_id', manifest.scope_snapshot_id,
        'stage_execution_id', manifest.stage_execution_id,
        'stage_run_unit_id', manifest.stage_run_unit_id,
        'organization_id', manifest.organization_id,
        'authority_kind', manifest.authority_kind,
        'inputs', (
            SELECT COALESCE(
                jsonb_agg(
                    jsonb_build_object(
                        'input_key', input.input_key,
                        'ordinal', input.ordinal,
                        'input_kind', input.input_kind,
                        'source_handoff_id', input.source_handoff_id,
                        'source_kind', input.source_kind,
                        'source_id', input.source_id,
                        'source_version', input.source_version,
                        'source_payload_hash',
                            application_model_sha256_jsonb(input.source_payload),
                        'evidence_ids', input.evidence_ids
                    ) ORDER BY input.ordinal
                ),
                '[]'::jsonb
            )
              FROM application_model_manifest_inputs AS input
             WHERE input.manifest_id=manifest.id
        )
    )
      FROM application_model_manifests AS manifest
     WHERE manifest.id=target_manifest_id;
$$ LANGUAGE sql STABLE STRICT;

CREATE FUNCTION application_model_revision_gate_material(target_revision_id UUID)
RETURNS JSONB AS $$
    SELECT jsonb_build_object(
        'schema_version', 'application_model_revision.v1',
        'manifest_id', revision.manifest_id,
        'operation_id', revision.operation_id,
        'scope_snapshot_id', revision.scope_snapshot_id,
        'stage_execution_id', revision.stage_execution_id,
        'stage_run_unit_id', revision.stage_run_unit_id,
        'organization_id', revision.organization_id,
        'source_submission_id', revision.source_submission_id,
        'structured_model', revision.structured_model,
        'decisions', (
            SELECT COALESCE(
                jsonb_agg(
                    jsonb_build_object(
                        'input_key', decision.input_key,
                        'disposition', decision.disposition,
                        'item_keys', decision.item_keys,
                        'duplicate_input_key', decision.duplicate_input_key,
                        'reason_code', decision.reason_code
                    ) ORDER BY decision.input_key
                ),
                '[]'::jsonb
            )
              FROM application_model_input_decisions AS decision
             WHERE decision.revision_id=revision.id
        ),
        'items', (
            SELECT COALESCE(
                jsonb_agg(
                    jsonb_build_object(
                        'item_key', item.item_key,
                        'item_kind', item.item_kind,
                        'truth_state', item.truth_state,
                        'source_input_keys', item.source_input_keys,
                        'referenced_item_keys', item.referenced_item_keys,
                        'payload', item.payload,
                        'evidence', (
                            SELECT COALESCE(
                                jsonb_agg(
                                    jsonb_build_object(
                                        'evidence_id', evidence.evidence_id,
                                        'role', evidence.role
                                    ) ORDER BY evidence.evidence_id
                                ),
                                '[]'::jsonb
                            )
                              FROM application_model_item_evidence AS evidence
                             WHERE evidence.revision_id=item.revision_id
                               AND evidence.item_key=item.item_key
                        )
                    ) ORDER BY item.ordinal
                ),
                '[]'::jsonb
            )
              FROM application_model_items AS item
             WHERE item.revision_id=revision.id
        )
    )
      FROM application_model_revisions AS revision
     WHERE revision.id=target_revision_id;
$$ LANGUAGE sql STABLE STRICT;

CREATE OR REPLACE FUNCTION application_model_validate_current_revision()
RETURNS trigger AS $$
DECLARE
    current_row application_model_current_revisions%ROWTYPE;
    manifest application_model_manifests%ROWTYPE;
    revision application_model_revisions%ROWTYPE;
    handoff stage_handoffs%ROWTYPE;
    unit stage_run_units%ROWTYPE;
    worker stage_worker_runs%ROWTYPE;
    submission stage_deliverable_submissions%ROWTYPE;
    completion org_stage_completions%ROWTYPE;
    frozen_scope_hash TEXT;
    expected_source_handoff_ids UUID[];
    actual_source_handoff_ids UUID[];
    expected_evidence_ids BIGINT[];
    expected_evidence_watermark BIGINT;
    expected_manifest_hash TEXT;
    expected_model_hash TEXT;
    expected_replay_material_hash TEXT;
    revision_material JSONB;
    expected_claim JSONB;
    expected_claims JSONB;
    canonical_refs JSONB;
    revision_ref JSONB;
    expected_coverage JSONB;
    expected_terminal_checkpoint JSONB;
    expected_gate_details JSONB;
    expected_canonical_keys JSONB;
    expected_seal_material JSONB;
    expected_gate_decision JSONB;
    expected_gate_decision_hash TEXT;
    expected_pass_watermark JSONB;
    expected_payload JSONB;
BEGIN
    SELECT * INTO STRICT current_row
      FROM application_model_current_revisions
     WHERE manifest_id=NEW.manifest_id;
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id=current_row.manifest_id;
    SELECT * INTO STRICT handoff
      FROM stage_handoffs
     WHERE id=current_row.stage_handoff_id;
    SELECT * INTO STRICT unit
      FROM stage_run_units
     WHERE id=manifest.stage_run_unit_id;
    SELECT * INTO STRICT submission
      FROM stage_deliverable_submissions
     WHERE id=current_row.deliverable_submission_id;
    SELECT * INTO STRICT worker
      FROM stage_worker_runs
     WHERE id=submission.worker_run_id;
    SELECT scope_hash INTO STRICT frozen_scope_hash
      FROM operation_org_scope_snapshots
     WHERE id=manifest.scope_snapshot_id
       AND operation_id=manifest.operation_id
       AND sealed_at IS NOT NULL;
    SELECT * INTO STRICT completion
      FROM org_stage_completions
     WHERE organization_id=manifest.organization_id
       AND stage_kind='application_understanding';

    SELECT COALESCE(array_agg(source.id ORDER BY source.id), '{}'::UUID[])
      INTO expected_source_handoff_ids
      FROM stage_handoffs AS source
      JOIN stage_run_units AS source_unit
        ON source_unit.id=source.source_stage_run_unit_id
       AND source_unit.operation_id=source.operation_id
       AND source_unit.stage_execution_id=source.stage_execution_id
       AND source_unit.organization_id=source.organization_id
       AND source_unit.stage_kind=source.from_stage_kind
     WHERE source.operation_id=manifest.operation_id
       AND source.scope_snapshot_id=manifest.scope_snapshot_id
       AND source.organization_id=manifest.organization_id
       AND source.from_stage_kind IN (
           'target_intel',
           'external_attack_surface',
           'enumeration',
           'vuln_triage'
       )
       AND source.invalidated_at IS NULL
       AND source_unit.status='passed';
    SELECT COALESCE(
               array_agg(input.source_handoff_id ORDER BY input.source_handoff_id),
               '{}'::UUID[]
           )
      INTO actual_source_handoff_ids
      FROM application_model_manifest_inputs AS input
     WHERE input.manifest_id=manifest.id;

    SELECT COALESCE(array_agg(evidence_id ORDER BY evidence_id), '{}'::BIGINT[])
      INTO expected_evidence_ids
      FROM (
          SELECT unnest(input.evidence_ids) AS evidence_id
            FROM application_model_manifest_inputs AS input
           WHERE input.manifest_id=manifest.id
          UNION
          SELECT evidence.evidence_id
            FROM application_model_item_evidence AS evidence
           WHERE evidence.manifest_id=manifest.id
      ) AS evidence;
    SELECT max(evidence_id) INTO expected_evidence_watermark
      FROM unnest(expected_evidence_ids) AS evidence_id;

    expected_manifest_hash := application_model_sha256_jsonb(
        application_model_manifest_gate_material(manifest.id)
    );
    IF manifest.authority_kind='model' THEN
        SELECT * INTO STRICT revision
          FROM application_model_revisions
         WHERE id=current_row.revision_id
           AND manifest_id=manifest.id;
        revision_material := application_model_revision_gate_material(revision.id);
        expected_model_hash := application_model_sha256_jsonb(revision.structured_model);
        expected_replay_material_hash := application_model_sha256_jsonb(revision_material);
        expected_canonical_keys := jsonb_build_array(
            jsonb_build_object(
                'kind', 'application_model_revision',
                'revision_id', revision.id
            )
        );
    ELSE
        expected_model_hash := NULL;
        expected_replay_material_hash := expected_manifest_hash;
        expected_canonical_keys := '[]'::JSONB;
    END IF;

    expected_claim := jsonb_build_object(
        'kind', 'application_model_authority',
        'payload', jsonb_build_object(
            'authority_kind', manifest.authority_kind,
            'manifest_id', manifest.id,
            'revision_id', current_row.revision_id,
            'manifest_hash', expected_manifest_hash,
            'model_hash', expected_model_hash,
            'replay_material_hash', expected_replay_material_hash,
            'deliverable_submission_id', submission.id
        )
    );
    expected_claims := jsonb_build_array(expected_claim);
    expected_coverage := jsonb_build_object(
        'schema_version', 'application_model_coverage.v1',
        'manifest_id', manifest.id,
        'revision_id', current_row.revision_id,
        'input_count', (
            SELECT count(*) FROM application_model_manifest_inputs
             WHERE manifest_id=manifest.id
        ),
        'decision_count', (
            SELECT count(*) FROM application_model_input_decisions
             WHERE manifest_id=manifest.id
        ),
        'item_count', (
            SELECT count(*) FROM application_model_items
             WHERE manifest_id=manifest.id
        ),
        'manifest_hash', expected_manifest_hash,
        'model_hash', expected_model_hash,
        'replay_material_hash', expected_replay_material_hash
    );
    expected_terminal_checkpoint := jsonb_build_object(
        'schema_version', 'application_model_terminal.v1',
        'manifest_id', manifest.id,
        'revision_id', current_row.revision_id,
        'manifest_hash', expected_manifest_hash,
        'model_hash', expected_model_hash,
        'replay_material_hash', expected_replay_material_hash,
        'deliverable_submission_id', submission.id
    );
    expected_gate_details := jsonb_build_object(
        'code', 'APPLICATION_MODEL_GATE_PASS',
        'authority_kind', manifest.authority_kind,
        'manifest_id', manifest.id,
        'revision_id', current_row.revision_id,
        'manifest_hash', expected_manifest_hash,
        'model_hash', expected_model_hash,
        'replay_material_hash', expected_replay_material_hash
    );
    expected_seal_material := jsonb_build_object(
        'canonical_fact_keys', expected_canonical_keys,
        'typed_claims', expected_claims,
        'coverage_watermark', expected_coverage,
        'evidence_ids', expected_evidence_ids,
        'terminal_checkpoint', expected_terminal_checkpoint,
        'deterministic_gate_details', expected_gate_details,
        'candidate_acceptance', NULL
    );
    expected_gate_decision := jsonb_build_object(
        'outcome', 'pass',
        'operation_id', manifest.operation_id,
        'stage_execution_id', manifest.stage_execution_id,
        'stage_run_unit_id', manifest.stage_run_unit_id,
        'deliverable_submission_id', submission.id,
        'scope_hash', frozen_scope_hash,
        'details', expected_gate_details,
        'seal_material_sha256', substring(
            application_model_sha256_jsonb(expected_seal_material) FROM 8
        )
    );
    expected_gate_decision_hash := application_model_sha256_jsonb(expected_gate_decision);
    expected_pass_watermark := jsonb_build_object(
        'handoff_id', handoff.id,
        'deliverable_submission_id', submission.id,
        'scope_hash', frozen_scope_hash,
        'coverage_watermark', expected_coverage,
        'gate_decision_hash', substring(expected_gate_decision_hash FROM 8),
        'evidence_watermark', expected_evidence_watermark
    );

    canonical_refs := handoff.payload -> 'canonical_fact_refs';
    IF manifest.authority_kind='model' THEN
        IF jsonb_typeof(canonical_refs) IS DISTINCT FROM 'array'
           OR jsonb_array_length(canonical_refs) <> 1
        THEN
            RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_REVISION_MISMATCH';
        END IF;
        revision_ref := canonical_refs -> 0;
        IF jsonb_typeof(revision_ref) IS DISTINCT FROM 'object'
           OR (SELECT count(*) FROM jsonb_object_keys(revision_ref)) <> 5
           OR jsonb_typeof(revision_ref -> 'key') IS DISTINCT FROM 'object'
           OR (
               SELECT count(*) FROM jsonb_object_keys(revision_ref -> 'key')
           ) <> 2
           OR revision_ref #>> '{key,kind}' <> 'application_model_revision'
           OR revision_ref #>> '{key,revision_id}' <> revision.id::TEXT
           OR revision_ref #>> '{organization_id}' <> revision.organization_id::TEXT
           OR (revision_ref #>> '{observed_at}')::TIMESTAMPTZ
                IS DISTINCT FROM revision.finalized_at
           OR revision_ref #>> '{content_sha256}' <>
                substring(
                    application_model_sha256_jsonb(
                        application_model_revision_canonical_content(revision.id)
                    ) FROM 8
                )
           OR revision_ref -> 'evidence_ids' <>
                to_jsonb(application_model_revision_evidence_ids(revision.id))
        THEN
            RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_REVISION_MISMATCH';
        END IF;
    ELSIF canonical_refs IS DISTINCT FROM '[]'::JSONB THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_AUTHORITY_MISMATCH';
    END IF;

    expected_payload := jsonb_build_object(
        'schema_version', 1,
        'canonical_fact_refs', canonical_refs,
        'typed_claims', expected_claims,
        'coverage_watermark', expected_coverage,
        'evidence_ids', expected_evidence_ids
    );

    IF current_row.authority_kind <> manifest.authority_kind
       OR manifest.stage_kind <> 'application_understanding'
       OR manifest.scope_snapshot_id <> unit.scope_snapshot_id
       OR manifest.input_count <> cardinality(actual_source_handoff_ids)
       OR actual_source_handoff_ids IS DISTINCT FROM expected_source_handoff_ids
       OR manifest.manifest_hash <> expected_manifest_hash
       OR manifest.replay_material_hash <> expected_manifest_hash
       OR current_row.manifest_hash <> expected_manifest_hash
       OR current_row.model_hash IS DISTINCT FROM expected_model_hash
       OR current_row.replay_material_hash <> expected_replay_material_hash
       OR current_row.gate_decision_hash <> expected_gate_decision_hash
       OR current_row.stage_handoff_id <> handoff.id
       OR current_row.deliverable_submission_id <> submission.id
       OR current_row.published_at IS DISTINCT FROM handoff.gate_passed_at
       OR handoff.operation_id <> manifest.operation_id
       OR handoff.scope_snapshot_id <> manifest.scope_snapshot_id
       OR handoff.organization_id <> manifest.organization_id
       OR handoff.from_stage_kind <> 'application_understanding'
       OR handoff.stage_execution_id <> manifest.stage_execution_id
       OR handoff.source_stage_run_unit_id <> manifest.stage_run_unit_id
       OR handoff.deliverable_submission_id <> submission.id
       OR handoff.scope_hash <> frozen_scope_hash
       OR handoff.invalidated_at IS NOT NULL
       OR handoff.schema_version <> 1
       OR handoff.aggregate_pass_token_hash IS NOT NULL
       OR handoff.payload IS DISTINCT FROM expected_payload
       OR ('sha256:' || handoff.payload_sha256) <>
            application_model_sha256_jsonb(handoff.payload)
       OR handoff.evidence_ids IS DISTINCT FROM expected_evidence_ids
       OR handoff.coverage_watermark IS DISTINCT FROM expected_coverage
       OR ('sha256:' || handoff.unit_gate_decision_hash) <>
            expected_gate_decision_hash
       OR unit.operation_id <> manifest.operation_id
       OR unit.stage_execution_id <> manifest.stage_execution_id
       OR unit.organization_id <> manifest.organization_id
       OR unit.stage_kind <> 'application_understanding'
       OR unit.status <> 'passed'
       OR unit.terminal_at IS DISTINCT FROM handoff.gate_passed_at
       OR unit.pass_watermark IS DISTINCT FROM expected_pass_watermark
       OR submission.operation_id <> manifest.operation_id
       OR submission.stage_execution_id <> manifest.stage_execution_id
       OR submission.stage_run_unit_id <> manifest.stage_run_unit_id
       OR submission.organization_id <> manifest.organization_id
       OR submission.stage_kind <> 'application_understanding'
       OR ('sha256:' || submission.payload_sha256) <>
            application_model_sha256_jsonb(submission.payload)
       OR worker.operation_id <> manifest.operation_id
       OR worker.stage_execution_id <> manifest.stage_execution_id
       OR worker.stage_run_unit_id <> manifest.stage_run_unit_id
       OR worker.organization_id <> manifest.organization_id
       OR worker.worker_generation <> unit.generation
       OR worker.status <> 'passed'
       OR worker.lease_token IS NOT NULL
       OR worker.lease_owner IS NOT NULL
       OR worker.lease_expires_at IS NOT NULL
       OR worker.active_tool_call_id IS NOT NULL
       OR worker.checkpoint IS DISTINCT FROM expected_terminal_checkpoint
       OR worker.evidence_watermark IS DISTINCT FROM expected_evidence_watermark
       OR worker.terminal_at IS DISTINCT FROM handoff.gate_passed_at
       OR submission.attempt_epoch IS DISTINCT FROM worker.attempt_epoch
       OR completion.passed_at IS DISTINCT FROM handoff.gate_passed_at
       OR completion.stage_run_id IS DISTINCT FROM manifest.operation_id::TEXT
       OR NOT EXISTS (
           SELECT 1 FROM tool_calls AS tool
            WHERE tool.id=submission.tool_call_record_id
              AND tool.name='submit_stage_deliverable'
              AND tool.call_id=submission.tool_request_id
              AND tool.status='finished'
       )
       OR EXISTS (
           SELECT 1 FROM stage_worker_runs AS sibling
            WHERE sibling.stage_run_unit_id=manifest.stage_run_unit_id
              AND sibling.id<>worker.id
              AND sibling.status NOT IN ('passed','failed','exhausted','superseded')
       )
       OR EXISTS (
           SELECT 1 FROM stage_work_items AS work_item
            WHERE work_item.stage_run_unit_id=manifest.stage_run_unit_id
              AND work_item.status NOT IN ('completed','exhausted','superseded')
       )
       OR EXISTS (
           SELECT 1 FROM tool_calls AS tool
            WHERE tool.operation_id=manifest.operation_id
              AND tool.stage_execution_id=manifest.stage_execution_id
              AND tool.stage_run_unit_id=manifest.stage_run_unit_id
              AND tool.name NOT IN ('submit_stage_deliverable','update_plan')
       )
       OR EXISTS (
           SELECT 1 FROM tool_calls AS tool
            WHERE tool.operation_id=manifest.operation_id
              AND tool.stage_execution_id=manifest.stage_execution_id
              AND tool.stage_run_unit_id=manifest.stage_run_unit_id
              AND tool.status IN ('received','running')
       )
       OR EXISTS (
           SELECT 1
             FROM application_model_manifest_inputs AS input
             JOIN stage_handoffs AS source
               ON source.id=input.source_handoff_id
             JOIN stage_run_units AS source_unit
               ON source_unit.id=source.source_stage_run_unit_id
              AND source_unit.operation_id=source.operation_id
              AND source_unit.stage_execution_id=source.stage_execution_id
              AND source_unit.organization_id=source.organization_id
              AND source_unit.stage_kind=source.from_stage_kind
            WHERE input.manifest_id=manifest.id
              AND (
                  source.invalidated_at IS NOT NULL
                  OR source_unit.status<>'passed'
                  OR source.operation_id<>manifest.operation_id
                  OR source.scope_snapshot_id<>manifest.scope_snapshot_id
                  OR source.organization_id<>manifest.organization_id
                  OR input.source_kind<>source.from_stage_kind
                  OR input.source_id<>source.id::TEXT
                  OR input.source_version<>source.schema_version
                  OR input.source_payload IS DISTINCT FROM source.payload
                  OR input.source_payload_hash<>
                        application_model_sha256_jsonb(input.source_payload)
                  OR ('sha256:' || source.payload_sha256)<>
                        application_model_sha256_jsonb(source.payload)
                  OR NOT input.evidence_ids <@ source.evidence_ids
              )
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_AUTHORITY_MISMATCH';
    END IF;

    IF manifest.authority_kind='model' THEN
        IF revision.status<>'final'
           OR revision.row_version<>1
           OR revision.finalized_at IS DISTINCT FROM handoff.gate_passed_at
           OR revision.model_hash<>expected_model_hash
           OR revision.replay_material_hash<>expected_replay_material_hash
           OR revision.source_submission_id<>submission.id
           OR submission.payload IS DISTINCT FROM jsonb_build_object(
               'stage_id', 'application_understanding',
               'stage_run_id', manifest.stage_execution_id,
               'schema_version', 1,
               'manifest_id', manifest.id,
               'structured_model', revision.structured_model,
               'decisions', revision_material -> 'decisions',
               'items', revision_material -> 'items'
           )
        THEN
            RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_REVISION_MISMATCH';
        END IF;
    ELSIF current_row.revision_id IS NOT NULL
       OR EXISTS (
           SELECT 1 FROM application_model_revisions
            WHERE manifest_id=manifest.id
       )
       OR submission.payload IS DISTINCT FROM jsonb_build_object(
           'stage_id', 'application_understanding',
           'stage_run_id', manifest.stage_execution_id,
           'schema_version', 1,
           'manifest_id', manifest.id,
           'authority_kind', 'terminal_no_input'
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_REVISION';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION application_model_validate_stage_handoff_bundle()
RETURNS trigger AS $$
BEGIN
    IF NEW.from_stage_kind = 'application_understanding'
       AND NOT EXISTS (
           SELECT 1
             FROM application_model_manifests AS manifest
             JOIN application_model_current_revisions AS current_revision
               ON current_revision.manifest_id = manifest.id
              AND current_revision.stage_handoff_id = NEW.id
              AND current_revision.deliverable_submission_id =
                    NEW.deliverable_submission_id
            WHERE manifest.operation_id = NEW.operation_id
              AND manifest.scope_snapshot_id = NEW.scope_snapshot_id
              AND manifest.stage_execution_id = NEW.stage_execution_id
              AND manifest.stage_run_unit_id = NEW.source_stage_run_unit_id
              AND manifest.organization_id = NEW.organization_id
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_HANDOFF_REQUIRES_CURRENT_POINTER';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_stage_handoff_bundle_exact
AFTER INSERT ON stage_handoffs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
WHEN (NEW.from_stage_kind = 'application_understanding')
EXECUTE FUNCTION application_model_validate_stage_handoff_bundle();

CREATE FUNCTION application_model_validate_passed_unit_bundle()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM application_model_manifests AS manifest
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.manifest_id=manifest.id
          JOIN stage_handoffs AS handoff
            ON handoff.id=current_revision.stage_handoff_id
           AND handoff.source_stage_run_unit_id=NEW.id
         WHERE manifest.stage_run_unit_id=NEW.id
           AND manifest.operation_id=NEW.operation_id
           AND manifest.scope_snapshot_id=NEW.scope_snapshot_id
           AND manifest.stage_execution_id=NEW.stage_execution_id
           AND manifest.organization_id=NEW.organization_id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_PASSED_UNIT_REQUIRES_EXACT_BUNDLE';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_passed_unit_bundle_exact
AFTER INSERT OR UPDATE OF status ON stage_run_units
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
WHEN (NEW.stage_kind='application_understanding' AND NEW.status='passed')
EXECUTE FUNCTION application_model_validate_passed_unit_bundle();

CREATE FUNCTION application_model_reject_active_unit_authority_change()
RETURNS trigger AS $$
BEGIN
    IF OLD.stage_kind='application_understanding'
       AND OLD.status='passed'
       AND EXISTS (
           SELECT 1
             FROM application_model_manifests AS manifest
             JOIN application_model_current_revisions AS current_revision
               ON current_revision.manifest_id=manifest.id
             JOIN stage_handoffs AS handoff
               ON handoff.id=current_revision.stage_handoff_id
              AND handoff.invalidated_at IS NULL
            WHERE manifest.stage_run_unit_id=OLD.id
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_UNIT_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_active_unit_authority_immutable
BEFORE UPDATE OR DELETE ON stage_run_units
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_unit_authority_change();

CREATE FUNCTION application_model_reject_active_worker_authority_change()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM stage_deliverable_submissions AS submission
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.deliverable_submission_id=submission.id
          JOIN stage_handoffs AS handoff
            ON handoff.id=current_revision.stage_handoff_id
           AND handoff.invalidated_at IS NULL
         WHERE submission.worker_run_id=OLD.id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_WORKER_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_active_worker_authority_immutable
BEFORE UPDATE OR DELETE ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_worker_authority_change();

CREATE FUNCTION application_model_reject_active_submit_tool_change()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM stage_deliverable_submissions AS submission
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.deliverable_submission_id=submission.id
          JOIN stage_handoffs AS handoff
            ON handoff.id=current_revision.stage_handoff_id
           AND handoff.invalidated_at IS NULL
         WHERE submission.tool_call_record_id=OLD.id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_SUBMIT_TOOL_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_active_submit_tool_authority_immutable
BEFORE UPDATE OR DELETE ON tool_calls
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_submit_tool_change();

CREATE FUNCTION application_model_reject_active_unit_child_change()
RETURNS trigger AS $$
DECLARE
    target_stage_run_unit_id UUID;
BEGIN
    IF TG_OP='DELETE' THEN
        target_stage_run_unit_id := OLD.stage_run_unit_id;
    ELSE
        target_stage_run_unit_id := NEW.stage_run_unit_id;
    END IF;
    IF target_stage_run_unit_id IS NOT NULL
       AND EXISTS (
           SELECT 1
             FROM application_model_manifests AS manifest
             JOIN application_model_current_revisions AS current_revision
               ON current_revision.manifest_id=manifest.id
             JOIN stage_handoffs AS handoff
               ON handoff.id=current_revision.stage_handoff_id
              AND handoff.invalidated_at IS NULL
            WHERE manifest.stage_run_unit_id=target_stage_run_unit_id
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_UNIT_CHILD_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_active_worker_set_immutable
BEFORE INSERT OR UPDATE OR DELETE ON stage_worker_runs
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_unit_child_change();

CREATE TRIGGER application_model_active_work_item_set_immutable
BEFORE INSERT OR UPDATE OR DELETE ON stage_work_items
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_unit_child_change();

CREATE TRIGGER application_model_active_tool_set_immutable
BEFORE INSERT OR UPDATE OR DELETE ON tool_calls
FOR EACH ROW EXECUTE FUNCTION application_model_reject_active_unit_child_change();

CREATE FUNCTION application_model_validate_predecessor_handoff_denominator()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM application_model_manifests AS manifest
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.manifest_id=manifest.id
          JOIN stage_handoffs AS application_handoff
            ON application_handoff.id=current_revision.stage_handoff_id
           AND application_handoff.invalidated_at IS NULL
         WHERE manifest.operation_id=NEW.operation_id
           AND manifest.scope_snapshot_id=NEW.scope_snapshot_id
           AND manifest.organization_id=NEW.organization_id
           AND NOT EXISTS (
               SELECT 1
                 FROM application_model_manifest_inputs AS input
                WHERE input.manifest_id=manifest.id
                  AND input.source_handoff_id=NEW.id
           )
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_PREDECESSOR_HANDOFF_OUTSIDE_FROZEN_DENOMINATOR';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_predecessor_handoff_denominator_exact
AFTER INSERT ON stage_handoffs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
WHEN (NEW.from_stage_kind IN (
    'target_intel',
    'external_attack_surface',
    'enumeration',
    'vuln_triage'
))
EXECUTE FUNCTION application_model_validate_predecessor_handoff_denominator();

CREATE FUNCTION application_model_validate_predecessor_source_change()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM stage_handoffs AS source_handoff
          JOIN application_model_manifest_inputs AS input
            ON input.source_handoff_id=source_handoff.id
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.manifest_id=input.manifest_id
          JOIN stage_handoffs AS application_handoff
            ON application_handoff.id=current_revision.stage_handoff_id
           AND application_handoff.invalidated_at IS NULL
         WHERE source_handoff.source_stage_run_unit_id=NEW.id
           AND (
               NEW.status<>'passed'
               OR source_handoff.invalidated_at IS NOT NULL
           )
    ) OR EXISTS (
        SELECT 1
          FROM stage_handoffs AS source_handoff
          JOIN application_model_manifests AS manifest
            ON manifest.operation_id=source_handoff.operation_id
           AND manifest.scope_snapshot_id=source_handoff.scope_snapshot_id
           AND manifest.organization_id=source_handoff.organization_id
          JOIN application_model_current_revisions AS current_revision
            ON current_revision.manifest_id=manifest.id
          JOIN stage_handoffs AS application_handoff
            ON application_handoff.id=current_revision.stage_handoff_id
           AND application_handoff.invalidated_at IS NULL
         WHERE source_handoff.source_stage_run_unit_id=NEW.id
           AND NEW.status='passed'
           AND NOT EXISTS (
               SELECT 1
                 FROM application_model_manifest_inputs AS input
                WHERE input.manifest_id=manifest.id
                  AND input.source_handoff_id=source_handoff.id
           )
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_PREDECESSOR_SOURCE_CHANGED';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_predecessor_source_unit_exact
AFTER UPDATE OF status ON stage_run_units
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
WHEN (
    NEW.stage_kind IN (
        'target_intel',
        'external_attack_surface',
        'enumeration',
        'vuln_triage'
    )
)
EXECUTE FUNCTION application_model_validate_predecessor_source_change();

CREATE FUNCTION application_model_validate_predecessor_invalidation()
RETURNS trigger AS $$
BEGIN
    IF NEW.invalidated_at IS NOT NULL
       AND EXISTS (
           SELECT 1
             FROM application_model_manifest_inputs AS input
             JOIN application_model_current_revisions AS current_revision
               ON current_revision.manifest_id=input.manifest_id
             JOIN stage_handoffs AS application_handoff
               ON application_handoff.id=current_revision.stage_handoff_id
              AND application_handoff.invalidated_at IS NULL
            WHERE input.source_handoff_id=NEW.id
       )
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ACTIVE_PREDECESSOR_INVALIDATED';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_predecessor_invalidation_exact
AFTER UPDATE OF invalidated_at ON stage_handoffs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
WHEN (
    OLD.invalidated_at IS NULL
    AND NEW.invalidated_at IS NOT NULL
    AND NEW.from_stage_kind IN (
        'target_intel',
        'external_attack_surface',
        'enumeration',
        'vuln_triage'
    )
)
EXECUTE FUNCTION application_model_validate_predecessor_invalidation();
