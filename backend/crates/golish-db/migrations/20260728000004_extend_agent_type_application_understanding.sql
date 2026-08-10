-- Register the two closed Application Understanding specialists in the legacy
-- tracking enum. Runtime-memory worker authority remains role/text based; this
-- additive enum expansion only restores agent_call/msg_log observability.

ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'application_understanding_shard_modeler';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'application_understanding_company_synthesizer';
