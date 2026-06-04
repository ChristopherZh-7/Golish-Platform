-- Per-stage plan isolation: tag each plan row with the harness stage it
-- belongs to. NULL = chat-mode / non-harness plan (legacy single-card).
ALTER TABLE execution_plans ADD COLUMN stage_id TEXT;

CREATE INDEX idx_plans_session_stage
    ON execution_plans(session_id, stage_id);

COMMENT ON COLUMN execution_plans.stage_id IS
    'Harness stage id (scoping, target_intel, …) this plan belongs to; NULL = chat-mode';
