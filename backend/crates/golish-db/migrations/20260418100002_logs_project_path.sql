-- ============================================================================
-- Add project_path to session-linked log tables for per-project filtering.
-- Backfill from the sessions table where possible.
-- ============================================================================

-- ── 1. Add project_path column ──────────────────────────────────────────

ALTER TABLE agent_logs
  ADD COLUMN IF NOT EXISTS project_path TEXT NOT NULL DEFAULT '';

ALTER TABLE terminal_logs
  ADD COLUMN IF NOT EXISTS project_path TEXT NOT NULL DEFAULT '';

ALTER TABLE search_logs
  ADD COLUMN IF NOT EXISTS project_path TEXT NOT NULL DEFAULT '';

-- ── 2. Backfill from sessions ───────────────────────────────────────────

UPDATE agent_logs a
  SET project_path = COALESCE(s.project_path, '')
  FROM sessions s
  WHERE a.session_id = s.id AND a.project_path = '';

UPDATE terminal_logs t
  SET project_path = COALESCE(s.project_path, '')
  FROM sessions s
  WHERE t.session_id = s.id AND t.project_path = '';

UPDATE search_logs sl
  SET project_path = COALESCE(s.project_path, '')
  FROM sessions s
  WHERE sl.session_id = s.id AND sl.project_path = '';

-- ── 3. Index for fast project filtering ─────────────────────────────────

CREATE INDEX IF NOT EXISTS idx_agent_logs_project ON agent_logs(project_path);
CREATE INDEX IF NOT EXISTS idx_terminal_logs_project ON terminal_logs(project_path);
CREATE INDEX IF NOT EXISTS idx_search_logs_project ON search_logs(project_path);

-- ── 4. Normalize remaining tables' project_path ─────────────────────────

UPDATE vault_entries SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE vault_entries ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE vault_entries ALTER COLUMN project_path SET NOT NULL;

UPDATE notes SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE notes ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE notes ALTER COLUMN project_path SET NOT NULL;

UPDATE pipelines SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE pipelines ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE pipelines ALTER COLUMN project_path SET NOT NULL;

UPDATE methodology_projects SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE methodology_projects ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE methodology_projects ALTER COLUMN project_path SET NOT NULL;

UPDATE scan_queue SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE scan_queue ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE scan_queue ALTER COLUMN project_path SET NOT NULL;

UPDATE custom_passive_rules SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE custom_passive_rules ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE custom_passive_rules ALTER COLUMN project_path SET NOT NULL;

UPDATE sitemap_store SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE sitemap_store ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE sitemap_store ALTER COLUMN project_path SET NOT NULL;

UPDATE sensitive_scan_results SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE sensitive_scan_results ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE sensitive_scan_results ALTER COLUMN project_path SET NOT NULL;

UPDATE sensitive_scan_history SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE sensitive_scan_history ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE sensitive_scan_history ALTER COLUMN project_path SET NOT NULL;

UPDATE audit_log SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE audit_log ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE audit_log ALTER COLUMN project_path SET NOT NULL;

-- ── 5. Add target_id UUID FK to findings ────────────────────────────────

ALTER TABLE findings ADD COLUMN IF NOT EXISTS target_id UUID REFERENCES targets(id) ON DELETE SET NULL;

UPDATE findings f
  SET target_id = t.id
  FROM targets t
  WHERE f.target_id IS NULL
    AND f.target != ''
    AND (t.value = f.target OR t.value LIKE '%://' || f.target || '%')
    AND t.project_path = f.project_path;

CREATE INDEX IF NOT EXISTS idx_findings_target_id ON findings(target_id);
