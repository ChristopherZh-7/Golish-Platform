-- Structured Temporal Graph C3.
--
-- Local V2 graph projection is deliberately separate from legacy
-- graph_entities/graph_relations. Every active identity is supported by one or
-- more immutable knowledge_assertions through temporal lineage rows. Rebuilds
-- write a new generation and only become visible after an atomic cutover.

CREATE TABLE knowledge_graph_generations (
    generation_id UUID PRIMARY KEY,
    scope_key TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (
        visibility IN ('organization_long_term','global_sanitized')
    ),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    status TEXT NOT NULL CHECK (status IN ('building','active','retired','failed')),
    build_hash TEXT CHECK (build_hash IS NULL OR build_hash ~ '^[0-9a-f]{64}$'),
    entity_count BIGINT CHECK (entity_count IS NULL OR entity_count >= 0),
    relation_count BIGINT CHECK (relation_count IS NULL OR relation_count >= 0),
    failure_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    activated_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    UNIQUE (generation_id, scope_key),
    CHECK (
        (visibility = 'organization_long_term'
            AND project_scope_id IS NOT NULL
            AND organization_id_at_time IS NOT NULL
            AND scope_key = 'org:' || project_scope_id::text || ':' || organization_id_at_time::text)
        OR
        (visibility = 'global_sanitized'
            AND project_scope_id IS NULL
            AND organization_id_at_time IS NULL
            AND scope_key = 'global_sanitized')
    ),
    CHECK (
        (status = 'building'
            AND activated_at IS NULL AND completed_at IS NULL
            AND build_hash IS NULL AND entity_count IS NULL
            AND relation_count IS NULL AND failure_reason IS NULL)
        OR
        (status = 'active'
            AND activated_at IS NOT NULL AND completed_at IS NOT NULL
            AND build_hash IS NOT NULL AND entity_count IS NOT NULL
            AND relation_count IS NOT NULL AND failure_reason IS NULL)
        OR
        (status = 'retired'
            AND activated_at IS NOT NULL AND completed_at IS NOT NULL
            AND build_hash IS NOT NULL AND entity_count IS NOT NULL
            AND relation_count IS NOT NULL AND failure_reason IS NULL)
        OR
        (status = 'failed'
            AND activated_at IS NULL AND completed_at IS NOT NULL
            AND failure_reason IS NOT NULL AND length(btrim(failure_reason)) > 0)
    )
);

CREATE UNIQUE INDEX knowledge_graph_one_active_generation
    ON knowledge_graph_generations(scope_key, projection_schema_version)
    WHERE status = 'active';

-- Only one rebuild may own a scope/schema at a time. This prevents an older,
-- slower generation from activating after a newer rebuild and reversing the
-- cutover order.
CREATE UNIQUE INDEX knowledge_graph_one_building_generation
    ON knowledge_graph_generations(scope_key, projection_schema_version)
    WHERE status = 'building';

CREATE TABLE knowledge_graph_entities (
    entity_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL,
    scope_key TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (
        visibility IN ('organization_long_term','global_sanitized')
    ),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    canonical_ref TEXT NOT NULL CHECK (length(btrim(canonical_ref)) > 0),
    identity_hash TEXT NOT NULL CHECK (identity_hash ~ '^[0-9a-f]{64}$'),
    entity_type TEXT NOT NULL CHECK (entity_type IN (
        'organization','target','host','service','endpoint',
        'vulnerability','finding','technique'
    )),
    display_name TEXT NOT NULL CHECK (length(btrim(display_name)) > 0),
    properties JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(properties) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (generation_id, scope_key)
        REFERENCES knowledge_graph_generations(generation_id, scope_key) ON DELETE CASCADE,
    UNIQUE (entity_id, generation_id, scope_key),
    UNIQUE (entity_id, generation_id),
    UNIQUE (generation_id, canonical_ref),
    UNIQUE (generation_id, identity_hash),
    CHECK (
        (visibility = 'organization_long_term'
            AND project_scope_id IS NOT NULL
            AND organization_id_at_time IS NOT NULL
            AND scope_key = 'org:' || project_scope_id::text || ':' || organization_id_at_time::text)
        OR
        (visibility = 'global_sanitized'
            AND project_scope_id IS NULL
            AND organization_id_at_time IS NULL
            AND scope_key = 'global_sanitized'
            AND entity_type = 'technique')
    )
);

CREATE TABLE knowledge_graph_entity_assertions (
    entity_id UUID NOT NULL REFERENCES knowledge_graph_entities(entity_id) ON DELETE CASCADE,
    generation_id UUID NOT NULL,
    assertion_id UUID NOT NULL REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    status TEXT NOT NULL CHECK (
        status IN ('active','superseded','refuted','expired')
    ),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    fresh_until TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (
        classification IN ('public','internal','customer_confidential','restricted')
    ),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (0 < ALL(evidence_refs)),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    PRIMARY KEY (entity_id, assertion_id, projection_schema_version),
    UNIQUE (generation_id, entity_id, assertion_id, projection_schema_version),
    FOREIGN KEY (entity_id, generation_id)
        REFERENCES knowledge_graph_entities(entity_id, generation_id) ON DELETE CASCADE
);

CREATE TABLE knowledge_graph_relations (
    relation_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL,
    scope_key TEXT NOT NULL,
    from_entity_id UUID NOT NULL,
    to_entity_id UUID NOT NULL,
    relation_type TEXT NOT NULL CHECK (relation_type IN (
        'contains','resolves_to','runs_service','exposes_endpoint',
        'has_vulnerability','supported_by_finding','associated_technique'
    )),
    identity_hash TEXT NOT NULL CHECK (identity_hash ~ '^[0-9a-f]{64}$'),
    properties JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(properties) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (from_entity_id <> to_entity_id),
    FOREIGN KEY (generation_id, scope_key)
        REFERENCES knowledge_graph_generations(generation_id, scope_key) ON DELETE CASCADE,
    FOREIGN KEY (from_entity_id, generation_id, scope_key)
        REFERENCES knowledge_graph_entities(entity_id, generation_id, scope_key) ON DELETE RESTRICT,
    FOREIGN KEY (to_entity_id, generation_id, scope_key)
        REFERENCES knowledge_graph_entities(entity_id, generation_id, scope_key) ON DELETE RESTRICT,
    UNIQUE (relation_id, generation_id),
    UNIQUE (generation_id, from_entity_id, to_entity_id, relation_type),
    UNIQUE (generation_id, identity_hash)
);

CREATE TABLE knowledge_graph_relation_assertions (
    relation_id UUID NOT NULL REFERENCES knowledge_graph_relations(relation_id) ON DELETE CASCADE,
    generation_id UUID NOT NULL,
    assertion_id UUID NOT NULL REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL CHECK (length(btrim(source_stream_key)) > 0),
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    status TEXT NOT NULL CHECK (
        status IN ('active','superseded','refuted','expired')
    ),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    fresh_until TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (
        classification IN ('public','internal','customer_confidential','restricted')
    ),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (0 < ALL(evidence_refs)),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    PRIMARY KEY (relation_id, assertion_id, projection_schema_version),
    UNIQUE (generation_id, relation_id, assertion_id, projection_schema_version),
    FOREIGN KEY (relation_id, generation_id)
        REFERENCES knowledge_graph_relations(relation_id, generation_id) ON DELETE CASCADE
);

CREATE FUNCTION validate_graph_entity_assertion_scope()
RETURNS trigger AS $$
DECLARE
    entity_row knowledge_graph_entities%ROWTYPE;
    assertion_row knowledge_assertions%ROWTYPE;
BEGIN
    SELECT * INTO STRICT entity_row
      FROM knowledge_graph_entities WHERE entity_id = NEW.entity_id;
    SELECT * INTO STRICT assertion_row
      FROM knowledge_assertions WHERE assertion_id = NEW.assertion_id;
    IF entity_row.generation_id IS DISTINCT FROM NEW.generation_id
       OR entity_row.visibility IS DISTINCT FROM assertion_row.visibility
       OR entity_row.project_scope_id IS DISTINCT FROM assertion_row.project_scope_id
       OR entity_row.organization_id_at_time IS DISTINCT FROM assertion_row.organization_id_at_time
       OR NEW.source_stream_key IS DISTINCT FROM assertion_row.source_stream_key
       OR NEW.source_version IS DISTINCT FROM assertion_row.source_version
       OR NEW.evidence_refs IS DISTINCT FROM assertion_row.evidence_refs
       OR NEW.status IS DISTINCT FROM assertion_row.status
       OR NEW.valid_from IS DISTINCT FROM assertion_row.valid_from
       OR NEW.valid_to IS DISTINCT FROM assertion_row.valid_to
       OR NEW.fresh_until IS DISTINCT FROM assertion_row.fresh_until
       OR NEW.classification IS DISTINCT FROM assertion_row.classification
       OR NEW.projection_schema_version IS DISTINCT FROM (
           SELECT projection_schema_version
           FROM knowledge_graph_generations
           WHERE generation_id = NEW.generation_id
       )
    THEN
        RAISE EXCEPTION 'graph entity assertion scope/lineage mismatch';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER knowledge_graph_entity_assertion_scope_guard
BEFORE INSERT OR UPDATE ON knowledge_graph_entity_assertions
FOR EACH ROW EXECUTE FUNCTION validate_graph_entity_assertion_scope();

CREATE FUNCTION validate_graph_relation_assertion_scope()
RETURNS trigger AS $$
DECLARE
    relation_scope_key TEXT;
    relation_generation_id UUID;
    generation_row knowledge_graph_generations%ROWTYPE;
    assertion_row knowledge_assertions%ROWTYPE;
BEGIN
    SELECT scope_key, generation_id
      INTO STRICT relation_scope_key, relation_generation_id
      FROM knowledge_graph_relations WHERE relation_id = NEW.relation_id;
    SELECT * INTO STRICT generation_row
      FROM knowledge_graph_generations WHERE generation_id = relation_generation_id;
    SELECT * INTO STRICT assertion_row
      FROM knowledge_assertions WHERE assertion_id = NEW.assertion_id;
    IF relation_generation_id IS DISTINCT FROM NEW.generation_id
       OR generation_row.visibility IS DISTINCT FROM assertion_row.visibility
       OR generation_row.project_scope_id IS DISTINCT FROM assertion_row.project_scope_id
       OR generation_row.organization_id_at_time IS DISTINCT FROM assertion_row.organization_id_at_time
       OR NEW.source_stream_key IS DISTINCT FROM assertion_row.source_stream_key
       OR NEW.source_version IS DISTINCT FROM assertion_row.source_version
       OR NEW.evidence_refs IS DISTINCT FROM assertion_row.evidence_refs
       OR NEW.status IS DISTINCT FROM assertion_row.status
       OR NEW.valid_from IS DISTINCT FROM assertion_row.valid_from
       OR NEW.valid_to IS DISTINCT FROM assertion_row.valid_to
       OR NEW.fresh_until IS DISTINCT FROM assertion_row.fresh_until
       OR NEW.classification IS DISTINCT FROM assertion_row.classification
       OR NEW.projection_schema_version IS DISTINCT FROM generation_row.projection_schema_version
    THEN
        RAISE EXCEPTION 'graph relation assertion scope/lineage mismatch';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER knowledge_graph_relation_assertion_scope_guard
BEFORE INSERT OR UPDATE ON knowledge_graph_relation_assertions
FOR EACH ROW EXECUTE FUNCTION validate_graph_relation_assertion_scope();

CREATE INDEX knowledge_graph_entity_active_lineage
    ON knowledge_graph_entity_assertions(entity_id, status, valid_from, valid_to);
CREATE INDEX knowledge_graph_relation_active_lineage
    ON knowledge_graph_relation_assertions(relation_id, status, valid_from, valid_to);
CREATE INDEX knowledge_graph_entity_source_version
    ON knowledge_graph_entity_assertions(generation_id, source_stream_key, projection_schema_version, source_version DESC);
CREATE INDEX knowledge_graph_relation_source_version
    ON knowledge_graph_relation_assertions(generation_id, source_stream_key, projection_schema_version, source_version DESC);

CREATE FUNCTION compute_knowledge_graph_generation_hash(target_generation_id UUID)
RETURNS TEXT AS $$
DECLARE
    manifest TEXT;
BEGIN
    SELECT string_agg(item, E'\n' ORDER BY item) INTO manifest
    FROM (
        SELECT 'entity|' || scope_key || '|' || canonical_ref
               || '|' || identity_hash || '|' || entity_type || '|' || display_name
               || '|' || properties::text AS item
        FROM knowledge_graph_entities
        WHERE generation_id = target_generation_id
        UNION ALL
        SELECT 'entity_lineage|' || entity.canonical_ref || '|' || lineage.assertion_id::text
               || '|' || lineage.source_stream_key || '|' || lineage.source_version::text
               || '|' || array_to_string(lineage.evidence_refs, ',') || '|' || lineage.status
               || '|' || lineage.valid_from::text || '|' || COALESCE(lineage.valid_to::text, 'null')
               || '|' || COALESCE(lineage.fresh_until::text, 'null') || '|' || lineage.classification
               || '|' || lineage.projection_schema_version::text AS item
        FROM knowledge_graph_entity_assertions lineage
        JOIN knowledge_graph_entities entity ON entity.entity_id = lineage.entity_id
        WHERE lineage.generation_id = target_generation_id
        UNION ALL
        SELECT 'relation|' || relation.scope_key || '|' || source.canonical_ref
               || '|' || target.canonical_ref || '|' || relation.relation_type
               || '|' || relation.identity_hash || '|' || relation.properties::text AS item
        FROM knowledge_graph_relations relation
        JOIN knowledge_graph_entities source ON source.entity_id = relation.from_entity_id
        JOIN knowledge_graph_entities target ON target.entity_id = relation.to_entity_id
        WHERE relation.generation_id = target_generation_id
        UNION ALL
        SELECT 'relation_lineage|' || source.canonical_ref || '|' || relation.relation_type
               || '|' || target.canonical_ref || '|' || lineage.assertion_id::text
               || '|' || lineage.source_stream_key || '|' || lineage.source_version::text
               || '|' || array_to_string(lineage.evidence_refs, ',') || '|' || lineage.status
               || '|' || lineage.valid_from::text || '|' || COALESCE(lineage.valid_to::text, 'null')
               || '|' || COALESCE(lineage.fresh_until::text, 'null') || '|' || lineage.classification
               || '|' || lineage.projection_schema_version::text AS item
        FROM knowledge_graph_relation_assertions lineage
        JOIN knowledge_graph_relations relation ON relation.relation_id = lineage.relation_id
        JOIN knowledge_graph_entities source ON source.entity_id = relation.from_entity_id
        JOIN knowledge_graph_entities target ON target.entity_id = relation.to_entity_id
        WHERE lineage.generation_id = target_generation_id
    ) manifest_rows;
    RETURN encode(sha256(convert_to(COALESCE(manifest, ''), 'UTF8')), 'hex');
END;
$$ LANGUAGE plpgsql STABLE;

CREATE FUNCTION refresh_active_knowledge_graph_generation()
RETURNS trigger AS $$
DECLARE
    target_generation_id UUID;
BEGIN
    target_generation_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.generation_id ELSE NEW.generation_id END;
    UPDATE knowledge_graph_generations generation
       SET build_hash = compute_knowledge_graph_generation_hash(target_generation_id),
           entity_count = (
               SELECT COUNT(*) FROM knowledge_graph_entities
               WHERE generation_id = target_generation_id
           ),
           relation_count = (
               SELECT COUNT(*) FROM knowledge_graph_relations
               WHERE generation_id = target_generation_id
           ),
           completed_at = NOW()
     WHERE generation.generation_id = target_generation_id
       AND generation.status = 'active';
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER knowledge_graph_entities_refresh_attestation
AFTER INSERT OR UPDATE OR DELETE ON knowledge_graph_entities
FOR EACH ROW EXECUTE FUNCTION refresh_active_knowledge_graph_generation();
CREATE TRIGGER knowledge_graph_entity_assertions_refresh_attestation
AFTER INSERT OR UPDATE OR DELETE ON knowledge_graph_entity_assertions
FOR EACH ROW EXECUTE FUNCTION refresh_active_knowledge_graph_generation();
CREATE TRIGGER knowledge_graph_relations_refresh_attestation
AFTER INSERT OR UPDATE OR DELETE ON knowledge_graph_relations
FOR EACH ROW EXECUTE FUNCTION refresh_active_knowledge_graph_generation();
CREATE TRIGGER knowledge_graph_relation_assertions_refresh_attestation
AFTER INSERT OR UPDATE OR DELETE ON knowledge_graph_relation_assertions
FOR EACH ROW EXECUTE FUNCTION refresh_active_knowledge_graph_generation();

INSERT INTO knowledge_projector_registry (
    projector_name,
    projector_schema_version,
    lifecycle
) VALUES ('graph-projector', 1, 'paused')
ON CONFLICT (projector_name, projector_schema_version) DO NOTHING;
