-- Extend agent_type enum to include all default sub-agent IDs.
-- PostgreSQL supports ALTER TYPE ... ADD VALUE for enum extension.

ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'analyzer';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'explorer';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'researcher';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'executor';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'js_harvester';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'js_analyzer';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'worker';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'planner';
