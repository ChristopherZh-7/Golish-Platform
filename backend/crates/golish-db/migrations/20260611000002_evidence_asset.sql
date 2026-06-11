-- Coverage 投影的资产维度 (设计 2026-06-11-coverage-auto-derive-from-evidence §5.2)。
-- 投影格是 (asset × technique)，但 audit_log.subject 对 shell/pentest_run 证据行
-- 存的是命令行字符串（"dig moresec.cn A"），不是干净资产 —— 投影需要由确定性
-- 解析器抽出的主资产（域名/IP）。同 evidence_technique：nullable、不回填、
-- 不进 detail JSON（哈希链输入不变，I10 安全）。NULL = 解析不出（不参与投影）。

ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS evidence_asset TEXT;

COMMENT ON COLUMN audit_log.evidence_asset IS
    'Primary asset (domain/IP) this evidence row tested, extracted deterministically from the tool invocation; NULL = unparsable/unmapped (row excluded from coverage projection, design 2026-06-11)';
