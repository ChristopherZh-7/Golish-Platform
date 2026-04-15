-- ============================================================================
-- KB Research Log: Persists AI research conversations per CVE.
-- Completely separate from home/terminal conversation history.
-- ============================================================================

CREATE TABLE kb_research_log (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cve_id      TEXT NOT NULL UNIQUE,
    session_id  TEXT NOT NULL,
    turns       JSONB NOT NULL DEFAULT '[]',
    status      TEXT NOT NULL DEFAULT 'in_progress',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN kb_research_log.turns IS 'JSON array of {text, toolCalls} objects representing completed conversation turns';
COMMENT ON COLUMN kb_research_log.status IS 'in_progress, completed, error';

CREATE INDEX idx_kb_research_log_cve ON kb_research_log(cve_id);
