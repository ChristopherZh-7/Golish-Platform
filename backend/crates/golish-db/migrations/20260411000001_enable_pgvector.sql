-- ============================================================================
-- Enable pgvector extension and upgrade memories.embedding from BYTEA to vector
-- Gracefully degrades if pgvector is not installed on the server.
-- ============================================================================

-- Clean up old BYTEA-based embedding columns (always safe)
ALTER TABLE memories DROP COLUMN IF EXISTS embedding;
ALTER TABLE memories DROP COLUMN IF EXISTS embedding_dim;

-- Try to load pgvector and create the typed column + index.
-- If the extension isn't installed (embedded PG without pgvector .so),
-- fall back to a BYTEA column so the schema is valid but semantic search
-- will use Rust-side cosine similarity instead of SQL-side.
DO $$
BEGIN
    CREATE EXTENSION IF NOT EXISTS vector;
    ALTER TABLE memories ADD COLUMN embedding vector(1536);
    CREATE INDEX IF NOT EXISTS idx_memories_embedding_cosine
        ON memories USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);
    RAISE NOTICE 'pgvector enabled — using native vector(1536) column';
EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'pgvector not available (%), falling back to BYTEA embedding', SQLERRM;
    ALTER TABLE memories ADD COLUMN IF NOT EXISTS embedding BYTEA;
END$$;

-- ============================================================================
-- Add agent_logs table for multi-agent call tracking
-- ============================================================================

CREATE TABLE IF NOT EXISTS agent_logs (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id     UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id  UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    initiator   agent_type NOT NULL,
    executor    agent_type NOT NULL,
    task        TEXT NOT NULL,
    result      TEXT,
    duration_ms INTEGER,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_agentlogs_session ON agent_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_agentlogs_initiator ON agent_logs(initiator);
CREATE INDEX IF NOT EXISTS idx_agentlogs_executor ON agent_logs(executor);
