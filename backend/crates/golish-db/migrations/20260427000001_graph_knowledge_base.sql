-- ============================================================================
-- Graph Knowledge Base: PostgreSQL-backed entity/relation graph
-- Stores typed security entities (host, service, vulnerability, ...) and the
-- relations between them so agents can reason about attack paths and lateral
-- movement. The (entity_type, name, project_id) triple uniquely identifies an
-- entity inside a project, enabling idempotent UPSERT semantics.
-- ============================================================================

CREATE TABLE graph_entities (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    entity_type  VARCHAR(50) NOT NULL,
    name         VARCHAR(500) NOT NULL,
    properties   JSONB NOT NULL DEFAULT '{}',
    session_id   UUID,
    project_id   VARCHAR(100),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT graph_entities_type_check CHECK (
        entity_type IN ('host', 'service', 'vulnerability', 'credential', 'technique', 'endpoint')
    )
);

COMMENT ON TABLE graph_entities IS 'Typed nodes of the security knowledge graph (host, service, vulnerability, ...).';
COMMENT ON COLUMN graph_entities.entity_type IS 'One of: host, service, vulnerability, credential, technique, endpoint';
COMMENT ON COLUMN graph_entities.properties  IS 'Arbitrary JSON payload (e.g. ip, port, banner, cvss). Merged on UPSERT.';
COMMENT ON COLUMN graph_entities.project_id  IS 'Optional project scope; entities with the same (type, name) are deduped per project.';

-- UPSERT key: dedupe on (entity_type, name, project_id). NULLs are coalesced
-- to a sentinel so two NULL project_ids collide instead of being treated as
-- distinct (which is the default Postgres unique-index behavior).
CREATE UNIQUE INDEX idx_graph_entities_unique
    ON graph_entities (entity_type, name, COALESCE(project_id, ''));

CREATE INDEX idx_graph_entities_type       ON graph_entities (entity_type);
CREATE INDEX idx_graph_entities_session    ON graph_entities (session_id);
CREATE INDEX idx_graph_entities_project    ON graph_entities (project_id);
CREATE INDEX idx_graph_entities_updated    ON graph_entities (updated_at DESC);
CREATE INDEX idx_graph_entities_name_trgm  ON graph_entities (name);

-- ============================================================================
-- Relations: directed, typed edges between entities.
-- (from_entity_id, to_entity_id, relation_type) is unique so re-discovering an
-- edge updates its properties rather than creating duplicates.
-- ============================================================================

CREATE TABLE graph_relations (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    from_entity_id  UUID NOT NULL REFERENCES graph_entities(id) ON DELETE CASCADE,
    to_entity_id    UUID NOT NULL REFERENCES graph_entities(id) ON DELETE CASCADE,
    relation_type   VARCHAR(100) NOT NULL,
    properties      JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE graph_relations IS 'Directed typed edges (runs_service, has_vulnerability, exploited_by, lateral_move, ...).';

CREATE UNIQUE INDEX idx_graph_relations_unique
    ON graph_relations (from_entity_id, to_entity_id, relation_type);

CREATE INDEX idx_graph_relations_from  ON graph_relations (from_entity_id);
CREATE INDEX idx_graph_relations_to    ON graph_relations (to_entity_id);
CREATE INDEX idx_graph_relations_type  ON graph_relations (relation_type);
