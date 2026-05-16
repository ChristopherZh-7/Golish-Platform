-- P0-4 · Sub-agent dispatch tracking.
--
-- Each row records one `execute_sub_agent_with_client` call so that on
-- the next app start we can see which sub-agents were mid-flight when
-- the previous instance died. See
-- docs/design/2026-05-17-dispatch-resume.md.

CREATE TYPE sub_agent_dispatch_status AS ENUM (
    'running',
    'completed',
    'failed',
    'cancelled'
);

CREATE TABLE sub_agent_dispatches (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id         UUID REFERENCES sessions(id) ON DELETE CASCADE,
    parent_dispatch_id UUID REFERENCES sub_agent_dispatches(id) ON DELETE SET NULL,
    agent_id           TEXT NOT NULL,
    tool_call_id       TEXT,
    depth              INT NOT NULL DEFAULT 0,
    status             sub_agent_dispatch_status NOT NULL DEFAULT 'running',
    args               JSONB NOT NULL DEFAULT '{}',
    result             JSONB,
    error_message      TEXT,
    started_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at        TIMESTAMPTZ
);

CREATE INDEX idx_sub_agent_dispatches_session ON sub_agent_dispatches(session_id);
CREATE INDEX idx_sub_agent_dispatches_status  ON sub_agent_dispatches(status);
CREATE INDEX idx_sub_agent_dispatches_parent  ON sub_agent_dispatches(parent_dispatch_id);
CREATE INDEX idx_sub_agent_dispatches_started ON sub_agent_dispatches(started_at DESC);

COMMENT ON TABLE sub_agent_dispatches IS
    'P0-4: lifecycle record of every sub-agent dispatch for resumability.';
COMMENT ON COLUMN sub_agent_dispatches.parent_dispatch_id IS
    'Id of the parent dispatch when a sub-agent triggered this one (depth > 0).';
COMMENT ON COLUMN sub_agent_dispatches.depth IS
    'Recursion depth: 0 for primary-LLM dispatched sub-agents, +1 each nested level.';
COMMENT ON COLUMN sub_agent_dispatches.args IS
    'JSON arguments passed to execute_sub_agent_with_client (sanitised by caller).';
COMMENT ON COLUMN sub_agent_dispatches.result IS
    'JSON result snapshot when status is completed; null otherwise.';
