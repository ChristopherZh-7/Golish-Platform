-- ============================================================================
-- Upgrade memories.embedding from BYTEA to vector(1536) if pgvector is now
-- available. This handles the case where the initial migration fell back to
-- BYTEA because the pgvector .dylib wasn't in the correct directory.
-- ============================================================================

DO $$
DECLARE
    col_type TEXT;
BEGIN
    CREATE EXTENSION IF NOT EXISTS vector;

    SELECT data_type INTO col_type
    FROM information_schema.columns
    WHERE table_name = 'memories' AND column_name = 'embedding';

    IF col_type IS NULL THEN
        ALTER TABLE memories ADD COLUMN embedding vector(1536);
    ELSIF col_type = 'bytea' THEN
        ALTER TABLE memories DROP COLUMN embedding;
        ALTER TABLE memories ADD COLUMN embedding vector(1536);
        RAISE NOTICE 'Upgraded memories.embedding from BYTEA to vector(1536)';
    END IF;

    CREATE INDEX IF NOT EXISTS idx_memories_embedding_cosine
        ON memories USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);

EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'pgvector still unavailable (%); keeping current column type', SQLERRM;
END$$;
