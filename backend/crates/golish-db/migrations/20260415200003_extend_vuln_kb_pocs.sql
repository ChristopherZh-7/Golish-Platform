-- Extend vuln_kb_pocs to support "PoC first, research later" workflow.
-- Adds source tracking, severity, and verification status.

ALTER TABLE vuln_kb_pocs
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual',
    ADD COLUMN IF NOT EXISTS source_url TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS severity TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS verified BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';

COMMENT ON COLUMN vuln_kb_pocs.source IS 'nuclei_template, github, exploitdb, manual';
COMMENT ON COLUMN vuln_kb_pocs.severity IS 'critical, high, medium, low, info, unknown';
COMMENT ON COLUMN vuln_kb_pocs.verified IS 'Whether this PoC has been tested and confirmed working';

CREATE INDEX IF NOT EXISTS idx_vuln_kb_pocs_source   ON vuln_kb_pocs(source);
CREATE INDEX IF NOT EXISTS idx_vuln_kb_pocs_severity ON vuln_kb_pocs(severity);
CREATE INDEX IF NOT EXISTS idx_vuln_kb_pocs_verified ON vuln_kb_pocs(verified);
CREATE INDEX IF NOT EXISTS idx_vuln_kb_pocs_tags     ON vuln_kb_pocs USING GIN(tags);
