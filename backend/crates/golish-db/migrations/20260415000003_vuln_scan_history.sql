-- ============================================================================
-- Vuln scan history: tracks scan results per CVE + target
-- ============================================================================

CREATE TABLE vuln_scan_history (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cve_id      TEXT NOT NULL,
    target      TEXT NOT NULL,
    result      TEXT NOT NULL DEFAULT 'pending',
    details     TEXT,
    scanned_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN vuln_scan_history.result IS 'vulnerable, not_vulnerable, error, pending';

CREATE INDEX idx_vuln_scan_history_cve ON vuln_scan_history(cve_id);
CREATE INDEX idx_vuln_scan_history_time ON vuln_scan_history(scanned_at DESC);
