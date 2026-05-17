-- ============================================================================
-- S2 · Targets 加 owner + time_window 字段 + 新增 engagements 元信息表
-- 配合 docs/design/2026-05-17-targets-organization-grouping.md §5 S2 范围
-- ============================================================================

-- ── Part 1: targets 表扩展 owner + time_window ─────────────────────────────

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS owner TEXT NOT NULL DEFAULT '';

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS time_window_start TIMESTAMPTZ;

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS time_window_end TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_targets_owner
  ON targets(owner) WHERE owner != '';

-- ── Part 2: engagements 项目元信息表 ───────────────────────────────────────
-- 一个 project_path 对应一个 engagement（如 HVV / 红队项目元信息）。
-- 选用 project_path 作为 PK（与 targets.project_path 自然 JOIN）；
-- 不加 FK 是因为项目本身在文件系统层，未必在 DB 里有对应行。

CREATE TABLE IF NOT EXISTS engagements (
    project_path TEXT PRIMARY KEY,
    hvv_name     TEXT NOT NULL DEFAULT '',
    team_members JSONB NOT NULL DEFAULT '[]',
    start_at     TIMESTAMPTZ,
    end_at       TIMESTAMPTZ,
    notes        TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Note: partial index predicates must be IMMUTABLE; NOW() is STABLE and
-- would error with "functions in index predicate must be marked IMMUTABLE".
-- Plain (start_at, end_at) index covers active-engagement lookups fine.
CREATE INDEX IF NOT EXISTS idx_engagements_active
  ON engagements(start_at, end_at);
