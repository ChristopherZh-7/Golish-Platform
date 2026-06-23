-- EAS/enumeration 阶段每维精确新鲜度（设计 docs/design/2026-06-22-intel-perdim-freshness-slim-deliverable.md Phase D）。
--
-- 背景：coverage gate 的 EAS DB-truth 投影（golish-db coverage_truth.rs
-- build_port_values_sql / build_liveness_values_sql / build_ipwhois_values_sql）
-- 读 targets.ports / http_status·real_ip / ip_whois 这些**可变列**——它们没有
-- per-采集时间戳，organizations 上一条「上次遗留」的端口/活性/IP-WHOIS 结果就能让
-- 本次 stage-run 直接判 found（无 run 锚）。本迁移给这 3 个 EAS 维度各加一个
-- per-维度采集时间戳，读路径据此只投影「本次 stage-run
-- （operation_state.stage_started_at）之后采到的」数据。
--
-- 维度 → 列映射（见 plan 2026-06-22 Phase 0 EAS 写点清单）：
--   PORT     → ports_scanned_at        （写点 update_ports_by_id / update_recon_extended_by_id /
--                                        output_store/targets / recon-app GUI cmds）
--   LIVENESS → liveness_checked_at     （写点 update_recon_extended_by_id / output_store/targets /
--                                        set_real_ip_by_id / backfill_real_ip_from_dns）
--   IPWHOIS  → ip_whois_collected_at   （写点 set_ip_whois_by_id）
--
-- 行级 EAS 维度（SERVICE-FINGERPRINT / DIR / PARAM / JSAPI）用各自子表已有的行时间戳
-- （fingerprints.detected_at / directory_entries.created_at / api_endpoints.discovered_at），
-- **无需**本迁移加列。
--
-- AGENTS.md I10（expand-first）：nullable、无 default。
--   NULL = 历史未知 = 「这次没采」（读路径 `<col> >= run_start` 对 NULL 为 false，
--   保守不放松 gate，符合 I8「已检查为空 ≠ 未检查」）。
--   **禁止** DEFAULT NOW()，否则旧行会被误判成本次新采。
-- 可安全 replay（IF NOT EXISTS）/ 回滚（DROP COLUMN，无代码引用时）。镜像
-- 20260622000001_organizations_intel_collected_at.sql 的 nullable additive 风格。
ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS ports_scanned_at      TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS liveness_checked_at   TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS ip_whois_collected_at TIMESTAMPTZ;
