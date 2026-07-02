-- ============================================================================
-- Surface identity: first-class NetworkEndpoint / WebOrigin model
-- ============================================================================
-- Phase 2.1 scope only:
-- - Add new identity tables.
-- - Do not alter legacy api_endpoints / js_analysis_results / directory_entries
--   / passive_scan_logs / fingerprints / findings tables.
-- - WebOrigin <-> NetworkEndpoint is many-to-many through observations.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- network_endpoints: observed IP:port service identity
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS network_endpoints (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id    UUID REFERENCES organizations(id) ON DELETE CASCADE,
    project_path       TEXT NOT NULL DEFAULT '',

    ip                 TEXT NOT NULL,
    port               INTEGER NOT NULL,
    transport          TEXT NOT NULL DEFAULT 'tcp',
    state              TEXT NOT NULL DEFAULT 'unknown',

    service_name       TEXT,
    service_product    TEXT,
    service_version    TEXT,
    banner             TEXT,
    tls_detected       BOOLEAN NOT NULL DEFAULT FALSE,

    first_seen_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_confirmed_at  TIMESTAMPTZ,

    source             TEXT NOT NULL DEFAULT 'unknown',
    confidence         REAL NOT NULL DEFAULT 0.5,

    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT network_endpoints_ip_not_empty CHECK (btrim(ip) <> ''),
    CONSTRAINT network_endpoints_transport_check CHECK (transport IN ('tcp', 'udp', 'unknown')),
    CONSTRAINT network_endpoints_state_check CHECK (state IN ('open', 'closed', 'filtered', 'unknown')),
    CONSTRAINT network_endpoints_port_check CHECK (port > 0 AND port <= 65535),
    CONSTRAINT network_endpoints_confidence_check CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_network_endpoints_org_ip_transport_port
ON network_endpoints (organization_id, ip, transport, port)
WHERE organization_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_network_endpoints_project_ip_transport_port
ON network_endpoints (project_path, ip, transport, port)
WHERE organization_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_network_endpoints_org
ON network_endpoints(organization_id);

CREATE INDEX IF NOT EXISTS idx_network_endpoints_project
ON network_endpoints(project_path);

CREATE INDEX IF NOT EXISTS idx_network_endpoints_ip
ON network_endpoints(ip);

-- ---------------------------------------------------------------------------
-- web_origins: normalized scheme://host:port identity
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS web_origins (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id    UUID REFERENCES organizations(id) ON DELETE CASCADE,
    project_path       TEXT NOT NULL DEFAULT '',

    scheme             TEXT NOT NULL DEFAULT 'unknown',
    host               TEXT NOT NULL,
    host_type          TEXT NOT NULL DEFAULT 'unknown',
    port               INTEGER NOT NULL,
    origin             TEXT NOT NULL,

    first_seen_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_confirmed_at  TIMESTAMPTZ,

    source             TEXT NOT NULL DEFAULT 'unknown',
    confidence         REAL NOT NULL DEFAULT 0.5,

    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT web_origins_scheme_check CHECK (scheme IN ('http', 'https', 'unknown')),
    CONSTRAINT web_origins_host_not_empty CHECK (btrim(host) <> ''),
    CONSTRAINT web_origins_host_type_check CHECK (host_type IN ('domain', 'ip', 'unknown')),
    CONSTRAINT web_origins_port_check CHECK (port > 0 AND port <= 65535),
    CONSTRAINT web_origins_confidence_check CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_web_origins_org_scheme_host_port
ON web_origins (organization_id, scheme, host, port)
WHERE organization_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_web_origins_project_scheme_host_port
ON web_origins (project_path, scheme, host, port)
WHERE organization_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_web_origins_org
ON web_origins(organization_id);

CREATE INDEX IF NOT EXISTS idx_web_origins_project
ON web_origins(project_path);

CREATE INDEX IF NOT EXISTS idx_web_origins_host
ON web_origins(host);

CREATE INDEX IF NOT EXISTS idx_web_origins_origin
ON web_origins(origin);

-- ---------------------------------------------------------------------------
-- web_origin_observations: many-to-many WebOrigin <-> NetworkEndpoint evidence
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS web_origin_observations (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id      UUID REFERENCES organizations(id) ON DELETE CASCADE,
    project_path         TEXT NOT NULL DEFAULT '',

    web_origin_id        UUID NOT NULL REFERENCES web_origins(id) ON DELETE CASCADE,
    network_endpoint_id  UUID REFERENCES network_endpoints(id) ON DELETE SET NULL,
    target_id            UUID REFERENCES targets(id) ON DELETE SET NULL,

    observed_ip          TEXT,
    sni                  TEXT,
    host_header          TEXT,
    status_code          INTEGER,
    title                TEXT,
    final_url            TEXT,
    redirect_chain       JSONB NOT NULL DEFAULT '[]',
    body_hash            TEXT,
    favicon_hash         TEXT,
    screenshot_path      TEXT,
    capture_path         TEXT,

    observed_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confidence           REAL NOT NULL DEFAULT 0.5,
    source               TEXT NOT NULL DEFAULT 'unknown',
    raw                  JSONB NOT NULL DEFAULT '{}',

    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT web_origin_observations_confidence_check CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE INDEX IF NOT EXISTS idx_woo_org
ON web_origin_observations(organization_id);

CREATE INDEX IF NOT EXISTS idx_woo_project
ON web_origin_observations(project_path);

CREATE INDEX IF NOT EXISTS idx_woo_origin
ON web_origin_observations(web_origin_id);

CREATE INDEX IF NOT EXISTS idx_woo_endpoint
ON web_origin_observations(network_endpoint_id);

CREATE INDEX IF NOT EXISTS idx_woo_target
ON web_origin_observations(target_id);

CREATE INDEX IF NOT EXISTS idx_woo_observed_at
ON web_origin_observations(observed_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS uq_woo_dedupe_capture
ON web_origin_observations(web_origin_id, network_endpoint_id, source, capture_path)
WHERE network_endpoint_id IS NOT NULL AND capture_path IS NOT NULL AND capture_path <> '';
