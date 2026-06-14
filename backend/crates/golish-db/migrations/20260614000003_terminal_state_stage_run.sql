-- Persist a session's live `stage_run` per-org fan-out (rows + summary +
-- stage/role/coverage config + tied requestId) so the chat tool card's
-- covered/active/queued/blocked styling and the per-org detail rows survive a
-- panel close / app restart instead of being lost with the in-memory state.
-- Nullable JSONB, mirrors plan_json / retired_plans_json. Backward compatible:
-- old rows read NULL and simply restore no stage-run (prior behaviour).
ALTER TABLE terminal_state ADD COLUMN IF NOT EXISTS stage_run_json JSONB;
