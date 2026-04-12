-- Execution plans: structured multi-step task plans that persist across sessions.
-- Used for AI agent task continuation ("continue" functionality).

CREATE TYPE plan_status AS ENUM ('planning', 'in_progress', 'paused', 'completed', 'failed', 'cancelled');

CREATE TABLE execution_plans (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID REFERENCES sessions(id) ON DELETE SET NULL,
    project_path    TEXT,
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    steps           JSONB NOT NULL DEFAULT '[]',
    status          plan_status NOT NULL DEFAULT 'planning',
    current_step    INTEGER NOT NULL DEFAULT 0,
    context         JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_plans_project ON execution_plans(project_path);
CREATE INDEX idx_plans_session ON execution_plans(session_id);
CREATE INDEX idx_plans_status  ON execution_plans(status);
CREATE INDEX idx_plans_updated ON execution_plans(updated_at DESC);

COMMENT ON TABLE execution_plans IS 'Structured multi-step execution plans for AI agent task tracking and continuation';
COMMENT ON COLUMN execution_plans.steps IS 'JSON array of {id, title, description, status, agent, result, started_at, completed_at}';
COMMENT ON COLUMN execution_plans.current_step IS 'Index of the currently executing step (0-based)';
COMMENT ON COLUMN execution_plans.context IS 'Arbitrary context data for plan resumption (findings, intermediate results, etc.)';
