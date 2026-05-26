-- ============================================================================
-- Evidence Ledger Schema (Phase 1a · Operation Harness MVP)
--
-- 落地 docs/design/2026-05-26-evidence-ledger-on-existing-audit-log.md (Doc 1)
-- + docs/design/2026-05-26-stage-harness-mvp-external-attack-surface.md (Doc 3)
-- 中的 schema 草案。
--
-- 设计原则:
--   - 与 §13.9 / F1 不变量一致: 不造与现有重复的新表 (audit_log 加 audit_role
--     而非新建 evidence_records)
--   - bitemporal 分类层: evidence_classifications 用 valid_from/valid_to +
--     partial unique index WHERE valid_to IS NULL 保证当前活跃分类唯一
--   - 所有 ALTER 都 IF NOT EXISTS, 所有 CREATE TABLE 都 IF NOT EXISTS, idempotent
--   - 向后兼容: audit_role 默认 'action', scope_rules_version 默认 1 不破坏现有行
--
-- 影响:
--   - 7 步 ALTER/CREATE, 全部向后兼容; down-migration 见末尾注释
--   - 无数据迁移; 现有 audit_log 行自动获得 audit_role='action'
-- ============================================================================

-- ── Step 1: audit_log 加 audit_role 字段 (Doc 1 §3.1) ────────────────────────
-- audit_role ∈ {'action', 'evidence', 'classification', 'approval'}
ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS audit_role TEXT NOT NULL DEFAULT 'action';

CREATE INDEX IF NOT EXISTS audit_log_audit_role_idx
    ON audit_log(audit_role);

-- ── Step 2: organizations 加 scope_rules_version (Doc 1 §3.3) ────────────────
-- 任何 organizations.scope_rules 修改 → application 把 version +1
-- ScopeService 启动时锁定 cursor.last_scope_version 不读最新 (O7 race 解决方案)
ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS scope_rules_version BIGINT NOT NULL DEFAULT 1;

-- ── Step 3: evidence_classifications 表 · bitemporal (Doc 1 §3.2) ────────────
-- IFC 分类层 + 因果回溯
-- 当前活跃分类 = WHERE valid_to IS NULL (partial unique index 保唯一)
CREATE TABLE IF NOT EXISTS evidence_classifications (
    id BIGSERIAL PRIMARY KEY,
    evidence_audit_id BIGINT NOT NULL REFERENCES audit_log(id),
    classification TEXT NOT NULL,            -- 'in_scope' / 'out_of_scope' / 'derived_from_out_of_scope'
    scope_version BIGINT NOT NULL,           -- 对应 organizations.scope_rules_version 当时快照
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_to TIMESTAMPTZ,                    -- NULL = 当前 active
    reason TEXT NOT NULL,
    relabel_decision TEXT,                   -- validate_relabel 返回的决策代号
    classified_by_session TEXT NOT NULL,     -- 哪个 Tauri session 触发的分类
    producing_stage_run_id UUID,             -- stage-scoped 隔离 (O4 终态)
    schema_v INT NOT NULL DEFAULT 1
);

-- 唯一性: 同一 evidence 的「当前」分类只能有一条
CREATE UNIQUE INDEX IF NOT EXISTS evidence_classifications_current_idx
    ON evidence_classifications(evidence_audit_id)
    WHERE valid_to IS NULL;

-- 查询性能: 常按 producing_stage_run_id 过滤
CREATE INDEX IF NOT EXISTS evidence_classifications_stage_idx
    ON evidence_classifications(producing_stage_run_id)
    WHERE valid_to IS NULL;

-- ── Step 4: operation_state cursor 表 (Doc 1 §3.4) ──────────────────────────
-- resume 写入侧 cursor 表 (注意: 这不是 operations 表; 没有 valid_until /
-- authz_level / scope; 那些走 targets / organizations)
CREATE TABLE IF NOT EXISTS operation_state (
    operation_id UUID PRIMARY KEY,
    profile TEXT NOT NULL,
    current_stage TEXT NOT NULL,
    stage_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_evidence_audit_id BIGINT,
    last_classification_id BIGINT REFERENCES evidence_classifications(id),
    last_scope_version BIGINT,
    state_blob JSONB NOT NULL DEFAULT '{}'::jsonb,   -- harness 私有 resume 状态
    superseded_by UUID REFERENCES operation_state(operation_id)  -- cross-profile transition
);

CREATE INDEX IF NOT EXISTS operation_state_profile_stage_idx
    ON operation_state(profile, current_stage);

-- ── Step 5: stage_runs 表 (Doc 3 §7 sprint_contracts FK 前置依赖) ────────────
-- 每个 stage 的运行实例 (一个 operation 下可能多次跑同一 stage_kind)
CREATE TABLE IF NOT EXISTS stage_runs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    stage_kind TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'started',
    active_sprint_contract_id UUID  -- FK 在 Step 7 加 (避免循环依赖)
);

CREATE INDEX IF NOT EXISTS stage_runs_operation_idx
    ON stage_runs(operation_id);

CREATE INDEX IF NOT EXISTS stage_runs_kind_idx
    ON stage_runs(stage_kind);

-- ── Step 6: sprint_contracts 表 (Doc 3 §7) ──────────────────────────────────
-- Profile-driven sprint skeleton + planner LLM 填变量 → locked-at-stage-start
CREATE TABLE IF NOT EXISTS sprint_contracts (
    id UUID PRIMARY KEY,
    stage_run_id UUID NOT NULL REFERENCES stage_runs(id),
    contract_text TEXT NOT NULL,
    locked_after TIMESTAMPTZ NOT NULL,
    superseded_by UUID REFERENCES sprint_contracts(id),
    status TEXT NOT NULL DEFAULT 'active',
    planner_llm_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS sprint_contracts_stage_run_idx
    ON sprint_contracts(stage_run_id);

CREATE INDEX IF NOT EXISTS sprint_contracts_status_idx
    ON sprint_contracts(status);

-- ── Step 7: stage_runs.active_sprint_contract_id FK ─────────────────────────
-- 现在 sprint_contracts 表存在了, 把 FK 补上 (Step 5 时不能加, 因为表还没建)
-- 用 DO 块包裹 + IF NOT EXISTS-style guard, 保证 idempotent
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.table_constraints
        WHERE constraint_name = 'stage_runs_active_sprint_contract_fk'
          AND table_name = 'stage_runs'
    ) THEN
        ALTER TABLE stage_runs
            ADD CONSTRAINT stage_runs_active_sprint_contract_fk
            FOREIGN KEY (active_sprint_contract_id) REFERENCES sprint_contracts(id);
    END IF;
END $$;

-- ============================================================================
-- Down-migration 草案 (手动执行, 不在本 migration 中)
--
-- 反向顺序:
--   ALTER TABLE stage_runs DROP CONSTRAINT IF EXISTS stage_runs_active_sprint_contract_fk;
--   DROP TABLE IF EXISTS sprint_contracts;
--   DROP TABLE IF EXISTS stage_runs;
--   DROP TABLE IF EXISTS operation_state;
--   DROP TABLE IF EXISTS evidence_classifications;
--   ALTER TABLE organizations DROP COLUMN IF EXISTS scope_rules_version;
--   ALTER TABLE audit_log DROP COLUMN IF EXISTS audit_role;
--   DROP INDEX IF EXISTS audit_log_audit_role_idx;
--
-- 注意:
--   - 反向前必须确认无应用代码引用这些表/字段 (即 feature flag harness.stage_mode_enabled
--     默认 OFF 且无生产数据)
--   - audit_log.audit_role 删除会丢失 evidence/classification/approval 三类行的语义区分,
--     必须先归档到外部存储
-- ============================================================================
