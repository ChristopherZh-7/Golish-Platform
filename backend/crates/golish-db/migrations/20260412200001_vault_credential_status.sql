-- Add credential validity tracking fields to vault_entries
ALTER TABLE vault_entries
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS last_validated_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS source_url TEXT NOT NULL DEFAULT '';
