-- ============================================================================
-- Standardize fields across all data tables:
--   1. Add source_tool where missing
--   2. Add updated_at where missing
--   3. Normalize project_path: NULL → '', set NOT NULL DEFAULT ''
--   4. Remove redundant targets.technologies (use fingerprints table)
-- ============================================================================

-- ── 1. Add source_tool ─────────────────────────────────────────────────

ALTER TABLE target_assets
  ADD COLUMN IF NOT EXISTS source_tool TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE js_analysis_results
  ADD COLUMN IF NOT EXISTS source_tool TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE sensitive_scan_results
  ADD COLUMN IF NOT EXISTS source_tool TEXT NOT NULL DEFAULT 'sensitive_scan';

-- ── 2. Add updated_at ──────────────────────────────────────────────────

ALTER TABLE js_analysis_results
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

ALTER TABLE fingerprints
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

ALTER TABLE directory_entries
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- ── 3. Normalize project_path (NULL → '') ──────────────────────────────

UPDATE target_assets SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE target_assets ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE target_assets ALTER COLUMN project_path SET NOT NULL;

UPDATE api_endpoints SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE api_endpoints ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE api_endpoints ALTER COLUMN project_path SET NOT NULL;

UPDATE js_analysis_results SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE js_analysis_results ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE js_analysis_results ALTER COLUMN project_path SET NOT NULL;

UPDATE fingerprints SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE fingerprints ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE fingerprints ALTER COLUMN project_path SET NOT NULL;

UPDATE passive_scan_logs SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE passive_scan_logs ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE passive_scan_logs ALTER COLUMN project_path SET NOT NULL;

UPDATE directory_entries SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE directory_entries ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE directory_entries ALTER COLUMN project_path SET NOT NULL;

UPDATE findings SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE findings ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE findings ALTER COLUMN project_path SET NOT NULL;

UPDATE targets SET project_path = '' WHERE project_path IS NULL;
ALTER TABLE targets ALTER COLUMN project_path SET DEFAULT '';
ALTER TABLE targets ALTER COLUMN project_path SET NOT NULL;

-- ── 4. Remove redundant targets.technologies ───────────────────────────

ALTER TABLE targets DROP COLUMN IF EXISTS technologies;

-- ── 5. Rename topology_scans → sitemap_store ───────────────────────────

ALTER TABLE IF EXISTS topology_scans RENAME TO sitemap_store;
