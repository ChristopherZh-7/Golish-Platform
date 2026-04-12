-- Scan queue (ZAP active scan endpoints) and custom passive rules
-- Previously stored in localStorage, now migrated to PostgreSQL

CREATE TABLE scan_queue (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    url          TEXT NOT NULL,
    scan_id      TEXT,
    progress     INTEGER NOT NULL DEFAULT 0,
    status       TEXT NOT NULL DEFAULT 'queued',
    alerts       JSONB NOT NULL DEFAULT '[]',
    added_at     BIGINT NOT NULL DEFAULT 0,
    project_path TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_scan_queue_project ON scan_queue(project_path);
CREATE INDEX idx_scan_queue_status ON scan_queue(status);

CREATE TABLE custom_passive_rules (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    pattern      TEXT NOT NULL,
    scope        TEXT NOT NULL DEFAULT 'all',
    severity     TEXT NOT NULL DEFAULT 'medium',
    enabled      BOOLEAN NOT NULL DEFAULT true,
    project_path TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_custom_rules_project ON custom_passive_rules(project_path);
