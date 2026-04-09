-- ============================================================================
-- Enhance memories table for richer embedding-based semantic search
-- Adds structured metadata columns (instead of relying solely on JSONB)
-- and an embedding_dim column so Rust-side similarity knows the vector size.
-- ============================================================================

-- Structured filter columns (mirrors PentAGI's pgvector metadata filters)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS task_id     UUID REFERENCES tasks(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS subtask_id  UUID REFERENCES subtasks(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS agent       agent_type;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS tool_name   TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS doc_type    TEXT NOT NULL DEFAULT 'memory';

-- Embedding dimension stored alongside the vector so we can decode BYTEA correctly
ALTER TABLE memories ADD COLUMN IF NOT EXISTS embedding_dim INTEGER;

-- Project-scoped isolation: memories belong to a project, not just a session
ALTER TABLE memories ADD COLUMN IF NOT EXISTS project_path TEXT;

-- Indexes for the new filter columns
CREATE INDEX IF NOT EXISTS idx_memories_session  ON memories(session_id);
CREATE INDEX IF NOT EXISTS idx_memories_task     ON memories(task_id);
CREATE INDEX IF NOT EXISTS idx_memories_doc_type ON memories(doc_type);
CREATE INDEX IF NOT EXISTS idx_memories_agent    ON memories(agent);
CREATE INDEX IF NOT EXISTS idx_memories_project  ON memories(project_path);
