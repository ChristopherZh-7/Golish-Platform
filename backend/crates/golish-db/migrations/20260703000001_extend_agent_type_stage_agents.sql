-- Extend agent_type enum with the stage-run / pipeline sub-agent IDs that were
-- added after 20260412000002 but never registered as enum values.
--
-- Symptom this fixes: `record_agent_call_impl` / `record_msg_log_impl`
-- (golish-agent-app tracking_bridge) cast agent ids to `agent_type`; ids such
-- as `recon` / `prober` / `enumerator` had no enum value, so every stage-run
-- sub-agent tracking write failed with:
--   [db-track] agent_call: error ... invalid input value for enum agent_type: "recon"
-- (105 occurrences across recent runs). Pentest主流程不受影响，但 agent_logs /
-- msg_logs 缺行、日志刷错。
--
-- AGENTS.md I10 (expand-first / 向后兼容): ADD VALUE IF NOT EXISTS 是纯 additive，
-- 可安全 replay，先扩枚举再上依赖它的代码。PostgreSQL 支持 ALTER TYPE ADD VALUE。

ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'recon';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'prober';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'enumerator';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'browser';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'refiner';
ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'orchestrator';
