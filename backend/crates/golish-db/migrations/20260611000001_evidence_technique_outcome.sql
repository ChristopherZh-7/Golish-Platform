-- Coverage = 证据账本投影 (设计 2026-06-11-coverage-auto-derive-from-evidence, D-store 法1)
-- 给 evidence 行加 (technique, outcome) 两个 nullable 列：coverage 矩阵从这些
-- 事实投影派生（Found / CheckedEmpty），模型不再手写矩阵。
-- I10 安全：纯加列、不回填、不进 detail JSON（哈希链输入不变，verify 不受影响）。

ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS evidence_technique TEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS evidence_outcome   TEXT;

COMMENT ON COLUMN audit_log.evidence_technique IS
    'Registered technique id (GOLISH-*/WSTG-*) this evidence proves; NULL for non-evidence rows or unmapped tools (coverage projection, design 2026-06-11)';
COMMENT ON COLUMN audit_log.evidence_outcome IS
    'found|empty — whether the technique run produced results or ran-but-empty (I8: checked-empty is a recorded fact, never inferred); NULL = unknown/legacy';

-- 投影查询按 (session, technique) 取证据事实；部分索引只覆盖带标注的 evidence 行。
CREATE INDEX IF NOT EXISTS idx_audit_log_evidence_facts
    ON audit_log (session_id, evidence_technique)
    WHERE audit_role = 'evidence' AND evidence_technique IS NOT NULL;
