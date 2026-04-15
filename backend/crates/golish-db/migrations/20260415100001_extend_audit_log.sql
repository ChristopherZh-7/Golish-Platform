-- ============================================================================
-- Extend audit_log to support pentest operation logging
-- Merges the operation_logs concept into the existing audit trail.
-- ============================================================================

ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS target_id    UUID REFERENCES targets(id) ON DELETE SET NULL;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS session_id   TEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS tool_name    TEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS status       TEXT NOT NULL DEFAULT 'completed';
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS detail       JSONB NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_audit_target    ON audit_log(target_id);
CREATE INDEX IF NOT EXISTS idx_audit_session   ON audit_log(session_id);
CREATE INDEX IF NOT EXISTS idx_audit_tool      ON audit_log(tool_name);
CREATE INDEX IF NOT EXISTS idx_audit_status    ON audit_log(status);
CREATE INDEX IF NOT EXISTS idx_audit_detail    ON audit_log USING GIN(detail);
