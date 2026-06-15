use crate::Result;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Organization;

/// Partial update payload for the profile (18 new fields added by the
/// `_organizations_profile_fields` migration).
///
/// 调用方按需填字段；`None` = 不修改，`Some(value)` = 覆盖。这样允许
/// 前端「只改 IP 段」「只改 domain」等 PATCH 行为，不会把别的 tab 的
/// 内容清空。后端格式校验前置在 Tauri 层（参考
/// `crate::tools::organizations::validate_profile_patch`）；repo 只做存储。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfilePatch {
    // basic
    pub aliases: Option<Vec<String>>,
    pub industry: Option<String>,
    pub tier: Option<String>,
    pub credit_code: Option<String>,
    // domain / network
    pub domains: Option<serde_json::Value>,
    pub ip_ranges: Option<serde_json::Value>,
    pub asns: Option<serde_json::Value>,
    pub email_domains: Option<serde_json::Value>,
    // scope
    pub scope_rules: Option<serde_json::Value>,
    // other
    pub intel: Option<serde_json::Value>,
    pub notes: Option<String>,
    // phase 2 fields (schema-only for now; included so backend doesn't
    // break if a future UI starts sending them).
    pub certificates: Option<serde_json::Value>,
    pub subsidiaries: Option<serde_json::Value>,
    pub business_systems: Option<serde_json::Value>,
    pub cloud_assets: Option<serde_json::Value>,
    pub github_orgs: Option<serde_json::Value>,
    pub social_accounts: Option<serde_json::Value>,
    pub historical_vulns: Option<serde_json::Value>,
    pub contacts: Option<serde_json::Value>,
}

impl ProfilePatch {
    /// Returns `true` if at least one field is `Some(_)`. Repo callers
    /// use this to short-circuit the no-op update path (still loads the
    /// row to return it, but skips the UPDATE statement).
    pub fn has_any(&self) -> bool {
        self.aliases.is_some()
            || self.industry.is_some()
            || self.tier.is_some()
            || self.credit_code.is_some()
            || self.domains.is_some()
            || self.ip_ranges.is_some()
            || self.asns.is_some()
            || self.email_domains.is_some()
            || self.scope_rules.is_some()
            || self.intel.is_some()
            || self.notes.is_some()
            || self.certificates.is_some()
            || self.subsidiaries.is_some()
            || self.business_systems.is_some()
            || self.cloud_assets.is_some()
            || self.github_orgs.is_some()
            || self.social_accounts.is_some()
            || self.historical_vulns.is_some()
            || self.contacts.is_some()
    }
}

/// Fetch a single organization by id. Returns `None` when the row no
/// longer exists (e.g. parent got cascade-deleted between list and get).
pub async fn get_one(pool: &PgPool, id: Uuid) -> Result<Option<Organization>> {
    super::scoped::get_by_id(pool, "organizations", id).await
}

/// List all organizations for a project (flat, sorted by parent_id NULLs first
/// then sort_order). Callers typically rebuild the tree client-side.
pub async fn list(pool: &PgPool, project_path: &str) -> Result<Vec<Organization>> {
    let rows = sqlx::query_as::<_, Organization>(
        r#"SELECT * FROM organizations
           WHERE project_path = $1
           ORDER BY parent_id NULLS FIRST, sort_order, name"#,
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List every in-scope organization id for the harness fan-out stage-pass gate.
/// `None` project_path = whole-DB axis (chat sessions carry no project key, the
/// same legacy axis `targets` in-scope reads use); `Some(p)` filters to that
/// project. The harness uses this to verify EVERY in-scope org freshly passed a
/// fan-out stage's per-org gate before the stage closes (design 2026-06-15).
pub async fn in_scope_ids(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Uuid>> {
    let ids = match project_path {
        Some(p) => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM organizations WHERE project_path = $1 ORDER BY id",
            )
            .bind(p)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM organizations ORDER BY id")
                .fetch_all(pool)
                .await?
        }
    };
    Ok(ids)
}

fn build_subtree_ids_sql() -> String {
    // Recursive walk DOWN the self-FK parent chain: seed = the root org row,
    // then every org whose parent is already in the set. Yields root + all
    // descendants so an engagement read stays inside the scoping-confirmed tree.
    "WITH RECURSIVE subtree AS ( \
         SELECT id FROM organizations WHERE id = $1 \
         UNION ALL \
         SELECT o.id FROM organizations o \
           JOIN subtree s ON o.parent_id = s.id \
       ) SELECT id FROM subtree"
        .to_string()
}

/// Every organization id in the subtree rooted at `root_id` — the root itself
/// plus all descendants reachable via `parent_id`. Engagement-org isolation
/// (设计 2026-06-15-engagement-org-isolation) uses this to confine a fan-out /
/// in-scope read to the scoping-confirmed org and its subsidiaries, so a sibling
/// engagement's org tree (e.g. a previously-tested company left in the same
/// workspace) can never leak into the current run's scope. Returns an empty vec
/// if `root_id` does not exist.
pub async fn subtree_ids(pool: &PgPool, root_id: Uuid) -> Result<Vec<Uuid>> {
    let ids = sqlx::query_scalar::<_, Uuid>(&build_subtree_ids_sql())
        .bind(root_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

pub async fn create(
    pool: &PgPool,
    project_path: &str,
    name: &str,
    parent_id: Option<Uuid>,
    description: &str,
    owner: &str,
) -> Result<Organization> {
    let row = sqlx::query_as::<_, Organization>(
        r#"INSERT INTO organizations (project_path, name, parent_id, description, owner)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(project_path)
    .bind(name)
    .bind(parent_id)
    .bind(description)
    .bind(owner)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    owner: Option<&str>,
    sort_order: Option<i32>,
) -> Result<Organization> {
    if let Some(n) = name {
        sqlx::query("UPDATE organizations SET name=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(d) = description {
        sqlx::query("UPDATE organizations SET description=$1, updated_at=NOW() WHERE id=$2")
            .bind(d)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(o) = owner {
        sqlx::query("UPDATE organizations SET owner=$1, updated_at=NOW() WHERE id=$2")
            .bind(o)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(s) = sort_order {
        sqlx::query("UPDATE organizations SET sort_order=$1, updated_at=NOW() WHERE id=$2")
            .bind(s)
            .bind(id)
            .execute(pool)
            .await?;
    }
    let row = sqlx::query_as::<_, Organization>("SELECT * FROM organizations WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// Move an organization under a new parent (or to root with `new_parent=None`).
/// Caller is responsible for preventing cycles; we add a guard here.
pub async fn move_to(pool: &PgPool, id: Uuid, new_parent: Option<Uuid>) -> Result<()> {
    if let Some(target) = new_parent {
        if target == id {
            return Err(anyhow::anyhow!("cannot move organization under itself").into());
        }
        // Walk ancestor chain to ensure `target` is not a descendant of `id`.
        let mut cursor = Some(target);
        while let Some(cur) = cursor {
            if cur == id {
                return Err(
                    anyhow::anyhow!("cannot move organization under its own descendant").into(),
                );
            }
            cursor = sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT parent_id FROM organizations WHERE id = $1",
            )
            .bind(cur)
            .fetch_optional(pool)
            .await?
            .flatten();
        }
    }
    sqlx::query("UPDATE organizations SET parent_id=$1, updated_at=NOW() WHERE id=$2")
        .bind(new_parent)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Hard-deletes an organization. DB foreign keys cascade the rest: the org
/// self-FK (`parent_id ON DELETE CASCADE`) drops the whole sub-tree, and
/// `targets.organization_id ON DELETE CASCADE` (migration
/// `20260614000002_targets_org_cascade_delete`) deletes every target owned by
/// the org or any descendant — which in turn cascades the targets'
/// recon/scan/dns/security-analysis rows. So deleting a parent org wipes its
/// entire branch; deleting a child org wipes just that child's targets.
/// Findings/audit keep history (their `target_id` is ON DELETE SET NULL).
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "organizations", id).await?;
    Ok(())
}

/// PATCH-style profile update. Each `Some(value)` field is written;
/// `None` is skipped so partial updates from individual UI tabs don't
/// clobber unrelated fields.
///
/// Returns the freshly-loaded row even on no-op so callers always get
/// a consistent shape back. Returns `Ok(None)` if the organization
/// doesn't exist.
pub async fn update_profile(
    pool: &PgPool,
    id: Uuid,
    patch: &ProfilePatch,
) -> Result<Option<Organization>> {
    if !patch.has_any() {
        return get_one(pool, id).await;
    }

    let mut tx = pool.begin().await?;

    // Verify existence first so we can return Ok(None) on a missing row
    // rather than swallowing the missing-row error inside individual
    // statements. Cheap because we'd refetch at the end anyway.
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM organizations WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Ok(None);
    }

    // Each field is written in its own statement to keep the SQL trivial
    // and avoid building a dynamic column list. Negligible overhead for
    // a workflow that typically writes 1–5 fields per call.
    macro_rules! patch_field {
        ($field:ident, $col:literal) => {
            if let Some(ref v) = patch.$field {
                sqlx::query(concat!(
                    "UPDATE organizations SET ",
                    $col,
                    " = $1, updated_at = NOW() WHERE id = $2"
                ))
                .bind(v)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
        };
    }

    patch_field!(aliases, "aliases");
    patch_field!(industry, "industry");
    patch_field!(tier, "tier");
    patch_field!(credit_code, "credit_code");
    patch_field!(domains, "domains");
    patch_field!(ip_ranges, "ip_ranges");
    patch_field!(asns, "asns");
    patch_field!(email_domains, "email_domains");
    patch_field!(scope_rules, "scope_rules");
    patch_field!(intel, "intel");
    patch_field!(notes, "notes");
    patch_field!(certificates, "certificates");
    patch_field!(subsidiaries, "subsidiaries");
    patch_field!(business_systems, "business_systems");
    patch_field!(cloud_assets, "cloud_assets");
    patch_field!(github_orgs, "github_orgs");
    patch_field!(social_accounts, "social_accounts");
    patch_field!(historical_vulns, "historical_vulns");
    patch_field!(contacts, "contacts");

    let row = sqlx::query_as::<_, Organization>("SELECT * FROM organizations WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(Some(row))
}

fn build_find_root_id_by_name_sql() -> String {
    "SELECT id FROM organizations WHERE project_path = $1 AND name = $2 AND parent_id IS NULL LIMIT 1"
        .to_string()
}

/// Find the root (parent-less) organization id by exact name within a project.
/// Mirrors the find-or-create rule used by `store_organization_update`. `None`
/// == no such root org yet.
pub async fn find_root_id_by_name(
    pool: &PgPool,
    project_path: &str,
    name: &str,
) -> Result<Option<Uuid>> {
    let id = sqlx::query_scalar::<_, Uuid>(&build_find_root_id_by_name_sql())
        .bind(project_path)
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(id)
}

fn build_find_child_id_by_name_sql() -> String {
    "SELECT id FROM organizations WHERE project_path = $1 AND name = $2 AND parent_id = $3 LIMIT 1"
        .to_string()
}

/// Find a child organization id by exact name *under a specific parent* within a
/// project. Powers the bulk get-or-create of approved subsidiaries (so re-running
/// scoping reuses an existing child instead of duplicating it). `None` == no such
/// child yet under that parent.
pub async fn find_child_id_by_name(
    pool: &PgPool,
    project_path: &str,
    name: &str,
    parent_id: Uuid,
) -> Result<Option<Uuid>> {
    let id = sqlx::query_scalar::<_, Uuid>(&build_find_child_id_by_name_sql())
        .bind(project_path)
        .bind(name)
        .bind(parent_id)
        .fetch_optional(pool)
        .await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_root_id_by_name_sql_matches_command_layer() {
        assert_eq!(
            build_find_root_id_by_name_sql(),
            "SELECT id FROM organizations WHERE project_path = $1 AND name = $2 AND parent_id IS NULL LIMIT 1"
        );
    }

    #[test]
    fn find_child_id_by_name_sql_scopes_to_parent() {
        // Child lookup must bind parent_id (not NULL) so bulk subsidiary
        // get-or-create reuses the right child and never collides with a
        // same-named root or a child under a different parent.
        assert_eq!(
            build_find_child_id_by_name_sql(),
            "SELECT id FROM organizations WHERE project_path = $1 AND name = $2 AND parent_id = $3 LIMIT 1"
        );
    }

    #[test]
    fn subtree_ids_sql_walks_parent_chain_from_root() {
        // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
        // confine a fan-out / in-scope read to the scoping-confirmed root org
        // PLUS every descendant, via the self-FK parent chain, so it can never
        // reach a sibling engagement's org tree. Recursive seed binds $1 = root.
        let sql = build_subtree_ids_sql();
        assert!(sql.contains("WITH RECURSIVE"), "must recurse: {sql}");
        assert!(sql.contains("WHERE id = $1"), "seed binds root id: {sql}");
        assert!(
            sql.contains("o.parent_id = s.id"),
            "recursion follows parent_id down the tree: {sql}"
        );
    }
}
