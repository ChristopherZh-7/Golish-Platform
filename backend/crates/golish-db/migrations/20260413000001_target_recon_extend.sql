-- Extend targets table with recon data fields for httpx/nmap/whatweb output

ALTER TABLE targets ADD COLUMN IF NOT EXISTS real_ip TEXT NOT NULL DEFAULT '';
ALTER TABLE targets ADD COLUMN IF NOT EXISTS cdn_waf TEXT NOT NULL DEFAULT '';
ALTER TABLE targets ADD COLUMN IF NOT EXISTS http_title TEXT NOT NULL DEFAULT '';
ALTER TABLE targets ADD COLUMN IF NOT EXISTS http_status INTEGER;
ALTER TABLE targets ADD COLUMN IF NOT EXISTS webserver TEXT NOT NULL DEFAULT '';
ALTER TABLE targets ADD COLUMN IF NOT EXISTS os_info TEXT NOT NULL DEFAULT '';
ALTER TABLE targets ADD COLUMN IF NOT EXISTS content_type TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_targets_real_ip ON targets(real_ip) WHERE real_ip != '';
CREATE INDEX IF NOT EXISTS idx_targets_cdn_waf ON targets(cdn_waf) WHERE cdn_waf != '';

-- Directory entries discovered by fuzzing tools (ffuf, feroxbuster, dirsearch)
CREATE TABLE IF NOT EXISTS directory_entries (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    target_id    UUID REFERENCES targets(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    status_code  INTEGER,
    content_length INTEGER,
    lines        INTEGER,
    words        INTEGER,
    content_type TEXT NOT NULL DEFAULT '',
    tool         TEXT NOT NULL DEFAULT '',
    project_path TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dirent_target ON directory_entries(target_id);
CREATE INDEX IF NOT EXISTS idx_dirent_url ON directory_entries(url);
CREATE INDEX IF NOT EXISTS idx_dirent_status ON directory_entries(status_code);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dirent_unique ON directory_entries(url, tool) WHERE target_id IS NOT NULL;
