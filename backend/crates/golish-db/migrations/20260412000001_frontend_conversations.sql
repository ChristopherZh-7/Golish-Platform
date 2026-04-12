-- ============================================================================
-- Frontend Conversation & Timeline Persistence
-- Replaces workspace.json for all runtime conversation and timeline data.
-- ============================================================================

-- Frontend chat conversations (one per "chat tab" in the UI)
CREATE TABLE conversations (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL DEFAULT 'New Chat',
    ai_session_id   TEXT NOT NULL,
    project_path    TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_conversations_project ON conversations(project_path);
CREATE INDEX idx_conversations_sort ON conversations(sort_order);

-- Individual chat messages within a conversation
CREATE TABLE chat_messages (
    id              TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content         TEXT NOT NULL DEFAULT '',
    thinking        TEXT,
    error           TEXT,
    tool_calls      JSONB,
    tool_calls_content_offset INTEGER,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, conversation_id)
);
CREATE INDEX idx_chat_messages_conv ON chat_messages(conversation_id);
CREATE INDEX idx_chat_messages_sort ON chat_messages(conversation_id, sort_order);

-- Timeline blocks: commands, tool executions, sub-agents, agent messages, pipelines
-- Each block belongs to a terminal session, which belongs to a conversation.
CREATE TABLE timeline_blocks (
    id              TEXT NOT NULL,
    session_id      TEXT NOT NULL,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    block_type      TEXT NOT NULL,
    data            JSONB NOT NULL DEFAULT '{}',
    batch_id        TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, session_id)
);
CREATE INDEX idx_timeline_session ON timeline_blocks(session_id);
CREATE INDEX idx_timeline_conv ON timeline_blocks(conversation_id);
CREATE INDEX idx_timeline_type ON timeline_blocks(block_type);
CREATE INDEX idx_timeline_sort ON timeline_blocks(session_id, sort_order);

-- Terminal state (scrollback, working directory) per conversation
CREATE TABLE terminal_state (
    session_id      TEXT PRIMARY KEY,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    working_directory TEXT NOT NULL DEFAULT '',
    scrollback      TEXT NOT NULL DEFAULT '',
    custom_name     TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_terminal_state_conv ON terminal_state(conversation_id);

-- Workspace-level preferences (replaces localStorage small data)
-- One row per project, preferences stored as JSONB
CREATE TABLE workspace_preferences (
    project_path    TEXT PRIMARY KEY,
    active_conversation_id TEXT,
    ai_model        JSONB,
    approval_mode   TEXT,
    approval_patterns JSONB,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
