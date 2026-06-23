-- #5（评审 claim #5）Source / Query Log 层：证明「哪些数据源查过」。
-- 设计 docs/design/2026-06-23-source-query-log.md（消费模型 A 审计/provenance-only，
--   用户 2026-06-23 拍板：reviewer/报告读，coverage gate 不 block）。
--
-- coverage（asset × technique）回答「这格 found 没」；technique_outcomes（#4）回答
-- 「哪个 provider/query 让这格 found」但每 (asset × technique) 只一行（UNIQUE 收口，
-- 多源塌成一行）。本表更细：每 (run × source × query × target) 一行，记录每一条被动
-- 情报数据源查询的结果态 + 计数 + 用时 + 证据，用来证明「我查过 CT/WHOIS/OSINT/代码
-- 平台——但为空/失败/无凭证」。
--
-- I10 安全：纯新增表，不动既有表/列。本 migration 只建表（I10 第 1 步）——写/读路径在
-- 后续 PR 接入，表先 inert 落地，零行为变化。

CREATE TABLE IF NOT EXISTS source_query_log (
    id              BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,  -- I2 org 隔离
    run_id          TEXT NOT NULL,                    -- session/stage_run 键 = run 隔离
    source          TEXT NOT NULL,                    -- 数据源/provider（crt.sh / rdap / subfinder / ENScan_GO …）
    query           TEXT NOT NULL,                    -- 实际查询/命令文本
    target          TEXT NOT NULL DEFAULT '',         -- 被查资产 canonical_asset_key；''=org 级/非资产专属
    technique       TEXT,                             -- 贡献的 technique id（GOLISH-INTEL-*）；NULL=未映射
    status          TEXT NOT NULL,                    -- 'found' | 'empty' | 'error' | 'blocked'（同 #4 词表，承接 T2 error / I8）
    result_count    INTEGER,                          -- 结果条数（empty=0；NULL=未知）
    evidence_ids    BIGINT[] NOT NULL DEFAULT '{}',   -- 指向 audit_log 真实行（I7 可追溯）
    detail          TEXT,                             -- 备注/失败原因/无凭证说明（NULL=无）
    started_at      TIMESTAMPTZ,                      -- 查询开始时刻（NULL=未知）
    finished_at     TIMESTAMPTZ,                      -- 查询结束时刻（NULL=未知）
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, source, query, target)           -- 每 (run,源,查询,目标) 一行；重跑同查询 = upsert，幂等不堆叠
);

-- run 隔离读（per-org 本 run 查过哪些源）。
CREATE INDEX IF NOT EXISTS idx_source_query_log_run
    ON source_query_log (run_id, organization_id);

-- 源覆盖审计（按 technique 看每个源的查询态，org 隔离）。
CREATE INDEX IF NOT EXISTS idx_source_query_log_source
    ON source_query_log (organization_id, technique, source);

COMMENT ON TABLE source_query_log IS
    'Per (run x source x query x target) passive-intel data-source query log (review claim #5, design 2026-06-23-source-query-log). Proves which data sources were queried and with what result (found|empty|error|blocked) + count + timing + evidence, finer-grained than technique_outcomes which collapses to one row per (asset x technique). Consumption = audit/provenance-only: reviewer/report read it; the coverage gate does NOT block on it. Inert until write/read wiring lands in later PRs.';
