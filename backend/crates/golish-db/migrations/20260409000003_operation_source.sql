-- Add operation source tracking to distinguish automated/ai/manual actions
-- Values: 'automated' (pipeline), 'ai' (agent), 'manual' (human)

ALTER TABLE findings ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';

ALTER TABLE tool_calls ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'ai';

ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';
