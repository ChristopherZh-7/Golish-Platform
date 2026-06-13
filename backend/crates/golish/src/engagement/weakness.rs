//! org 薄弱度评分 + 续跑判定 oracle（原 fleet F3，纯逻辑搬回；设计
//! 2026-06-12-engagement-fleet-orchestration §6.2 + 2026-06-13 重设计 §6.4）。
//!
//! 薄弱度分 = Σ(DB 真值计数 × 权重)，全部输入来自业务表的确定性 COUNT（org 隔离，
//! scope='in'），**不是 AI 主观分**——延续总纲「gate 只信 DB 真值」哲学。funnel 模式
//! 按此分降序排，把人力压在最可能出洞的 org 上。
//!
//! 计数 SQL 住 `golish_db::repo::engagement_truth`（SHARED repo，守卫合规）；本层
//! 只做权重表、评分纯函数与「阶段 → 真值表」映射。
//!
//! 续跑判定（[`org_stage_has_truth`]）住在这一层而非 scheduler：oracle **允许**认识
//! 阶段语义（哪个阶段看哪张表），scheduler 不允许（§4.1）——阶段知识的正确归属。

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use golish_agent_kit::harness::StageKind;

use crate::engagement::scheduler::WeaknessScorer;

pub use golish_db::repo::engagement_truth::{fetch_weakness_counts, WeaknessCounts};

/// 权重表（fleet 设计决策 2；配置化雏形：先硬编码默认，后续可提 JSON）。
/// 「能直接打」的（CVE / 登录面）权重最高，「面大」的（子域名）最低。
#[derive(Debug, Clone, Copy)]
pub struct WeaknessWeights {
    pub cve: i64,
    pub login: i64,
    pub port: i64,
    pub cert: i64,
    pub subdomain: i64,
}

impl Default for WeaknessWeights {
    fn default() -> Self {
        Self {
            cve: 10,
            login: 10,
            port: 5,
            cert: 3,
            subdomain: 1,
        }
    }
}

/// 纯函数：计数 × 权重 → 总分。可单测。
pub fn weakness_score(c: &WeaknessCounts, w: &WeaknessWeights) -> i64 {
    c.cve_hits * w.cve
        + c.login_surfaces * w.login
        + c.open_ports * w.port
        + c.certs * w.cert
        + c.subdomains * w.subdomain
}

/// 某 org 该阶段是否已有 DB 真值（续跑判定的保守近似，fail-closed）。
///
/// oracle 层允许认识阶段语义（§4.1 只约束 scheduler）：每个阶段看它的主真值表。
/// 查询出错 / 拿不准 → 返 false（不跳过 → 真跑），绝不误判已完成（I8 fail-closed）。
pub async fn org_stage_has_truth(pool: &PgPool, org_id: Uuid, to_stage: StageKind) -> Result<bool> {
    let counts = fetch_weakness_counts(pool, org_id).await?;
    Ok(match to_stage {
        // 被动情报：有子域名 / 证书即视作收集过（保守，宁可重跑也不误跳）。
        StageKind::TargetIntel => counts.subdomains > 0 || counts.certs > 0,
        // 主动攻击面：有开放端口即视作探测过。
        StageKind::ExternalAttackSurface => counts.open_ports > 0,
        // 内容枚举：有登录面 / 端点即视作枚举过。
        StageKind::Enumeration => counts.login_surfaces > 0,
        // 其它阶段不做续跑跳过（保守：总是真跑）。
        _ => false,
    })
}

/// 注入 scheduler 的薄弱度评分器。查不到 → 0（fail-soft，不影响调度推进）。
pub struct DbWeaknessScorer {
    pub db_pool: std::sync::Arc<PgPool>,
}

#[async_trait::async_trait]
impl WeaknessScorer for DbWeaknessScorer {
    async fn score(&self, org_id: Uuid) -> i64 {
        match fetch_weakness_counts(&self.db_pool, org_id).await {
            Ok(c) => weakness_score(&c, &WeaknessWeights::default()),
            Err(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_weights_cve_and_login_highest() {
        let w = WeaknessWeights::default();
        // 1 个 CVE 命中 = 10；1 个登录面 = 10；1 个子域名 = 1。
        let cve_only = weakness_score(
            &WeaknessCounts {
                cve_hits: 1,
                ..Default::default()
            },
            &w,
        );
        let subs_only = weakness_score(
            &WeaknessCounts {
                subdomains: 1,
                ..Default::default()
            },
            &w,
        );
        assert_eq!(cve_only, 10);
        assert_eq!(subs_only, 1);
        assert!(cve_only > subs_only);
    }

    #[test]
    fn score_sums_all_dimensions() {
        let w = WeaknessWeights::default();
        let c = WeaknessCounts {
            cve_hits: 2,
            login_surfaces: 1,
            open_ports: 3,
            certs: 4,
            subdomains: 5,
        };
        // 2*10 + 1*10 + 3*5 + 4*3 + 5*1 = 20+10+15+12+5 = 62
        assert_eq!(weakness_score(&c, &w), 62);
    }

    #[test]
    fn default_weights_values() {
        let w = WeaknessWeights::default();
        assert_eq!(
            (w.cve, w.login, w.port, w.cert, w.subdomain),
            (10, 10, 5, 3, 1)
        );
    }
}
