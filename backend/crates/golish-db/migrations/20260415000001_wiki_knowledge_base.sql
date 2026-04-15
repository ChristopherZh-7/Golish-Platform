-- ============================================================================
-- Wiki Knowledge Base: Persistent vulnerability knowledge pages
-- Stores wiki page metadata + content for full-text search via PostgreSQL.
-- The filesystem (markdown files) remains the primary storage; this table
-- acts as a search index that is kept in sync on write/delete.
-- ============================================================================

CREATE TABLE wiki_pages (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    path            TEXT UNIQUE NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    category        TEXT NOT NULL DEFAULT 'uncategorized',
    tags            TEXT[] NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'draft',
    content         TEXT NOT NULL DEFAULT '',
    word_count      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN wiki_pages.status IS 'Knowledge completeness: draft, partial, complete, needs-poc, verified';

CREATE INDEX idx_wiki_pages_category ON wiki_pages(category);
CREATE INDEX idx_wiki_pages_tags     ON wiki_pages USING GIN(tags);
CREATE INDEX idx_wiki_pages_updated  ON wiki_pages(updated_at DESC);

CREATE INDEX idx_wiki_pages_fts ON wiki_pages
    USING GIN (to_tsvector('english', title || ' ' || content));

-- ============================================================================
-- Vulnerability knowledge links (migrated from frontend localStorage)
-- Connects CVEs to wiki pages, PoC templates, and scan history.
-- ============================================================================

CREATE TABLE vuln_kb_links (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cve_id          TEXT NOT NULL,
    wiki_path       TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(cve_id, wiki_path)
);

CREATE INDEX idx_vuln_kb_links_cve  ON vuln_kb_links(cve_id);
CREATE INDEX idx_vuln_kb_links_path ON vuln_kb_links(wiki_path);

CREATE TABLE vuln_kb_pocs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cve_id          TEXT NOT NULL,
    name            TEXT NOT NULL,
    poc_type        TEXT NOT NULL DEFAULT 'script',
    language        TEXT NOT NULL DEFAULT 'python',
    content         TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_vuln_kb_pocs_cve ON vuln_kb_pocs(cve_id);
