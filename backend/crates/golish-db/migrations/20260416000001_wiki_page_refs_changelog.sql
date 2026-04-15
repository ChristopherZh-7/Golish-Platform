-- ============================================================================
-- Wiki page cross-references: tracks which pages link to which other pages.
-- Enables backlink queries, orphan detection, and relationship graphs.
-- ============================================================================

CREATE TABLE wiki_page_refs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    source_path     TEXT NOT NULL,
    target_path     TEXT NOT NULL,
    context         TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(source_path, target_path)
);

COMMENT ON COLUMN wiki_page_refs.source_path IS 'Path of the page containing the link';
COMMENT ON COLUMN wiki_page_refs.target_path IS 'Path of the page being linked to';
COMMENT ON COLUMN wiki_page_refs.context IS 'Surrounding text snippet where the link appears';

CREATE INDEX idx_wiki_refs_source ON wiki_page_refs(source_path);
CREATE INDEX idx_wiki_refs_target ON wiki_page_refs(target_path);

-- ============================================================================
-- Wiki changelog: append-only log of all wiki modifications.
-- Replaces the unmaintained log.md with a queryable DB table.
-- ============================================================================

CREATE TABLE wiki_changelog (
    id              BIGSERIAL PRIMARY KEY,
    page_path       TEXT NOT NULL,
    action          TEXT NOT NULL DEFAULT 'update',
    title           TEXT NOT NULL DEFAULT '',
    category        TEXT NOT NULL DEFAULT '',
    actor           TEXT NOT NULL DEFAULT 'agent',
    summary         TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN wiki_changelog.action IS 'create, update, delete, link, unlink';
COMMENT ON COLUMN wiki_changelog.actor IS 'agent, user, ingest, lint';
COMMENT ON COLUMN wiki_changelog.summary IS 'Brief description of what changed';

CREATE INDEX idx_wiki_changelog_path ON wiki_changelog(page_path);
CREATE INDEX idx_wiki_changelog_time ON wiki_changelog(created_at DESC);
CREATE INDEX idx_wiki_changelog_action ON wiki_changelog(action);
