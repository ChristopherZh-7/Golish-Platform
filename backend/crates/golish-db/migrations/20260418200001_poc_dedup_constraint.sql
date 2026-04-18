-- Add unique constraint on (cve_id, name) to prevent duplicate PoC imports.
-- First remove any existing duplicates (keep the most recently updated one).
DELETE FROM vuln_kb_pocs a
USING vuln_kb_pocs b
WHERE a.id < b.id
  AND a.cve_id = b.cve_id
  AND a.name = b.name;

CREATE UNIQUE INDEX IF NOT EXISTS idx_vuln_kb_pocs_cve_name
    ON vuln_kb_pocs(cve_id, name);
