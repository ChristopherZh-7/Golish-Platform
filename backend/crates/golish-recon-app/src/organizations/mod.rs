//! Organizations Tauri commands.
//!
//! 组织 = 甲方资产情报库。§S3 起为多级树形结构（`parent_id` 自引用），
//! 2026-05-17 的 profile 升级（migration `_organizations_profile_fields`）
//! 在原 8 个基础字段之外加了 18 个 profile 字段，把组织从「一个名字」
//! 升级为 HVV 攻击方需要的资产情报库。
//!
//! 字段分组（与前端 5-tab UI 对应）：
//!   基础 tab : aliases / industry / tier / credit_code
//!   域名 tab : domains
//!   网络 tab : ip_ranges / asns / email_domains
//!   范围 tab : scope_rules
//!   其他 tab : intel / notes
//!   二期    : certificates / subsidiaries / business_systems / cloud_assets
//!            / github_orgs / social_accounts / historical_vulns / contacts
//!
//! 注意：`grp` 字段在 §S1 的字符串分级仍兼容保留作为回退；新建 target 可以
//! 直接关联 `organization_id`。
//!
//! 子模块：[`types`]（wire 类型）、[`candidates`]（engagement 候选读写）、
//! [`validation`]（profile patch 校验）。

use golish_app_core::DbState;
use golish_app_core::GolishError;
use uuid::Uuid;

mod artifact_cleanup;
mod candidates;
mod types;
mod validation;

#[cfg(test)]
mod tests;

pub use types::*;

use candidates::read_candidates_from_intel;
pub use candidates::upsert_organization_candidates_for_org;
use validation::validate_profile_patch;

fn to_org(o: golish_db::models::Organization) -> Organization {
    Organization {
        id: o.id.to_string(),
        project_path: o.project_path,
        name: o.name,
        parent_id: o.parent_id.map(|u| u.to_string()),
        description: o.description,
        owner: o.owner,
        sort_order: o.sort_order,
        aliases: o.aliases,
        industry: o.industry,
        tier: o.tier,
        credit_code: o.credit_code,
        domains: o.domains,
        ip_ranges: o.ip_ranges,
        asns: o.asns,
        email_domains: o.email_domains,
        scope_rules: o.scope_rules,
        intel: o.intel,
        notes: o.notes,
        certificates: o.certificates,
        subsidiaries: o.subsidiaries,
        business_systems: o.business_systems,
        cloud_assets: o.cloud_assets,
        github_orgs: o.github_orgs,
        social_accounts: o.social_accounts,
        historical_vulns: o.historical_vulns,
        contacts: o.contacts,
        created_at: o.created_at.timestamp() as u64,
        updated_at: o.updated_at.timestamp() as u64,
    }
}

#[tauri::command]
pub async fn organization_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<Organization>, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let rows = golish_db::repo::organizations::list(pool, pp).await?;
    Ok(rows.into_iter().map(to_org).collect())
}

#[tauri::command]
pub async fn organization_get(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, uid)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_create(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    name: String,
    parent_id: Option<String>,
    description: Option<String>,
    owner: Option<String>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let pid: Option<Uuid> = parent_id.and_then(|s| s.parse().ok());
    let row = golish_db::repo::organizations::create(
        pool,
        pp,
        name.trim(),
        pid,
        description.as_deref().unwrap_or(""),
        owner.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_update(
    state: tauri::State<'_, DbState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    owner: Option<String>,
    sort_order: Option<i32>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    let row = golish_db::repo::organizations::update(
        pool,
        uid,
        name.as_deref(),
        description.as_deref(),
        owner.as_deref(),
        sort_order,
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_move(
    state: tauri::State<'_, DbState>,
    id: String,
    new_parent_id: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    let new_parent: Option<Uuid> = new_parent_id.and_then(|s| s.parse().ok());
    golish_db::repo::organizations::move_to(pool, uid, new_parent).await?;
    Ok(())
}

#[tauri::command]
pub async fn organization_delete(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    artifact_cleanup::cleanup_before_delete(pool, uid).await?;
    golish_db::repo::organizations::delete(pool, uid).await?;
    Ok(())
}

#[tauri::command]
pub async fn organization_update_profile(
    state: tauri::State<'_, DbState>,
    id: String,
    patch: OrganizationProfilePatch,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;

    let errs = validate_profile_patch(&patch);
    if !errs.is_empty() {
        let summary: String = errs
            .iter()
            .map(|(f, v, r)| format!("{f}=`{v}` → {r}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(GolishError::Validation(summary));
    }

    let row = golish_db::repo::organizations::update_profile(pool, uid, &patch.into())
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_candidates_list(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<OrganizationCandidates, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, uid)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(read_candidates_from_intel(&row.intel))
}

#[tauri::command]
pub async fn organization_candidates_upsert(
    state: tauri::State<'_, DbState>,
    id: String,
    candidates: Vec<OrganizationCandidate>,
) -> Result<OrganizationCandidates, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;
    upsert_organization_candidates_for_org(pool, uid, candidates).await
}
