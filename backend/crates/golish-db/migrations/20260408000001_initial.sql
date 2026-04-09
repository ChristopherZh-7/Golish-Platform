-- ============================================================================
-- Golish Platform: Initial PostgreSQL Schema
-- Migrated from SQLite + JSON file storage
-- Includes pgvector extension for semantic memory
-- ============================================================================

-- Core extensions (pgvector will be added in a later migration once available)
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ============================================================================
-- AI Session & Execution Tracking (new, inspired by PentAGI)
-- ============================================================================

CREATE TYPE session_status AS ENUM ('created', 'running', 'waiting', 'finished', 'failed');
CREATE TYPE task_status    AS ENUM ('created', 'running', 'waiting', 'finished', 'failed');
CREATE TYPE subtask_status AS ENUM ('created', 'running', 'waiting', 'finished', 'failed');
CREATE TYPE toolcall_status AS ENUM ('received', 'running', 'finished', 'failed');
CREATE TYPE stream_type    AS ENUM ('stdin', 'stdout', 'stderr');
CREATE TYPE memory_type    AS ENUM ('observation', 'conclusion', 'technique', 'vulnerability', 'tool_usage');
CREATE TYPE agent_type     AS ENUM (
    'primary', 'pentester', 'coder', 'searcher', 'memorist',
    'reporter', 'adviser', 'reflector', 'enricher', 'installer',
    'summarizer', 'assistant'
);

-- A complete AI conversation session
CREATE TABLE sessions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    title           TEXT,
    status          session_status NOT NULL DEFAULT 'created',
    workspace_path  TEXT,
    workspace_label TEXT,
    model           TEXT,
    provider        TEXT,
    project_path    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created ON sessions(created_at DESC);

-- Session data (messages, transcript, tools) stored as JSONB for flexibility
CREATE TABLE session_data (
    session_id          UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    messages            JSONB NOT NULL DEFAULT '[]',
    transcript          JSONB NOT NULL DEFAULT '[]',
    distinct_tools      JSONB NOT NULL DEFAULT '[]',
    total_messages      INTEGER NOT NULL DEFAULT 0,
    sidecar_session_id  TEXT,
    agent_mode          TEXT
);

-- A user prompt and its full processing cycle
CREATE TABLE tasks (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    title       TEXT,
    input       TEXT NOT NULL,
    result      TEXT,
    status      task_status NOT NULL DEFAULT 'created',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_tasks_session ON tasks(session_id);
CREATE INDEX idx_tasks_status ON tasks(status);

-- Agent-decomposed sub-units of work
CREATE TABLE subtasks (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    title           TEXT,
    description     TEXT,
    agent           agent_type,
    result          TEXT,
    context         TEXT,
    status          subtask_status NOT NULL DEFAULT 'created',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_subtasks_task ON subtasks(task_id);
CREATE INDEX idx_subtasks_session ON subtasks(session_id);

-- Every tool invocation with full lifecycle tracking
CREATE TABLE tool_calls (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    call_id         TEXT NOT NULL,
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id         UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id      UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    agent           agent_type,
    name            TEXT NOT NULL,
    args            JSONB NOT NULL DEFAULT '{}',
    result          TEXT,
    status          toolcall_status NOT NULL DEFAULT 'received',
    duration_ms     INTEGER,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_toolcalls_session ON tool_calls(session_id);
CREATE INDEX idx_toolcalls_task ON tool_calls(task_id);
CREATE INDEX idx_toolcalls_name ON tool_calls(name);
CREATE INDEX idx_toolcalls_status ON tool_calls(status);

-- Terminal I/O records
CREATE TABLE terminal_logs (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id     UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id  UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    stream      stream_type NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_termlogs_session ON terminal_logs(session_id);

-- Web/API search records
CREATE TABLE search_logs (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id     UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id  UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    initiator   agent_type,
    engine      TEXT NOT NULL,
    query       TEXT NOT NULL,
    result      TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_searchlogs_session ON search_logs(session_id);
CREATE INDEX idx_searchlogs_engine ON search_logs(engine);

-- Per-agent conversation chain with token/cost tracking
CREATE TABLE message_chains (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id         UUID REFERENCES tasks(id) ON DELETE SET NULL,
    subtask_id      UUID REFERENCES subtasks(id) ON DELETE SET NULL,
    agent           agent_type NOT NULL,
    model           TEXT,
    provider        TEXT,
    chain           JSONB,
    tokens_in       INTEGER NOT NULL DEFAULT 0,
    tokens_out      INTEGER NOT NULL DEFAULT 0,
    tokens_cache_in INTEGER NOT NULL DEFAULT 0,
    cost_in_usd     DOUBLE PRECISION NOT NULL DEFAULT 0,
    cost_out_usd    DOUBLE PRECISION NOT NULL DEFAULT 0,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_msgchains_session ON message_chains(session_id);
CREATE INDEX idx_msgchains_agent ON message_chains(agent);

-- Long-term knowledge memory
-- NOTE: embedding column uses BYTEA as a placeholder. When pgvector is
-- available, a migration will ALTER this to vector(1536) and add an IVFFlat index.
CREATE TABLE memories (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id      UUID REFERENCES sessions(id) ON DELETE SET NULL,
    content         TEXT NOT NULL,
    mem_type        memory_type NOT NULL,
    embedding       BYTEA,
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_memories_type ON memories(mem_type);

-- ============================================================================
-- Pentest Data (migrated from SQLite data.db)
-- ============================================================================

CREATE TYPE target_type AS ENUM ('domain', 'ip', 'cidr', 'url', 'wildcard');
CREATE TYPE scope_type  AS ENUM ('in', 'out');
CREATE TYPE severity    AS ENUM ('critical', 'high', 'medium', 'low', 'info');
CREATE TYPE finding_status AS ENUM ('open', 'confirmed', 'fixed', 'false_positive', 'accepted');
CREATE TYPE vault_entry_type AS ENUM ('password', 'api_key', 'token', 'certificate', 'ssh_key', 'other');

-- Penetration testing targets
CREATE TABLE targets (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name        TEXT NOT NULL,
    target_type target_type NOT NULL DEFAULT 'domain',
    value       TEXT NOT NULL,
    tags        JSONB NOT NULL DEFAULT '[]',
    notes       TEXT NOT NULL DEFAULT '',
    scope       scope_type NOT NULL DEFAULT 'in',
    grp         TEXT NOT NULL DEFAULT 'default',
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_targets_type ON targets(target_type);
CREATE INDEX idx_targets_scope ON targets(scope);
CREATE INDEX idx_targets_project ON targets(project_path);

CREATE TABLE target_groups (
    name         TEXT NOT NULL,
    project_path TEXT,
    PRIMARY KEY (name, project_path)
);

-- Discovered vulnerabilities / findings
CREATE TABLE findings (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    title       TEXT NOT NULL,
    sev         severity NOT NULL DEFAULT 'info',
    cvss        DOUBLE PRECISION,
    url         TEXT NOT NULL DEFAULT '',
    target      TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    steps       TEXT NOT NULL DEFAULT '',
    remediation TEXT NOT NULL DEFAULT '',
    tags        JSONB NOT NULL DEFAULT '[]',
    tool        TEXT NOT NULL DEFAULT '',
    template    TEXT NOT NULL DEFAULT '',
    refs        JSONB NOT NULL DEFAULT '[]',
    evidence    JSONB NOT NULL DEFAULT '[]',
    status      finding_status NOT NULL DEFAULT 'open',
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_findings_severity ON findings(sev);
CREATE INDEX idx_findings_status ON findings(status);
CREATE INDEX idx_findings_target ON findings(target);
CREATE INDEX idx_findings_project ON findings(project_path);

-- Entity-attached notes
CREATE TABLE notes (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    entity_type TEXT NOT NULL,
    entity_id   TEXT NOT NULL,
    content     TEXT NOT NULL,
    color       TEXT NOT NULL DEFAULT 'yellow',
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_notes_entity ON notes(entity_type, entity_id);

-- Audit trail
CREATE TABLE audit_log (
    id          BIGSERIAL PRIMARY KEY,
    action      TEXT NOT NULL,
    category    TEXT NOT NULL DEFAULT 'general',
    details     TEXT NOT NULL DEFAULT '',
    entity_type TEXT,
    entity_id   TEXT,
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_audit_created ON audit_log(created_at DESC);
CREATE INDEX idx_audit_category ON audit_log(category);

-- Credential vault (values stored obfuscated)
CREATE TABLE vault_entries (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name        TEXT NOT NULL,
    entry_type  vault_entry_type NOT NULL DEFAULT 'password',
    value       TEXT NOT NULL DEFAULT '',
    username    TEXT NOT NULL DEFAULT '',
    notes       TEXT NOT NULL DEFAULT '',
    project     TEXT NOT NULL DEFAULT '',
    tags        JSONB NOT NULL DEFAULT '[]',
    project_path TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Network topology scans (JSON blobs)
CREATE TABLE topology_scans (
    name         TEXT NOT NULL,
    data         JSONB NOT NULL,
    project_path TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (name, project_path)
);

-- Methodology project instances (JSON blobs)
CREATE TABLE methodology_projects (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    data         JSONB NOT NULL,
    project_path TEXT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Automation pipelines (JSON blobs)
CREATE TABLE pipelines (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    data         JSONB NOT NULL,
    project_path TEXT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- Vulnerability Intelligence (migrated from JSON files)
-- ============================================================================

CREATE TABLE vuln_feeds (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    feed_type    TEXT NOT NULL,
    url          TEXT NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    last_fetched TIMESTAMPTZ
);

CREATE TABLE vuln_entries (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cve_id            TEXT NOT NULL UNIQUE,
    title             TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    sev               TEXT NOT NULL DEFAULT 'unknown',
    cvss_score        DOUBLE PRECISION,
    published         TEXT NOT NULL DEFAULT '',
    source            TEXT NOT NULL DEFAULT '',
    refs              JSONB NOT NULL DEFAULT '[]',
    affected_products JSONB NOT NULL DEFAULT '[]',
    fetched_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_vuln_cve ON vuln_entries(cve_id);
CREATE INDEX idx_vuln_sev ON vuln_entries(sev);
