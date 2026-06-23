-- #4（评审 claim #4）DB-truth 带 source + E3 technique_outcomes 物化表。
-- 设计 docs/design/2026-06-23-technique-outcomes-provenance.md（建于 2026-06-18
--   E3 §3.3 之上；D0「建独立物化表」用户 2026-06-18 已拍板）。
--
-- coverage gate 的单一真值源 + provenance：每 (run × asset × technique) 的当前覆盖
-- 态 + 来源（哪个 provider、哪条 query、何时采、置信几何）。
--
-- I10 安全：纯新增表，不动既有表/列；audit_log 三列 + coverage_truth 业务表 union
-- 仍为 append-only 底座（gate 读路径后续 PR-D 灰度切换）。本 migration 只建表
-- （I10 第 1 步）——写/读路径在后续 PR 接入，表先 inert 落地，零行为变化。

CREATE TABLE IF NOT EXISTS technique_outcomes (
    id              BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,  -- I2 org 隔离
    run_id          TEXT NOT NULL,                    -- session/stage_run 键 = run 隔离（freshness）
    asset           TEXT NOT NULL,                    -- canonical_asset_key().key（E1 规范键）
    technique       TEXT NOT NULL,                    -- 注册 technique id（GOLISH-INTEL-* 等）
    outcome         TEXT NOT NULL,                    -- 'found' | 'empty' | 'error' | 'blocked'（I8 + T2）
    -- ── #4 provenance（评审 claim #4「DB-truth 带 source」）────────────────────
    source          TEXT,                             -- 数据源/provider（crt.sh / rdap / subfinder …），NULL=未知
    query           TEXT,                             -- 实际查询/命令文本
    result_count    INTEGER,                          -- 结果条数（empty=0；NULL=未知）
    confidence      REAL,                             -- 0..1 置信度（NULL=未知）
    -- ── E3 §3.3 既有列 ───────────────────────────────────────────────────────
    evidence_ids    BIGINT[] NOT NULL DEFAULT '{}',   -- 指向 audit_log 真实行（I7 可追溯）
    seq             BIGINT NOT NULL,                  -- 本 run 内落库序号（D2：每 run 从 1 自增）
    collected_at    TIMESTAMPTZ,                      -- 该维实际采集时刻（freshness；NULL=未知）
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, asset, technique)                 -- 每维一行；重跑同维 = upsert，幂等不堆叠
);

-- run 隔离读（per-org 本 run 覆盖态）。
CREATE INDEX IF NOT EXISTS idx_technique_outcomes_run
    ON technique_outcomes (run_id, organization_id);

-- coverage join（asset × technique，org 隔离）+ 真值读取用。
CREATE INDEX IF NOT EXISTS idx_technique_outcomes_join
    ON technique_outcomes (organization_id, asset, technique);

COMMENT ON TABLE technique_outcomes IS
    'Per (run x asset x technique) coverage outcome + provenance (review claim #4 / E3, design 2026-06-23-technique-outcomes-provenance). Becomes the coverage gate single source of truth once the read path migrates (PR-D); audit_log 3-cols + coverage_truth remain the append-only base. asset = canonical_asset_key; outcome in found|empty|error|blocked.';
