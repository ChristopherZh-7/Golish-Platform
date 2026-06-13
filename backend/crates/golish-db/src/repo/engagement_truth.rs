//! Engagement 总览的跨服务只读真值投影（设计 2026-06-13-engagement-scoping-fanout §6.4）。
//!
//! 只读地回答两个问题，供 `golish/src/engagement/`（snapshot 查询 + 薄弱度评分 +
//! 续跑判定 oracle）使用：
//! 1. 某 org 在各业务表里的真值计数（[`WeaknessCounts`]）——薄弱度评分与
//!    `org_stage_has_truth` 续跑判定的唯一输入；
//! 2. 某 project 的 org 树行集（[`list_orgs`]）——engagement snapshot 的树骨架。
//!
//! 与 `coverage_truth` 同性质：跨 recon/pentest 业务表的**只读** SELECT 聚合，
//! 服务归属上是编排关注点而非单一服务的 CRUD，故登记为 SHARED repo
//! （`scripts/check_repo_ownership.py`）。
//!
//! 红线：只读不写库；全部查询 org 隔离（`organization_id = $1`）+ `scope='in'`，
//! 不跨 org 串数据（AGENTS.md I2）。

use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Organization;
use crate::Result;

/// 各维度 DB 真值计数（org 隔离）。维度对齐 fleet 设计决策 2
/// （2026-06-12-engagement-fleet-orchestration §6.2，被 2026-06-13 重设计继承）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WeaknessCounts {
    /// 已知 CVE 命中数。本期未接 vuln→org 精确链路 → 恒 0（字段+权重已就位）。
    pub cve_hits: i64,
    /// 暴露登录面 / 管理后台（api_endpoints + directory_entries url 命中 login/admin/manage）。
    pub login_surfaces: i64,
    /// 开放端口总数（高危端口暴露的代理：面越大越值得看）。
    pub open_ports: i64,
    /// 证书存量（即将过期的精确判定列后续；当前用证书条数作代理）。
    pub certs: i64,
    /// 子域名数（面大但不等于脆弱 → 权重最低）。
    pub subdomains: i64,
}

fn build_subdomain_count_sql() -> String {
    "SELECT COUNT(*)::bigint FROM target_assets ta \
       JOIN targets t ON ta.target_id = t.id \
      WHERE t.scope::text = 'in' AND t.organization_id = $1 \
        AND ta.asset_type = 'subdomain'"
        .to_string()
}

fn build_open_ports_sum_sql() -> String {
    "SELECT COALESCE(SUM(jsonb_array_length(t.ports)), 0)::bigint FROM targets t \
      WHERE t.scope::text = 'in' AND t.organization_id = $1 \
        AND jsonb_typeof(t.ports) = 'array'"
        .to_string()
}

fn build_login_endpoints_count_sql() -> String {
    "SELECT COUNT(*)::bigint FROM api_endpoints ae \
       JOIN targets t ON ae.target_id = t.id \
      WHERE t.scope::text = 'in' AND t.organization_id = $1 \
        AND (ae.url ILIKE '%login%' OR ae.url ILIKE '%admin%' OR ae.url ILIKE '%manage%')"
        .to_string()
}

fn build_login_dirs_count_sql() -> String {
    "SELECT COUNT(*)::bigint FROM directory_entries de \
       JOIN targets t ON de.target_id = t.id \
      WHERE t.scope::text = 'in' AND t.organization_id = $1 \
        AND (de.url ILIKE '%login%' OR de.url ILIKE '%admin%' OR de.url ILIKE '%manage%')"
        .to_string()
}

fn build_certs_count_sql() -> String {
    "SELECT COALESCE(jsonb_array_length(certificates), 0)::bigint FROM organizations \
      WHERE id = $1 AND jsonb_typeof(certificates) = 'array'"
        .to_string()
}

/// 一个返回单个 i64 的 org 隔离 COUNT/SUM 查询。`$1 = org_id`。
async fn scalar_count(pool: &PgPool, sql: &str, org_id: Uuid) -> Result<i64> {
    let n: i64 = sqlx::query_scalar(sql).bind(org_id).fetch_one(pool).await?;
    Ok(n)
}

/// 查某 org 的 in-scope 真值计数（与 coverage_truth 同源表，org 隔离）。
pub async fn fetch_weakness_counts(pool: &PgPool, org_id: Uuid) -> Result<WeaknessCounts> {
    let subdomains = scalar_count(pool, &build_subdomain_count_sql(), org_id).await?;
    let open_ports = scalar_count(pool, &build_open_ports_sum_sql(), org_id).await?;
    let login_endpoints = scalar_count(pool, &build_login_endpoints_count_sql(), org_id).await?;
    let login_dirs = scalar_count(pool, &build_login_dirs_count_sql(), org_id).await?;
    // 证书列可能为 NULL / 非数组 → fetch_one 无行或类型不符时视作 0。
    let certs = scalar_count(pool, &build_certs_count_sql(), org_id)
        .await
        .unwrap_or(0);

    Ok(WeaknessCounts {
        cve_hits: 0, // 后续接 vuln→org 链路
        login_surfaces: login_endpoints + login_dirs,
        open_ports,
        certs,
        subdomains,
    })
}

/// engagement snapshot 的 org 树行集（透传 `organizations::list` 的排序语义：
/// parent NULLS FIRST → sort_order → name）。让 engagement 编排层经 SHARED repo
/// 读 org 树，不直接耦合 recon-owned 的 organizations repo（守卫合规）。
pub async fn list_orgs(pool: &PgPool, project_path: &str) -> Result<Vec<Organization>> {
    super::organizations::list(pool, project_path).await
}

/// 单 org 存在性/详情读取（worker scope 校验用，Phase B）。与 [`list_orgs`]
/// 同性质：engagement 编排层（agent-app 的 worker-scope 命令）经 SHARED repo
/// 读 org，不直接耦合 recon-owned 的 organizations repo（守卫合规）。
pub async fn get_org(pool: &PgPool, id: Uuid) -> Result<Option<Organization>> {
    super::organizations::get_one(pool, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SQL 文本守卫：org 隔离 (`organization_id = $1`) + in-scope (`scope='in'`)
    /// 是 I2 的硬前提，任何改动必须显式过这条测试。
    #[test]
    fn count_sqls_are_org_isolated_and_in_scope() {
        for sql in [
            build_subdomain_count_sql(),
            build_open_ports_sum_sql(),
            build_login_endpoints_count_sql(),
            build_login_dirs_count_sql(),
        ] {
            assert!(sql.contains("t.organization_id = $1"), "org filter: {sql}");
            assert!(sql.contains("t.scope::text = 'in'"), "in-scope: {sql}");
        }
        // 证书查 organizations 自身行，按 id 隔离。
        let certs = build_certs_count_sql();
        assert!(certs.contains("WHERE id = $1"));
    }

    #[test]
    fn login_surface_sql_matches_both_url_sources() {
        assert!(build_login_endpoints_count_sql().contains("api_endpoints"));
        assert!(build_login_dirs_count_sql().contains("directory_entries"));
        for sql in [
            build_login_endpoints_count_sql(),
            build_login_dirs_count_sql(),
        ] {
            for kw in ["%login%", "%admin%", "%manage%"] {
                assert!(sql.contains(kw), "missing {kw}: {sql}");
            }
        }
    }
}
