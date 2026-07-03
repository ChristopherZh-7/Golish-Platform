-- 死资产标记：targets.liveness_state（设计 docs/design/2026-07-02-dead-asset-liveness-state.md，
-- 实现计划 docs/superpowers/plans/2026-07-02-dead-asset-liveness-state.md Task 1.1 /
-- docs/superpowers/plans/2026-07-03-eas-stage-optimization.md P1）。
--
-- 目的：给每个 in-scope target 一个一等、持久化的「存活态」，让 EAS 探活后能把死资产标下来，
-- 下游 enumeration / vuln_triage 不再对已确认死亡的资产灌覆盖率分母、不再浪费工具，前端可显示
-- 活 / 死 / 不可达 / 未探。
--
-- I10 expand-first：nullable、无 default。NULL = 未探（unknown）。值域 alive|dead|unreachable，
-- 用 CHECK 约束守卫（允许 NULL）。liveness_reason 记失败细分（dns_fail|timeout|conn_refused|
-- no_service 等，可空）。本迁移只加列 + 一次性回填（inert，读路径不依赖），P2 才写、P3 才读。
-- 可安全 replay（IF NOT EXISTS）/ 回滚（DROP COLUMN，无代码引用时）。

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS liveness_state  TEXT,
  ADD COLUMN IF NOT EXISTS liveness_reason TEXT;

ALTER TABLE targets DROP CONSTRAINT IF EXISTS targets_liveness_state_check;
ALTER TABLE targets ADD CONSTRAINT targets_liveness_state_check
  CHECK (liveness_state IS NULL OR liveness_state IN ('alive', 'dead', 'unreachable'));

-- 一次性回填：只对「已探过」(liveness_checked_at 非空) 的行推导初值；未探过的行保持 NULL
-- （未知，绝不假装死，符合 I8「已检查为空 ≠ 未检查」）。alive 判据与
-- coverage_truth::build_liveness_values_sql 完全一致：http_status 非空 OR real_ip 非空 OR
-- ports 里有 state=open 的端口。历史行分不清「探了无回应」与「解析失败」，保守归 dead。
UPDATE targets
SET liveness_state = CASE
    WHEN http_status IS NOT NULL
      OR real_ip <> ''
      OR EXISTS (
          SELECT 1 FROM jsonb_array_elements(ports) p
          WHERE COALESCE(p->>'state', 'open') = 'open'
      )
    THEN 'alive'
    ELSE 'dead'
  END
WHERE liveness_checked_at IS NOT NULL
  AND liveness_state IS NULL;

CREATE INDEX IF NOT EXISTS idx_targets_liveness_state
  ON targets(liveness_state) WHERE liveness_state IS NOT NULL;
