-- ============================================================================
-- Security Analysis Module: Structured storage for recon / analysis data
-- ============================================================================

-- ---------------------------------------------------------------------------
-- target_assets: Discovered sub-domains, IPs, services for a target
-- ---------------------------------------------------------------------------
CREATE TABLE target_assets (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id       UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path    TEXT,
    asset_type      TEXT NOT NULL DEFAULT 'subdomain',
    value           TEXT NOT NULL,
    port            INTEGER,
    protocol        TEXT,
    service         TEXT,
    version         TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    discovered_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(target_id, asset_type, value)
);

COMMENT ON COLUMN target_assets.asset_type IS 'subdomain, ip, service, cdn, waf, cloud_resource';
COMMENT ON COLUMN target_assets.status     IS 'active, inactive, unresolved';

CREATE INDEX idx_ta_target  ON target_assets(target_id);
CREATE INDEX idx_ta_type    ON target_assets(asset_type);
CREATE INDEX idx_ta_project ON target_assets(project_path);

-- ---------------------------------------------------------------------------
-- api_endpoints: Discovered API routes + parameters
-- ---------------------------------------------------------------------------
CREATE TABLE api_endpoints (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id       UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path    TEXT,
    url             TEXT NOT NULL,
    method          TEXT NOT NULL DEFAULT 'GET',
    path            TEXT NOT NULL DEFAULT '/',
    params          JSONB NOT NULL DEFAULT '[]',
    headers         JSONB NOT NULL DEFAULT '{}',
    auth_type       TEXT,
    response_type   TEXT,
    status_code     INTEGER,
    notes           TEXT NOT NULL DEFAULT '',
    source          TEXT NOT NULL DEFAULT 'manual',
    risk_level      TEXT NOT NULL DEFAULT 'unknown',
    tested          BOOLEAN NOT NULL DEFAULT FALSE,
    discovered_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN api_endpoints.source     IS 'manual, js_analysis, proxy, crawler, ai';
COMMENT ON COLUMN api_endpoints.risk_level IS 'unknown, low, medium, high, critical';

CREATE INDEX idx_api_target  ON api_endpoints(target_id);
CREATE INDEX idx_api_project ON api_endpoints(project_path);
CREATE INDEX idx_api_tested  ON api_endpoints(tested);

-- ---------------------------------------------------------------------------
-- js_analysis_results: JavaScript file analysis (frameworks, secrets, endpoints)
-- ---------------------------------------------------------------------------
CREATE TABLE js_analysis_results (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id       UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path    TEXT,
    url             TEXT NOT NULL,
    filename        TEXT NOT NULL DEFAULT '',
    size_bytes      BIGINT,
    hash_sha256     TEXT,
    frameworks      JSONB NOT NULL DEFAULT '[]',
    libraries       JSONB NOT NULL DEFAULT '[]',
    endpoints_found JSONB NOT NULL DEFAULT '[]',
    secrets_found   JSONB NOT NULL DEFAULT '[]',
    comments        JSONB NOT NULL DEFAULT '[]',
    source_maps     BOOLEAN NOT NULL DEFAULT FALSE,
    risk_summary    TEXT NOT NULL DEFAULT '',
    raw_analysis    JSONB NOT NULL DEFAULT '{}',
    analyzed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN js_analysis_results.frameworks    IS 'Detected JS frameworks: [{name, version, confidence}]';
COMMENT ON COLUMN js_analysis_results.endpoints_found IS 'API endpoints found in JS: [{url, method, context}]';
COMMENT ON COLUMN js_analysis_results.secrets_found IS 'Potential secrets: [{type, value_preview, line, context}]';

CREATE INDEX idx_jsa_target  ON js_analysis_results(target_id);
CREATE INDEX idx_jsa_project ON js_analysis_results(project_path);

-- ---------------------------------------------------------------------------
-- fingerprints: Technology fingerprints (web server, CMS, WAF, etc.)
-- ---------------------------------------------------------------------------
CREATE TABLE fingerprints (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id       UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path    TEXT,
    category        TEXT NOT NULL DEFAULT 'technology',
    name            TEXT NOT NULL,
    version         TEXT,
    confidence      REAL NOT NULL DEFAULT 0.5,
    evidence        JSONB NOT NULL DEFAULT '[]',
    cpe             TEXT,
    source          TEXT NOT NULL DEFAULT 'manual',
    detected_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(target_id, category, name)
);

COMMENT ON COLUMN fingerprints.category   IS 'technology, framework, cms, waf, cdn, os, server, language';
COMMENT ON COLUMN fingerprints.confidence IS '0.0-1.0 detection confidence';
COMMENT ON COLUMN fingerprints.cpe        IS 'Common Platform Enumeration string, e.g. cpe:2.3:a:apache:httpd:2.4.51';

CREATE INDEX idx_fp_target   ON fingerprints(target_id);
CREATE INDEX idx_fp_category ON fingerprints(category);
CREATE INDEX idx_fp_project  ON fingerprints(project_path);

-- ---------------------------------------------------------------------------
-- passive_scan_logs: Records of passive / manual security tests
-- ---------------------------------------------------------------------------
CREATE TABLE passive_scan_logs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id       UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path    TEXT,
    test_type       TEXT NOT NULL,
    payload         TEXT NOT NULL DEFAULT '',
    url             TEXT NOT NULL DEFAULT '',
    parameter       TEXT NOT NULL DEFAULT '',
    result          TEXT NOT NULL DEFAULT 'pending',
    evidence        TEXT NOT NULL DEFAULT '',
    severity        TEXT NOT NULL DEFAULT 'info',
    tool_used       TEXT NOT NULL DEFAULT '',
    tester          TEXT NOT NULL DEFAULT 'manual',
    notes           TEXT NOT NULL DEFAULT '',
    detail          JSONB NOT NULL DEFAULT '{}',
    tested_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN passive_scan_logs.test_type IS 'xss, sqli, cmd_injection, ssrf, idor, auth_bypass, lfi, rfi, xxe, open_redirect, cors, csrf, info_leak, etc.';
COMMENT ON COLUMN passive_scan_logs.result    IS 'vulnerable, not_vulnerable, potential, error, pending';
COMMENT ON COLUMN passive_scan_logs.tester    IS 'manual, ai, scanner_name';

CREATE INDEX idx_psl_target   ON passive_scan_logs(target_id);
CREATE INDEX idx_psl_type     ON passive_scan_logs(test_type);
CREATE INDEX idx_psl_result   ON passive_scan_logs(result);
CREATE INDEX idx_psl_project  ON passive_scan_logs(project_path);
CREATE INDEX idx_psl_tested   ON passive_scan_logs(tested_at DESC);
