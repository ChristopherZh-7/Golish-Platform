-- Memory Fabric C1: historical canonical episodes/assertions, deterministic
-- documents, fixed-dimension embeddings, and immutable per-projector outbox.
--
-- This is an additive migration. Historical rows intentionally retain
-- organization/target ids as at-time provenance without foreign keys to live
-- entities. Only the stable project_scopes registry is referenced.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE stage_episodes (
    episode_id UUID PRIMARY KEY,
    project_scope_id UUID NOT NULL
        REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    source_operation_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    source_scope_snapshot_hash TEXT NOT NULL CHECK (length(btrim(source_scope_snapshot_hash)) > 0),
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (length(btrim(stage_kind)) > 0),
    stage_run_unit_id_at_time UUID,
    worker_run_id_at_time UUID,
    candidate_attempt_id_at_time UUID,
    wave INTEGER,
    verdict TEXT NOT NULL CHECK (
        verdict IN ('passed','blocked','exhausted','failed','superseded')
    ),
    deliverable_submission_id_at_time UUID,
    handoff_id_at_time UUID,
    reason_codes JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(reason_codes) = 'array'),
    fact_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(fact_refs) = 'array'),
    evidence_refs BIGINT[] NOT NULL DEFAULT '{}',
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL CHECK (ended_at >= started_at),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (0 < ALL(evidence_refs))
);

CREATE TABLE knowledge_assertions (
    assertion_id UUID PRIMARY KEY,
    visibility TEXT NOT NULL CHECK (
        visibility IN ('organization_long_term','global_sanitized')
    ),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    source_operation_id UUID NOT NULL,
    source_scope_snapshot_hash TEXT NOT NULL CHECK (length(btrim(source_scope_snapshot_hash)) > 0),
    source_kind TEXT NOT NULL CHECK (length(btrim(source_kind)) > 0),
    source_id_kind TEXT NOT NULL CHECK (source_id_kind IN ('uuid','int64','text')),
    source_id_value TEXT NOT NULL CHECK (length(source_id_value) BETWEEN 1 AND 512),
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    subject_key TEXT NOT NULL CHECK (length(btrim(subject_key)) > 0),
    predicate TEXT NOT NULL CHECK (length(btrim(predicate)) > 0),
    object_hash TEXT NOT NULL CHECK (object_hash ~ '^[0-9a-f]{64}$'),
    assertion_identity_hash TEXT NOT NULL CHECK (assertion_identity_hash ~ '^[0-9a-f]{64}$'),
    object_kind TEXT NOT NULL CHECK (object_kind IN ('json','vault_ref')),
    object_value JSONB,
    vault_ref UUID,
    assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
        'observation','checked_empty','verified_outcome','refuted_outcome',
        'technique_experience','cleanup_attestation','residual_risk'
    )),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','refuted','expired')),
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    fresh_until TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (
        classification IN ('public','internal','customer_confidential','restricted')
    ),
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (source_id_kind = 'uuid' AND source_id_value ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
        OR (source_id_kind = 'int64' AND source_id_value ~ '^(0|-?[1-9][0-9]*)$')
        OR (source_id_kind = 'text' AND source_id_value = btrim(source_id_value))
    ),
    CHECK (
        (object_kind = 'json' AND object_value IS NOT NULL AND vault_ref IS NULL)
        OR (object_kind = 'vault_ref' AND object_value IS NULL AND vault_ref IS NOT NULL)
    ),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    CHECK (assertion_kind <> 'checked_empty' OR fresh_until IS NOT NULL),
    CHECK (
        (visibility = 'organization_long_term'
            AND project_scope_id IS NOT NULL
            AND organization_id_at_time IS NOT NULL)
        OR
        (visibility = 'global_sanitized'
            AND project_scope_id IS NULL
            AND organization_id_at_time IS NULL
            AND vault_ref IS NULL
            AND assertion_kind = 'technique_experience'
            AND classification IN ('public','internal'))
    ),
    CHECK (0 < ALL(evidence_refs))
);

CREATE UNIQUE INDEX knowledge_assertions_source_identity
    ON knowledge_assertions (
        project_scope_id,
        source_stream_key,
        source_version,
        subject_key,
        predicate,
        object_hash
    ) NULLS NOT DISTINCT;

CREATE TABLE knowledge_documents (
    document_id UUID PRIMARY KEY,
    document_key TEXT NOT NULL UNIQUE CHECK (document_key ~ '^[0-9a-f]{64}$'),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version = 1),
    redaction_policy_version INTEGER NOT NULL CHECK (redaction_policy_version > 0),
    assertion_ids UUID[] NOT NULL CHECK (cardinality(assertion_ids) > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','invalidated')),
    document_type TEXT NOT NULL CHECK (length(btrim(document_type)) > 0),
    redacted_content TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    classification TEXT NOT NULL CHECK (
        classification IN ('public','internal','customer_confidential','restricted')
    ),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_to IS NULL OR valid_to >= valid_from)
);

CREATE UNIQUE INDEX knowledge_documents_projection_key
    ON knowledge_documents (
        project_scope_id,
        source_stream_key,
        source_version,
        redaction_policy_version
    ) NULLS NOT DISTINCT;

CREATE TABLE knowledge_embeddings (
    embedding_id UUID PRIMARY KEY,
    document_id UUID NOT NULL REFERENCES knowledge_documents(document_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','invalidated')),
    provider TEXT NOT NULL CHECK (length(btrim(provider)) > 0),
    model TEXT NOT NULL CHECK (length(btrim(model)) > 0),
    embedding VECTOR(1536) NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension = 1536),
    embedding_schema_version INTEGER NOT NULL CHECK (embedding_schema_version = 1),
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    UNIQUE (
        document_id,
        provider,
        model,
        embedding_schema_version,
        content_hash
    )
);

CREATE TABLE knowledge_projector_registry (
    projector_name TEXT NOT NULL,
    projector_schema_version INTEGER NOT NULL CHECK (projector_schema_version > 0),
    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('enabled','paused','disabled')),
    disabled_reason TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (projector_name, projector_schema_version),
    CHECK (lifecycle <> 'disabled' OR length(btrim(disabled_reason)) > 0)
);

INSERT INTO knowledge_projector_registry (
    projector_name,
    projector_schema_version,
    lifecycle
) VALUES
    ('assertion-promoter', 1, 'paused'),
    ('document-projector', 1, 'paused'),
    ('embedding-projector', 1, 'paused'),
    ('report-artifact-indexer', 1, 'paused');

CREATE TABLE knowledge_outbox_events (
    event_id UUID PRIMARY KEY,
    event_name TEXT NOT NULL CHECK (length(btrim(event_name)) > 0),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    source_operation_id UUID NOT NULL,
    source_kind TEXT NOT NULL CHECK (length(btrim(source_kind)) > 0),
    source_id_kind TEXT NOT NULL CHECK (source_id_kind IN ('uuid','int64','text')),
    source_id_value TEXT NOT NULL CHECK (length(source_id_value) BETWEEN 1 AND 512),
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    occurred_at TIMESTAMPTZ NOT NULL,
    dedupe_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (source_id_kind = 'uuid' AND source_id_value ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
        OR (source_id_kind = 'int64' AND source_id_value ~ '^(0|-?[1-9][0-9]*)$')
        OR (source_id_kind = 'text' AND source_id_value = btrim(source_id_value))
    )
);

CREATE UNIQUE INDEX knowledge_outbox_source_version_identity
    ON knowledge_outbox_events (
        project_scope_id,
        event_name,
        schema_version,
        source_stream_key,
        source_version,
        source_id_kind,
        source_id_value
    ) NULLS NOT DISTINCT;

CREATE TABLE knowledge_projection_deliveries (
    event_id UUID NOT NULL REFERENCES knowledge_outbox_events(event_id) ON DELETE RESTRICT,
    projector_name TEXT NOT NULL,
    projector_schema_version INTEGER NOT NULL CHECK (projector_schema_version > 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'blocked_dependency','pending','leased','succeeded',
        'succeeded_suppressed','retryable_failed','stale','dead_letter'
    )),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    depends_on_projector TEXT,
    depends_on_schema_version INTEGER,
    terminal_reason TEXT,
    last_error TEXT,
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, projector_name, projector_schema_version),
    FOREIGN KEY (projector_name, projector_schema_version)
        REFERENCES knowledge_projector_registry(projector_name, projector_schema_version)
        ON DELETE RESTRICT,
    FOREIGN KEY (event_id, depends_on_projector, depends_on_schema_version)
        REFERENCES knowledge_projection_deliveries(event_id, projector_name, projector_schema_version)
        ON DELETE RESTRICT,
    CHECK (
        (status = 'leased' AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR status <> 'leased'
    ),
    CHECK (
        (status = 'succeeded_suppressed' AND length(btrim(terminal_reason)) > 0)
        OR status <> 'succeeded_suppressed'
    ),
    CHECK (
        (depends_on_projector IS NULL) = (depends_on_schema_version IS NULL)
    ),
    CHECK (
        status <> 'blocked_dependency' OR depends_on_projector IS NOT NULL
    )
);

CREATE FUNCTION reject_knowledge_outbox_event_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'knowledge_outbox_events are immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER knowledge_outbox_events_immutable
BEFORE UPDATE OR DELETE ON knowledge_outbox_events
FOR EACH ROW EXECUTE FUNCTION reject_knowledge_outbox_event_mutation();

CREATE INDEX stage_episodes_operation_stage
    ON stage_episodes(source_operation_id, organization_id_at_time, stage_kind, ended_at DESC);
CREATE INDEX knowledge_assertions_org_active
    ON knowledge_assertions(project_scope_id, organization_id_at_time, status, fresh_until)
    WHERE visibility = 'organization_long_term';
CREATE INDEX knowledge_documents_active_scope
    ON knowledge_documents(project_scope_id, status, source_stream_key, source_version DESC);
CREATE INDEX knowledge_embeddings_active_document
    ON knowledge_embeddings(document_id, status, embedding_schema_version);
CREATE INDEX knowledge_deliveries_claim
    ON knowledge_projection_deliveries(
        projector_name,
        projector_schema_version,
        status,
        available_at
    );
