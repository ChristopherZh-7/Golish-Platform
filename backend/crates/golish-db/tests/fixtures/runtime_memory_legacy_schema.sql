-- Minimal pre-runtime-memory schema used by the Task 1 upgrade contract tests.
-- It deliberately contains only the tables and keys referenced by the expand
-- migration, so the test proves the migration does not rely on unrelated rows.

CREATE TABLE operation_state (
    operation_id UUID PRIMARY KEY,
    profile TEXT NOT NULL,
    current_stage TEXT NOT NULL,
    stage_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_evidence_audit_id BIGINT,
    last_classification_id BIGINT,
    last_scope_version BIGINT,
    state_blob JSONB NOT NULL DEFAULT '{}'::jsonb,
    superseded_by UUID REFERENCES operation_state(operation_id),
    engagement_org_id UUID
);

CREATE TABLE stage_runs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    stage_kind TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'started',
    active_sprint_contract_id UUID
);

CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    title TEXT,
    status TEXT NOT NULL DEFAULT 'created',
    workspace_path TEXT,
    project_path TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tasks (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    title TEXT,
    input TEXT NOT NULL,
    result TEXT,
    status TEXT NOT NULL DEFAULT 'created',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tool_calls (
    id UUID PRIMARY KEY,
    call_id TEXT NOT NULL,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id UUID,
    agent TEXT,
    name TEXT NOT NULL,
    args JSONB NOT NULL DEFAULT '{}'::jsonb,
    result TEXT,
    status TEXT NOT NULL DEFAULT 'received',
    duration_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE message_chains (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id UUID,
    agent TEXT NOT NULL,
    model TEXT,
    provider TEXT,
    chain JSONB,
    tokens_in INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    tokens_cache_in INTEGER NOT NULL DEFAULT 0,
    cost_in_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    cost_out_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
