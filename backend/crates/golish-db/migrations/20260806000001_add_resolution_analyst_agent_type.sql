-- Resolution Analysts are a first-class bounded Enumeration role. Tracking
-- writes cast the runtime role to agent_type, so the database enum must be
-- expanded before the role is used. This is forward-only and additive.

ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'resolution_analyst';
