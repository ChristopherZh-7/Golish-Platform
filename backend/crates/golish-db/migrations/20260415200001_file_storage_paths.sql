-- ============================================================================
-- Add file_path columns for hybrid DB + filesystem storage
-- ============================================================================

-- JS analysis: link to captured JS file on disk
ALTER TABLE js_analysis_results ADD COLUMN file_path TEXT;
COMMENT ON COLUMN js_analysis_results.file_path IS 'Relative path from project root to captured JS file, e.g. .golish/captures/example.com/443/js/a1b2c3d4_app.js';

-- API endpoints: link to captured HTTP request/response
ALTER TABLE api_endpoints ADD COLUMN capture_path TEXT;
COMMENT ON COLUMN api_endpoints.capture_path IS 'Relative path to HTTP request/response capture file';
