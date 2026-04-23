-- ============================================================================
-- Observability Enhancement: msg_logs, screenshots, vector_store_logs
-- Closes gap with PentAGI's per-concern logging granularity.
-- ============================================================================

-- Per-message LLM conversation log (PentAGI: msglogs).
-- Captures each LLM turn as a separate row rather than a JSONB blob,
-- enabling per-turn queries, filtering by type, and streaming subscriptions.
CREATE TYPE msglog_type AS ENUM (
    'user_message', 'assistant_message', 'tool_call', 'tool_result',
    'system_hook', 'plan_update', 'error'
);

CREATE TYPE msglog_result_format AS ENUM ('text', 'json', 'markdown', 'html');

CREATE TABLE msg_logs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id         UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id      UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    agent           agent_type,
    msg_type        msglog_type NOT NULL,
    message         TEXT NOT NULL DEFAULT '',
    result          TEXT NOT NULL DEFAULT '',
    result_format   msglog_result_format NOT NULL DEFAULT 'text',
    thinking        TEXT,
    project_path    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_msglogs_session ON msg_logs(session_id);
CREATE INDEX idx_msglogs_task    ON msg_logs(task_id);
CREATE INDEX idx_msglogs_type    ON msg_logs(msg_type);

-- Browser screenshots captured during pentest sessions (PentAGI: screenshots).
CREATE TABLE screenshots (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id         UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id      UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    name            TEXT NOT NULL,
    url             TEXT NOT NULL,
    file_path       TEXT,
    content_type    TEXT NOT NULL DEFAULT 'image/png',
    size_bytes      INTEGER,
    project_path    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_screenshots_session ON screenshots(session_id);

-- Vector store operation audit trail (PentAGI: vecstorelogs).
-- Tracks every memory store/search/delete operation for observability.
CREATE TYPE vecstore_action AS ENUM ('store', 'search', 'delete', 'update');

CREATE TABLE vector_store_logs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id         UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id      UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    initiator       agent_type,
    executor        agent_type,
    action          vecstore_action NOT NULL,
    query           TEXT NOT NULL DEFAULT '',
    filter          JSONB NOT NULL DEFAULT '{}',
    result          TEXT NOT NULL DEFAULT '',
    result_count    INTEGER NOT NULL DEFAULT 0,
    project_path    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_vecstorelogs_session ON vector_store_logs(session_id);
CREATE INDEX idx_vecstorelogs_action  ON vector_store_logs(action);
