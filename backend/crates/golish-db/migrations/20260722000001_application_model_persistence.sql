-- Dormant Application Understanding persistence foundation.
--
-- This migration is additive only.  It deliberately does not register a new
-- StageKind, alter the operation graph, publish a StageHandoff, or make any
-- existing operation enter Application Understanding.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE FUNCTION application_model_canonical_jsonb(input_value JSONB)
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
                           application_model_canonical_jsonb(element.value),
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
                               application_model_canonical_jsonb(entry.value),
                           ',' ORDER BY entry.key
                       ),
                       ''
                   ) || '}'
              INTO rendered
              FROM jsonb_each(input_value) AS entry(key, value);
            RETURN rendered;
        ELSE
            RAISE EXCEPTION 'APPLICATION_MODEL_JSON_INVALID';
    END CASE;
END;
$$ LANGUAGE plpgsql IMMUTABLE STRICT;

CREATE FUNCTION application_model_sha256_jsonb(input_value JSONB)
RETURNS TEXT AS $$
    SELECT 'sha256:' || ENCODE(
        DIGEST(application_model_canonical_jsonb(input_value), 'sha256'),
        'hex'
    );
$$ LANGUAGE sql IMMUTABLE STRICT;

CREATE FUNCTION application_model_text_array_is_canonical(
    input_values TEXT[],
    allow_empty BOOLEAN
)
RETURNS BOOLEAN AS $$
    SELECT input_values IS NOT NULL
       AND (allow_empty OR cardinality(input_values) > 0)
       AND NOT EXISTS (
           SELECT 1
             FROM unnest(input_values) AS value
            WHERE value IS NULL
               OR btrim(value) = ''
               OR length(value) > 256
       )
       AND cardinality(input_values) = (
           SELECT count(DISTINCT value) FROM unnest(input_values) AS value
       )
       AND input_values = ARRAY(
           SELECT value FROM unnest(input_values) AS value ORDER BY value
       );
$$ LANGUAGE sql IMMUTABLE;

CREATE FUNCTION application_model_bigint_array_is_canonical(
    input_values BIGINT[]
)
RETURNS BOOLEAN AS $$
    SELECT input_values IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM unnest(input_values) AS value WHERE value <= 0
       )
       AND cardinality(input_values) = (
           SELECT count(DISTINCT value) FROM unnest(input_values) AS value
       )
       AND input_values = ARRAY(
           SELECT value FROM unnest(input_values) AS value ORDER BY value
       );
$$ LANGUAGE sql IMMUTABLE;

CREATE TABLE application_model_manifests (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL UNIQUE,
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (stage_kind = 'application_understanding'),
    authority_kind TEXT NOT NULL
        CHECK (authority_kind IN ('model', 'terminal_no_input')),
    input_count INTEGER NOT NULL CHECK (input_count >= 0 AND input_count <= 10000),
    manifest_hash TEXT NOT NULL
        CHECK (manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    replay_material_hash TEXT NOT NULL
        CHECK (replay_material_hash ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version = 0),
    frozen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (
        id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ),
    FOREIGN KEY (
        stage_run_unit_id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) REFERENCES stage_run_units(
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id) ON DELETE RESTRICT,
    CHECK (
        (authority_kind = 'model' AND input_count > 0)
        OR
        (authority_kind = 'terminal_no_input' AND input_count = 0)
    )
);

CREATE INDEX application_model_manifests_owner_lookup
    ON application_model_manifests(operation_id, organization_id, stage_execution_id);

CREATE TABLE application_model_manifest_inputs (
    manifest_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    input_key TEXT NOT NULL CHECK (length(btrim(input_key)) BETWEEN 1 AND 256),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    input_kind TEXT NOT NULL CHECK (length(btrim(input_kind)) BETWEEN 1 AND 64),
    source_handoff_id UUID NOT NULL REFERENCES stage_handoffs(id) ON DELETE RESTRICT,
    source_kind TEXT NOT NULL CHECK (length(btrim(source_kind)) BETWEEN 1 AND 64),
    source_id TEXT NOT NULL CHECK (length(btrim(source_id)) BETWEEN 1 AND 512),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    source_payload JSONB NOT NULL
        CHECK (jsonb_typeof(source_payload) IN ('object', 'array')),
    source_payload_hash TEXT NOT NULL
        CHECK (source_payload_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_ids BIGINT[] NOT NULL DEFAULT '{}'
        CHECK (application_model_bigint_array_is_canonical(evidence_ids)),
    PRIMARY KEY (manifest_id, input_key),
    UNIQUE (manifest_id, ordinal),
    FOREIGN KEY (
        manifest_id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES application_model_manifests(
        id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT
);

CREATE INDEX application_model_manifest_inputs_source_lookup
    ON application_model_manifest_inputs(source_handoff_id, manifest_id);

CREATE TABLE application_model_revisions (
    id UUID PRIMARY KEY,
    manifest_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    revision_ordinal INTEGER NOT NULL CHECK (revision_ordinal > 0),
    stage_kind TEXT NOT NULL CHECK (stage_kind = 'application_understanding'),
    schema_version TEXT NOT NULL CHECK (schema_version = 'application_model.v1'),
    status TEXT NOT NULL CHECK (status IN ('building', 'proposed', 'final')),
    structured_model JSONB NOT NULL CHECK (jsonb_typeof(structured_model) = 'object'),
    model_hash TEXT NOT NULL CHECK (model_hash ~ '^sha256:[0-9a-f]{64}$'),
    replay_material_hash TEXT NOT NULL
        CHECK (replay_material_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_submission_id UUID NOT NULL,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finalized_at TIMESTAMPTZ,
    UNIQUE (manifest_id, revision_ordinal),
    UNIQUE (manifest_id, id),
    UNIQUE (
        id,
        manifest_id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ),
    FOREIGN KEY (
        manifest_id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) REFERENCES application_model_manifests(
        id,
        operation_id,
        scope_snapshot_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_submission_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ) REFERENCES stage_deliverable_submissions(
        id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        organization_id,
        stage_kind
    ) ON DELETE RESTRICT,
    CHECK (
        (status IN ('building', 'proposed') AND row_version = 0 AND finalized_at IS NULL)
        OR
        (status = 'final' AND row_version = 1 AND finalized_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX application_model_revisions_one_final
    ON application_model_revisions(manifest_id)
    WHERE status = 'final';

CREATE TABLE application_model_input_decisions (
    revision_id UUID NOT NULL,
    manifest_id UUID NOT NULL,
    input_key TEXT NOT NULL,
    disposition TEXT NOT NULL
        CHECK (disposition IN ('incorporated', 'duplicate', 'not_relevant', 'unknown')),
    item_keys TEXT[] NOT NULL DEFAULT '{}',
    duplicate_input_key TEXT,
    reason_code TEXT,
    PRIMARY KEY (revision_id, input_key),
    FOREIGN KEY (revision_id, manifest_id)
        REFERENCES application_model_revisions(id, manifest_id) ON DELETE RESTRICT,
    FOREIGN KEY (manifest_id, input_key)
        REFERENCES application_model_manifest_inputs(manifest_id, input_key) ON DELETE RESTRICT,
    CHECK (application_model_text_array_is_canonical(item_keys, TRUE)),
    CHECK (
        (
            disposition = 'incorporated'
            AND cardinality(item_keys) > 0
            AND duplicate_input_key IS NULL
            AND reason_code IS NULL
        )
        OR
        (
            disposition = 'duplicate'
            AND cardinality(item_keys) = 0
            AND duplicate_input_key IS NOT NULL
            AND btrim(duplicate_input_key) <> ''
            AND duplicate_input_key <> input_key
            AND reason_code IS NULL
        )
        OR
        (
            disposition IN ('not_relevant', 'unknown')
            AND cardinality(item_keys) = 0
            AND duplicate_input_key IS NULL
            AND reason_code ~ '^[a-z0-9_]{1,64}$'
        )
    )
);

CREATE TABLE application_model_items (
    revision_id UUID NOT NULL,
    manifest_id UUID NOT NULL,
    item_key TEXT NOT NULL CHECK (length(btrim(item_key)) BETWEEN 1 AND 256),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    item_kind TEXT NOT NULL CHECK (length(btrim(item_kind)) BETWEEN 1 AND 64),
    truth_state TEXT NOT NULL CHECK (truth_state IN ('observed', 'inferred', 'unknown')),
    source_input_keys TEXT[] NOT NULL,
    referenced_item_keys TEXT[] NOT NULL DEFAULT '{}',
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY (revision_id, item_key),
    UNIQUE (revision_id, ordinal),
    FOREIGN KEY (revision_id, manifest_id)
        REFERENCES application_model_revisions(id, manifest_id) ON DELETE RESTRICT,
    CHECK (application_model_text_array_is_canonical(source_input_keys, FALSE)),
    CHECK (application_model_text_array_is_canonical(referenced_item_keys, TRUE)),
    CHECK (NOT item_key = ANY(referenced_item_keys)),
    CHECK (application_model_sha256_jsonb(payload) = payload_hash)
);

CREATE TABLE application_model_item_evidence (
    revision_id UUID NOT NULL,
    manifest_id UUID NOT NULL,
    item_key TEXT NOT NULL,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('observation', 'support')),
    PRIMARY KEY (revision_id, item_key, evidence_id, role),
    UNIQUE (revision_id, item_key, evidence_id),
    FOREIGN KEY (revision_id, manifest_id)
        REFERENCES application_model_revisions(id, manifest_id) ON DELETE RESTRICT,
    FOREIGN KEY (revision_id, item_key)
        REFERENCES application_model_items(revision_id, item_key) ON DELETE RESTRICT
);

CREATE INDEX application_model_item_evidence_evidence_lookup
    ON application_model_item_evidence(evidence_id, revision_id);

CREATE TABLE application_model_current_revisions (
    manifest_id UUID PRIMARY KEY REFERENCES application_model_manifests(id) ON DELETE RESTRICT,
    revision_id UUID UNIQUE,
    authority_kind TEXT NOT NULL
        CHECK (authority_kind IN ('model', 'terminal_no_input')),
    stage_handoff_id UUID NOT NULL UNIQUE REFERENCES stage_handoffs(id) ON DELETE RESTRICT,
    deliverable_submission_id UUID NOT NULL UNIQUE
        REFERENCES stage_deliverable_submissions(id) ON DELETE RESTRICT,
    manifest_hash TEXT NOT NULL CHECK (manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    model_hash TEXT CHECK (model_hash IS NULL OR model_hash ~ '^sha256:[0-9a-f]{64}$'),
    replay_material_hash TEXT NOT NULL
        CHECK (replay_material_hash ~ '^sha256:[0-9a-f]{64}$'),
    gate_decision_hash TEXT NOT NULL
        CHECK (gate_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (manifest_id, revision_id)
        REFERENCES application_model_revisions(manifest_id, id)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (authority_kind = 'model' AND revision_id IS NOT NULL AND model_hash IS NOT NULL)
        OR
        (authority_kind = 'terminal_no_input' AND revision_id IS NULL AND model_hash IS NULL)
    )
);

CREATE FUNCTION application_model_reject_immutable_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'APPLICATION_MODEL_FROZEN_ROW_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_manifests_immutable
BEFORE UPDATE OR DELETE ON application_model_manifests
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TRIGGER application_model_manifest_inputs_immutable
BEFORE UPDATE OR DELETE ON application_model_manifest_inputs
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TRIGGER application_model_input_decisions_immutable
BEFORE UPDATE OR DELETE ON application_model_input_decisions
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TRIGGER application_model_items_immutable
BEFORE UPDATE OR DELETE ON application_model_items
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TRIGGER application_model_item_evidence_immutable
BEFORE UPDATE OR DELETE ON application_model_item_evidence
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TRIGGER application_model_current_revisions_immutable
BEFORE UPDATE OR DELETE ON application_model_current_revisions
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE FUNCTION application_model_validate_manifest_input_source()
RETURNS trigger AS $$
DECLARE
    manifest application_model_manifests%ROWTYPE;
    handoff stage_handoffs%ROWTYPE;
    source_unit_status TEXT;
BEGIN
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id = NEW.manifest_id
     FOR SHARE;
    IF manifest.authority_kind <> 'model' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_INPUTS';
    END IF;
    IF ROW(
        NEW.operation_id,
        NEW.scope_snapshot_id,
        NEW.stage_execution_id,
        NEW.stage_run_unit_id,
        NEW.organization_id
    ) IS DISTINCT FROM ROW(
        manifest.operation_id,
        manifest.scope_snapshot_id,
        manifest.stage_execution_id,
        manifest.stage_run_unit_id,
        manifest.organization_id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_INPUT_OWNER_MISMATCH';
    END IF;
    SELECT * INTO STRICT handoff
      FROM stage_handoffs
     WHERE id = NEW.source_handoff_id
     FOR SHARE;
    SELECT status INTO STRICT source_unit_status
      FROM stage_run_units
     WHERE id = handoff.source_stage_run_unit_id
       AND operation_id = handoff.operation_id
       AND stage_execution_id = handoff.stage_execution_id
       AND organization_id = handoff.organization_id
       AND stage_kind = handoff.from_stage_kind
     FOR SHARE;
    IF handoff.operation_id <> manifest.operation_id
       OR handoff.scope_snapshot_id <> manifest.scope_snapshot_id
       OR handoff.organization_id <> manifest.organization_id
       OR handoff.invalidated_at IS NOT NULL
       OR source_unit_status <> 'passed'
       OR NEW.source_kind <> handoff.from_stage_kind
       OR NEW.source_id <> handoff.id::TEXT
       OR NEW.source_version <> handoff.schema_version
       OR ('sha256:' || handoff.payload_sha256) <> NEW.source_payload_hash
       OR NOT NEW.evidence_ids <@ handoff.evidence_ids
       OR application_model_sha256_jsonb(NEW.source_payload) <> NEW.source_payload_hash
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_INPUT_SOURCE_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_manifest_input_source_authority
BEFORE INSERT ON application_model_manifest_inputs
FOR EACH ROW EXECUTE FUNCTION application_model_validate_manifest_input_source();

CREATE FUNCTION application_model_validate_manifest_complete()
RETURNS trigger AS $$
DECLARE
    target_manifest_id UUID;
    manifest application_model_manifests%ROWTYPE;
    persisted_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'application_model_manifests' THEN
        target_manifest_id := NEW.id;
    ELSE
        target_manifest_id := NEW.manifest_id;
    END IF;
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id = target_manifest_id;
    SELECT count(*) INTO persisted_count
      FROM application_model_manifest_inputs
     WHERE manifest_id = target_manifest_id;
    IF persisted_count <> manifest.input_count THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_MANIFEST_INPUT_COUNT_MISMATCH';
    END IF;
    IF manifest.authority_kind = 'terminal_no_input' AND persisted_count <> 0 THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_INPUTS';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_manifest_header_complete
AFTER INSERT ON application_model_manifests
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_manifest_complete();

CREATE CONSTRAINT TRIGGER application_model_manifest_input_complete
AFTER INSERT ON application_model_manifest_inputs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_manifest_complete();

CREATE FUNCTION application_model_validate_revision_complete()
RETURNS trigger AS $$
DECLARE
    target_revision_id UUID;
    revision application_model_revisions%ROWTYPE;
    manifest application_model_manifests%ROWTYPE;
    decision_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'application_model_revisions' THEN
        target_revision_id := NEW.id;
    ELSE
        target_revision_id := NEW.revision_id;
    END IF;
    SELECT * INTO STRICT revision
      FROM application_model_revisions
     WHERE id = target_revision_id;
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id = revision.manifest_id;
    IF revision.status = 'building' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_BUILD_NOT_SEALED';
    END IF;
    IF manifest.authority_kind <> 'model' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_REVISION';
    END IF;
    SELECT count(*) INTO decision_count
      FROM application_model_input_decisions
     WHERE revision_id = target_revision_id;
    IF decision_count <> manifest.input_count THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_DECISION_COUNT_MISMATCH';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM application_model_input_decisions AS decision
          CROSS JOIN LATERAL unnest(decision.item_keys) AS decision_item(item_key)
          LEFT JOIN application_model_items AS item
            ON item.revision_id = decision.revision_id
           AND item.item_key = decision_item.item_key
         WHERE decision.revision_id = target_revision_id
           AND (
               item.item_key IS NULL
               OR NOT decision.input_key = ANY(item.source_input_keys)
           )
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_DECISION_ITEM_SOURCE_MISMATCH';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM application_model_items AS item
         WHERE item.revision_id = target_revision_id
           AND (
               EXISTS (
                   SELECT 1
                     FROM unnest(item.source_input_keys) AS source_input(input_key)
                     LEFT JOIN application_model_manifest_inputs AS manifest_input
                       ON manifest_input.manifest_id = item.manifest_id
                      AND manifest_input.input_key = source_input.input_key
                    WHERE manifest_input.input_key IS NULL
               )
               OR EXISTS (
                   SELECT 1
                     FROM unnest(item.referenced_item_keys) AS item_reference(item_key)
                     LEFT JOIN application_model_items AS referenced_item
                       ON referenced_item.revision_id = item.revision_id
                      AND referenced_item.item_key = item_reference.item_key
                    WHERE referenced_item.item_key IS NULL
               )
               OR NOT EXISTS (
                   SELECT 1
                     FROM application_model_input_decisions AS decision
                    WHERE decision.revision_id = item.revision_id
                      AND decision.disposition = 'incorporated'
                      AND item.item_key = ANY(decision.item_keys)
               )
           )
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ITEM_CLOSURE_MISMATCH';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM application_model_items AS item
         WHERE item.revision_id = target_revision_id
           AND (
               (
                   item.truth_state = 'observed'
                   AND NOT EXISTS (
                       SELECT 1
                         FROM application_model_item_evidence AS evidence
                        WHERE evidence.revision_id = item.revision_id
                          AND evidence.item_key = item.item_key
                          AND evidence.role = 'observation'
                   )
               )
               OR
               (
                   item.truth_state IN ('inferred', 'unknown')
                   AND EXISTS (
                       SELECT 1
                         FROM application_model_item_evidence AS evidence
                        WHERE evidence.revision_id = item.revision_id
                          AND evidence.item_key = item.item_key
                          AND evidence.role = 'observation'
                   )
               )
           )
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ITEM_EVIDENCE_ROLE_MISMATCH';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_revision_header_complete
AFTER INSERT ON application_model_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_revision_complete();

CREATE CONSTRAINT TRIGGER application_model_revision_decision_complete
AFTER INSERT ON application_model_input_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_revision_complete();

CREATE CONSTRAINT TRIGGER application_model_revision_item_complete
AFTER INSERT ON application_model_items
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_revision_complete();

CREATE CONSTRAINT TRIGGER application_model_revision_evidence_complete
AFTER INSERT ON application_model_item_evidence
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_revision_complete();

CREATE FUNCTION application_model_restrict_revision_change()
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
    IF OLD.status = 'proposed' AND NEW.status = 'final' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_DORMANT_PUBLICATION_DISABLED';
    END IF;
    RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_revisions_transition_only
BEFORE UPDATE OR DELETE ON application_model_revisions
FOR EACH ROW EXECUTE FUNCTION application_model_restrict_revision_change();

CREATE FUNCTION application_model_require_building_revision()
RETURNS trigger AS $$
DECLARE
    revision_status TEXT;
BEGIN
    SELECT status INTO STRICT revision_status
      FROM application_model_revisions
     WHERE id = NEW.revision_id
     FOR UPDATE;
    IF revision_status <> 'building' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_CHILDREN_FROZEN';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_input_decisions_building_only
BEFORE INSERT ON application_model_input_decisions
FOR EACH ROW EXECUTE FUNCTION application_model_require_building_revision();

CREATE TRIGGER application_model_items_building_only
BEFORE INSERT ON application_model_items
FOR EACH ROW EXECUTE FUNCTION application_model_require_building_revision();

CREATE FUNCTION application_model_validate_item_evidence_insert()
RETURNS trigger AS $$
DECLARE
    revision_status TEXT;
BEGIN
    SELECT status INTO STRICT revision_status
      FROM application_model_revisions
     WHERE id = NEW.revision_id
     FOR UPDATE;
    IF revision_status <> 'building' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_CHILDREN_FROZEN';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM application_model_items AS item
          CROSS JOIN LATERAL unnest(item.source_input_keys) AS source_input(input_key)
          JOIN application_model_manifest_inputs AS manifest_input
            ON manifest_input.manifest_id = item.manifest_id
           AND manifest_input.input_key = source_input.input_key
         WHERE item.revision_id = NEW.revision_id
           AND item.item_key = NEW.item_key
           AND NEW.evidence_id = ANY(manifest_input.evidence_ids)
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_ITEM_EVIDENCE_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_item_evidence_building_authority
BEFORE INSERT ON application_model_item_evidence
FOR EACH ROW EXECUTE FUNCTION application_model_validate_item_evidence_insert();

CREATE FUNCTION application_model_reject_dormant_publication()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'APPLICATION_MODEL_DORMANT_PUBLICATION_DISABLED';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER application_model_current_revisions_dormant
BEFORE INSERT ON application_model_current_revisions
FOR EACH ROW EXECUTE FUNCTION application_model_reject_dormant_publication();

CREATE TRIGGER application_model_stage_handoffs_dormant
BEFORE INSERT ON stage_handoffs
FOR EACH ROW
WHEN (NEW.from_stage_kind = 'application_understanding')
EXECUTE FUNCTION application_model_reject_dormant_publication();

CREATE FUNCTION application_model_validate_current_revision()
RETURNS trigger AS $$
DECLARE
    current_row application_model_current_revisions%ROWTYPE;
    manifest application_model_manifests%ROWTYPE;
    revision application_model_revisions%ROWTYPE;
    handoff stage_handoffs%ROWTYPE;
BEGIN
    SELECT * INTO STRICT current_row
      FROM application_model_current_revisions
     WHERE manifest_id = NEW.manifest_id;
    SELECT * INTO STRICT manifest
      FROM application_model_manifests
     WHERE id = current_row.manifest_id;
    SELECT * INTO STRICT handoff
      FROM stage_handoffs
     WHERE id = current_row.stage_handoff_id;
    IF current_row.authority_kind <> manifest.authority_kind
       OR current_row.manifest_hash <> manifest.manifest_hash
       OR handoff.operation_id <> manifest.operation_id
       OR handoff.scope_snapshot_id <> manifest.scope_snapshot_id
       OR handoff.organization_id <> manifest.organization_id
       OR handoff.stage_execution_id <> manifest.stage_execution_id
       OR handoff.source_stage_run_unit_id <> manifest.stage_run_unit_id
       OR handoff.deliverable_submission_id <> current_row.deliverable_submission_id
       OR handoff.invalidated_at IS NOT NULL
    THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_AUTHORITY_MISMATCH';
    END IF;
    IF manifest.authority_kind = 'model' THEN
        SELECT * INTO STRICT revision
          FROM application_model_revisions
         WHERE id = current_row.revision_id
           AND manifest_id = manifest.id;
        IF revision.status <> 'final'
           OR revision.model_hash <> current_row.model_hash
           OR revision.replay_material_hash <> current_row.replay_material_hash
           OR revision.source_submission_id <> current_row.deliverable_submission_id
        THEN
            RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_REVISION_MISMATCH';
        END IF;
    ELSIF current_row.replay_material_hash <> manifest.replay_material_hash THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CURRENT_AUTHORITY_MISMATCH';
    ELSIF EXISTS (
        SELECT 1 FROM application_model_revisions WHERE manifest_id = manifest.id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_TERMINAL_NO_INPUT_HAS_REVISION';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_current_revision_exact
AFTER INSERT ON application_model_current_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_current_revision();

CREATE FUNCTION application_model_validate_final_revision_pointer()
RETURNS trigger AS $$
BEGIN
    IF NEW.status = 'final' AND NOT EXISTS (
        SELECT 1
          FROM application_model_current_revisions AS current_revision
         WHERE current_revision.manifest_id = NEW.manifest_id
           AND current_revision.revision_id = NEW.id
    ) THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_FINAL_REVISION_REQUIRES_CURRENT_POINTER';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER application_model_final_revision_has_pointer
AFTER UPDATE OF status ON application_model_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION application_model_validate_final_revision_pointer();
