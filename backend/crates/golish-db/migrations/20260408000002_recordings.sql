-- ============================================================================
-- Add recordings table for terminal session recordings
-- Migrated from JSON file storage
-- ============================================================================

CREATE TABLE recordings (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    width       SMALLINT NOT NULL DEFAULT 80,
    height      SMALLINT NOT NULL DEFAULT 24,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    event_count INTEGER NOT NULL DEFAULT 0,
    events      JSONB NOT NULL DEFAULT '[]',
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_recordings_session ON recordings(session_id);
CREATE INDEX idx_recordings_project ON recordings(project_path);
CREATE INDEX idx_recordings_created ON recordings(created_at DESC);
