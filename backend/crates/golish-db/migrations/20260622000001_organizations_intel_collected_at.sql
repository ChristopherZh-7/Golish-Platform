-- intel 阶段每维精确新鲜度（设计 docs/design/2026-06-22-intel-perdim-freshness-slim-deliverable.md §3.2/§4）。
--
-- 背景：coverage gate 的 DB-truth 投影（golish-db coverage_truth.rs build_org_intel_presence_sql）
-- 是纯 presence SELECT，无时间窗——organizations 上一条「上次遗留」的 asns / certificates /
-- whois / intel 行就能让本次 stage-run 直接判 found（execute.rs db_truth_facts，哨兵 evidence_id=0、
-- 无 run 锚）。本迁移给 4 个 org 级情报维度各加一个 per-维度采集时间戳，读路径据此只投影
-- 「本次 stage-run（operation_state.stage_started_at）之后采到的」数据。
--
-- 维度 → 列映射（见 plan 2026-06-22 Phase 0 写点清单）：
--   ASN   → asns_collected_at          （写面 A1 append_string_array col=asns / B1 ProfileAccumulator / C1 patch）
--   CT    → certificates_collected_at  （写面 A2 append_object_with_value col=certificates / B1 / C2）
--   WHOIS → whois_collected_at         （写面 A3 merge_whois / B2 land_whois）
--   OSINT → osint_collected_at         （写面 A4-A7 contacts/social/business/intel.records / B1 / C3）
--
-- AGENTS.md I10（expand-first）：nullable、无 default。
--   NULL = 历史未知 = 「这次没采」（读路径 `<col>_collected_at >= run_start` 对 NULL 为 false，
--   保守不放松 gate，符合 I8「已检查为空 ≠ 未检查」）。
--   **禁止** DEFAULT NOW()，否则旧行会被误判成本次新采。
-- 可安全 replay（IF NOT EXISTS）/ 回滚（DROP COLUMN，无代码引用时）。镜像
-- 20260612000003_organizations_whois.sql 的 nullable additive 风格。
ALTER TABLE organizations
  ADD COLUMN IF NOT EXISTS asns_collected_at         TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS certificates_collected_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS whois_collected_at        TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS osint_collected_at        TIMESTAMPTZ;
