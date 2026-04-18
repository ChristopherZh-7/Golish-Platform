-- Sensitive file scanner: track scanned directories and results

CREATE TABLE IF NOT EXISTS sensitive_scan_results (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    base_url       TEXT NOT NULL,
    probe_path     TEXT NOT NULL,
    full_url       TEXT NOT NULL,
    status_code    INTEGER NOT NULL,
    content_length INTEGER NOT NULL DEFAULT 0,
    content_type   TEXT NOT NULL DEFAULT '',
    is_confirmed   BOOLEAN NOT NULL DEFAULT FALSE,
    ai_verdict     TEXT,
    wordlist_id    TEXT,
    project_path   TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sensitive_project ON sensitive_scan_results(project_path);
CREATE INDEX IF NOT EXISTS idx_sensitive_base ON sensitive_scan_results(base_url);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sensitive_dedup ON sensitive_scan_results(full_url, project_path);

-- Track which directory+wordlist combos have been fully scanned
CREATE TABLE IF NOT EXISTS sensitive_scan_history (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    base_url       TEXT NOT NULL,
    wordlist_id    TEXT NOT NULL,
    probe_count    INTEGER NOT NULL DEFAULT 0,
    hit_count      INTEGER NOT NULL DEFAULT 0,
    project_path   TEXT,
    scanned_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_scan_hist_dedup ON sensitive_scan_history(base_url, wordlist_id, project_path);
