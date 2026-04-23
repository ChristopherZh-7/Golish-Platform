-- ============================================================================
-- Prompt template overrides: user-customizable prompt templates stored in DB.
-- Default templates are embedded in the binary; DB rows override them.
-- ============================================================================

CREATE TABLE prompt_templates (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    template_name   TEXT NOT NULL UNIQUE,
    content         TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    is_active       BOOLEAN NOT NULL DEFAULT true,
    project_path    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_prompt_templates_name ON prompt_templates(template_name);
CREATE INDEX idx_prompt_templates_active ON prompt_templates(is_active) WHERE is_active = true;
