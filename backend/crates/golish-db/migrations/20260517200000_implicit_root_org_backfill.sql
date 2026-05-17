-- ============================================================================
-- Schema E · 隐式组织 backfill
--
-- Schema D 中 pentest 项目的 organization_id 永远 NULL，redteam 项目才在
-- organizations 表里有节点。Schema E 把这条边界拆掉：每个项目至少 1 个 root
-- org，pentest 项目就是「单 root org、无 children」的特例。
--
-- 这个 migration 一次性完成两件事：
--   1. 为所有出现在 targets 表里但 organizations 表里没有 root org 的
--      project_path 建一个 root org。名字用 project_path 的尾段（如
--      `/home/u/golish-platform/proj-x` → `proj-x`），尾段为空时回退
--      到 `Default`。后续用户可在 OrganizationsPanel 重命名。
--   2. 把所有 organization_id IS NULL 的 target backfill 到对应项目
--      的第一个 root org。
--
-- 配合 `docs/design/2026-05-17-targets-organization-grouping.md §E`。
-- ============================================================================

-- ── Part 1: 给每个仍缺 root org 的 project_path 插入隐式 root org ──────────

INSERT INTO organizations (project_path, name, parent_id)
SELECT
    pp,
    COALESCE(NULLIF(SUBSTRING(pp FROM '[^/\\]+$'), ''), 'Default'),
    NULL
FROM (
    SELECT DISTINCT COALESCE(project_path, '') AS pp
    FROM targets
    WHERE COALESCE(project_path, '') <> ''
) p
WHERE NOT EXISTS (
    SELECT 1 FROM organizations o
    WHERE o.project_path = p.pp
      AND o.parent_id IS NULL
)
ON CONFLICT DO NOTHING;

-- ── Part 2: backfill targets.organization_id → 项目的第一个 root org ───────

UPDATE targets t
SET organization_id = root.id
FROM (
    SELECT DISTINCT ON (project_path) id, project_path
    FROM organizations
    WHERE parent_id IS NULL
    ORDER BY project_path, created_at, sort_order
) root
WHERE t.organization_id IS NULL
  AND COALESCE(t.project_path, '') = root.project_path
  AND COALESCE(t.project_path, '') <> '';
