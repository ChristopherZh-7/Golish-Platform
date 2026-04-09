-- Add status tracking to targets for HITL workflow

DO $$ BEGIN
    CREATE TYPE target_status AS ENUM ('new', 'recon', 'recon_done', 'scanning', 'tested');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

ALTER TABLE targets ADD COLUMN IF NOT EXISTS status target_status NOT NULL DEFAULT 'new';

-- Source: how this target was added
ALTER TABLE targets ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';
-- 'manual' = user added, 'discovered' = AI found it, 'imported' = batch import

-- Parent target (for discovered sub-targets)
ALTER TABLE targets ADD COLUMN IF NOT EXISTS parent_id UUID REFERENCES targets(id) ON DELETE SET NULL;

-- Port/service info discovered during recon
ALTER TABLE targets ADD COLUMN IF NOT EXISTS ports JSONB NOT NULL DEFAULT '[]';
-- e.g. [{"port": 80, "proto": "tcp", "service": "http", "version": "nginx/1.18"}]

-- Technology fingerprint
ALTER TABLE targets ADD COLUMN IF NOT EXISTS technologies JSONB NOT NULL DEFAULT '[]';
-- e.g. ["nginx", "react", "express"]

CREATE INDEX IF NOT EXISTS idx_targets_status ON targets(status);
CREATE INDEX IF NOT EXISTS idx_targets_source ON targets(source);
CREATE INDEX IF NOT EXISTS idx_targets_parent ON targets(parent_id);
