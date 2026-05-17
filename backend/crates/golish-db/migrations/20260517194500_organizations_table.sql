-- ============================================================================
-- S3 · organizations 表 + 从 targets.grp 平滑迁移
-- 配合 docs/design/2026-05-17-targets-organization-grouping.md §S3 范围
-- ============================================================================

-- ── Part 1: organizations 多级组织表 ───────────────────────────────────────
-- 自引用 FK 实现树形结构（parent_id NULL 表示根节点）。
-- 同一 (project_path, parent_id) 下 name 不可重复。

CREATE TABLE IF NOT EXISTS organizations (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_path TEXT NOT NULL DEFAULT '',
    name         TEXT NOT NULL,
    parent_id    UUID REFERENCES organizations(id) ON DELETE CASCADE,
    description  TEXT NOT NULL DEFAULT '',
    owner        TEXT NOT NULL DEFAULT '',
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 唯一约束：同父节点 + 同 project 下 name 不重；用 expression UNIQUE
-- 处理 parent_id IS NULL 的情况
CREATE UNIQUE INDEX IF NOT EXISTS uq_orgs_root_name
  ON organizations(project_path, name)
  WHERE parent_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_orgs_child_name
  ON organizations(project_path, parent_id, name)
  WHERE parent_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_orgs_project ON organizations(project_path);
CREATE INDEX IF NOT EXISTS idx_orgs_parent  ON organizations(parent_id);

-- ── Part 2: targets 关联到 organization ────────────────────────────────────

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_targets_organization ON targets(organization_id);

-- ── Part 3: 平滑迁移 grp → organizations ───────────────────────────────────
-- 简化策略：每个 (project_path, grp) 唯一对组合作为一个根节点 organization，
-- grp 字符串中的 / 暂不递归拆分（避免重复 INSERT 复杂度）。用户后续可在
-- OrganizationsPanel 中手动重组织。
-- grp = '' 或 'default' 视为未分组，不创建 organization。

INSERT INTO organizations (project_path, name)
SELECT DISTINCT
    COALESCE(project_path, ''),
    grp
FROM targets
WHERE grp IS NOT NULL
  AND grp != ''
  AND grp != 'default'
ON CONFLICT DO NOTHING;

UPDATE targets t
SET organization_id = o.id
FROM organizations o
WHERE t.organization_id IS NULL
  AND t.grp = o.name
  AND COALESCE(t.project_path, '') = o.project_path
  AND o.parent_id IS NULL;
