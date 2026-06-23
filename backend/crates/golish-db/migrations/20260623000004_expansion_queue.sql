-- #6（评审 claim #6）Expansion Queue：证明「新线索有没有继续追」。
-- 设计 docs/design/2026-06-23-expansion-queue.md（消费模型 A 审计/reviewer-only，
--   用户 2026-06-23 拍板：reviewer/报告读 pending 线索，coverage gate 不 block）。
--
-- 被动收集的完整性还取决于「发现的新线索（子公司 / 新域名 / github org / email 域 …）
-- 有没有递归深挖」。本表登记每条发现的待扩展线索 + 其处理态，reviewer/run_tree.py
-- 据此报「高置信 pending 线索没追完」。future B（gate 强制：pending 高置信线索未处理
-- 则不能 complete）= 另立设计的灰度增量；本表的 status/processed_at 列已为其预留。
--
-- I10 安全：纯新增表，不动既有表/列。本 migration 只建表（I10 第 1 步）——入队写路径 +
-- reviewer 读在后续 PR/同轮接入，表先 inert 落地，零行为变化。

CREATE TABLE IF NOT EXISTS expansion_queue (
    id              BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,  -- I2 org 隔离
    run_id          TEXT NOT NULL,                    -- session/stage_run 键 = run 隔离
    lead_type       TEXT NOT NULL,                    -- new_domain | brand | app | github_org | subsidiary | email_domain
    lead_value      TEXT NOT NULL,                    -- 线索值（公司名 / 域名 / org 名 …）
    source          TEXT,                             -- 线索发现处（recon_discover_subsidiaries / enrich provider …）
    confidence      REAL,                             -- 0..1 置信度（NULL=未知）
    status          TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'processed' | 'skipped' | 'blocked'（B 预留）
    evidence_ids    BIGINT[] NOT NULL DEFAULT '{}',   -- 指向 audit_log 真实行（I7 可追溯）
    detail          TEXT,                             -- 备注（NULL=无）
    discovered_at   TIMESTAMPTZ,                      -- 线索发现时刻（NULL=未知）
    processed_at    TIMESTAMPTZ,                      -- 线索处理时刻（B 预留；NULL=未处理）
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, lead_type, lead_value)            -- 每 (run,类型,值) 一条；重复发现 = upsert，幂等不堆叠
);

-- run 隔离读（per-org 本 run 发现哪些线索）。
CREATE INDEX IF NOT EXISTS idx_expansion_queue_run
    ON expansion_queue (run_id, organization_id);

-- pending 线索审计（按 status/type 看待追线索，org 隔离）。
CREATE INDEX IF NOT EXISTS idx_expansion_queue_pending
    ON expansion_queue (organization_id, status, lead_type);

COMMENT ON TABLE expansion_queue IS
    'Per (run x lead_type x lead_value) expansion-lead queue (review claim #6, design 2026-06-23-expansion-queue). Tracks discovered leads (subsidiary / new_domain / github_org / email_domain / ...) pending recursive recon so a reviewer can prove high-confidence leads were followed up. Consumption = audit/reviewer-only: reviewer/report read it; the coverage gate does NOT block on it (status/processed_at reserved for a future gate-enforced opt-in). Inert until write/read wiring lands.';
