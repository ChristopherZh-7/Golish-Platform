-- The per-conversation `use_agents` toggle has been retired. Sub-agent
-- dispatch is now strictly bound to the execution mode column (chat =
-- single-agent, task = multi-agent), so the dedicated boolean column is
-- redundant. Drop it.
ALTER TABLE terminal_state DROP COLUMN IF EXISTS use_agents;
