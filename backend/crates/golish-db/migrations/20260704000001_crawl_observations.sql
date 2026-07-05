-- ============================================================================
-- Crawl observations: source-origin-owned crawler output
-- ============================================================================
-- Additive display/audit table. These rows retain URLs emitted by crawler-class
-- tools under the Web Origin / target that was crawled. They are intentionally
-- separate from api_endpoints, so third-party or dead links do not become scoped
-- targets and do not affect enumeration coverage truth.
-- ============================================================================

CREATE TABLE IF NOT EXISTS crawl_observations (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),

    origin_target_id  UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    organization_id   UUID REFERENCES organizations(id) ON DELETE SET NULL,
    project_path      TEXT NOT NULL DEFAULT '',

    origin_url        TEXT NOT NULL,
    origin_key        TEXT NOT NULL,
    observed_url      TEXT NOT NULL,
    observed_host     TEXT,
    observed_path     TEXT,

    kind              TEXT NOT NULL DEFAULT 'url',
    same_origin       BOOLEAN NOT NULL DEFAULT FALSE,
    source_tool       TEXT NOT NULL DEFAULT 'crawler',
    source_record_id  TEXT,
    evidence_id       BIGINT REFERENCES audit_log(id) ON DELETE SET NULL,
    metadata          JSONB NOT NULL DEFAULT '{}',

    discovered_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT crawl_observations_origin_url_not_empty CHECK (btrim(origin_url) <> ''),
    CONSTRAINT crawl_observations_origin_key_not_empty CHECK (btrim(origin_key) <> ''),
    CONSTRAINT crawl_observations_observed_url_not_empty CHECK (btrim(observed_url) <> ''),
    CONSTRAINT crawl_observations_kind_not_empty CHECK (btrim(kind) <> ''),
    CONSTRAINT crawl_observations_source_tool_not_empty CHECK (btrim(source_tool) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_crawl_observations_origin_url_tool_kind
ON crawl_observations(origin_target_id, observed_url, source_tool, kind);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_origin_target
ON crawl_observations(origin_target_id);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_org
ON crawl_observations(organization_id);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_project
ON crawl_observations(project_path);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_origin_key
ON crawl_observations(origin_key);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_observed_host
ON crawl_observations(observed_host);

CREATE INDEX IF NOT EXISTS idx_crawl_observations_discovered_at
ON crawl_observations(discovered_at DESC);

COMMENT ON TABLE crawl_observations IS
'Crawler URL observations owned by the crawled origin/target. Not target promotion and not enumeration gate truth.';
