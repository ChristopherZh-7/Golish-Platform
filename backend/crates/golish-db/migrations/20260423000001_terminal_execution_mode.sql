-- Persist execution_mode, use_agents, and retired plan history on terminal_state
-- so Task mode, sub-agent toggle, and plan iteration cards survive app restart.
ALTER TABLE terminal_state ADD COLUMN IF NOT EXISTS execution_mode TEXT;
ALTER TABLE terminal_state ADD COLUMN IF NOT EXISTS use_agents BOOLEAN;
ALTER TABLE terminal_state ADD COLUMN IF NOT EXISTS retired_plans_json JSONB;
