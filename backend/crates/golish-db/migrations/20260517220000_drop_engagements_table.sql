-- ============================================================================
-- DROP engagements 表
--
-- 2026-05-17 用户判定项目元信息（HVV 名/时间窗/团队/客户）与新升级的
-- organization 资产情报库职责重复，决定直接废除。前端 ProjectInfoPanel、
-- API client、Tauri commands、repo、model 已同步删除。
--
-- 影响：
--   - 已填写过 engagement 数据的项目会丢失这些字段（hvv_name / team_members
--     / start_at / end_at / notes）。如果某个具体项目仍需保留这些信息，请
--     先手动 SELECT 备份再升级。
--   - 表上的 idx_engagements_active 索引随表一起 DROP。
--   - 历史 migration `20260517193500_targets_owner_engagements.sql` 中的
--     Part 1（targets.owner / time_window_*）保留——这部分被 target 数据
--     模型继续使用。
-- ============================================================================

DROP INDEX IF EXISTS idx_engagements_active;
DROP TABLE IF EXISTS engagements;
