//! engagement 快照查询命令（「范围已锁定」信号 + 总览读模型）。
//!
//! scoping 在 chat 里锁定范围（org 树落库）后，前端经本命令读快照：org 树
//! （organizations 表）+ per-org 薄弱度（weakness）+ 覆盖状态
//! （org_stage_has_truth → Passed / 否则 Pending）。运行时终态（Blocked/Failed）
//! 不落库，快照里恒 0；Phase B 的前端会话工人池持有活跑会话的运行时态，
//! 渲染时覆盖到快照之上（设计 2026-06-13 §10「运行时态 vs DB 真值」）。

use std::collections::HashMap;

use golish_app_core::{DbState, GolishError};
use uuid::Uuid;

use golish_agent_kit::harness::StageKind;

use crate::engagement::contract::{
    EngagementSnapshot, OrgRunStatusDto, OrgTreeNode, OrgWeaknessScore,
};
use crate::engagement::scheduler::FleetMode;
use crate::engagement::weakness::{
    fetch_weakness_counts, org_stage_has_truth, weakness_score, WeaknessWeights,
};

/// 从 org 的 `intel.asset_intel_discovery.ownershipPercent` 取持股比例（Phase 2 写入）。
fn extract_ownership(intel: &serde_json::Value) -> Option<f64> {
    intel
        .get("asset_intel_discovery")?
        .get("ownershipPercent")?
        .as_f64()
}

/// 把扁平 org 列表 + per-org 状态/薄弱度装配成根森林（纯函数，可单测）。
pub(crate) fn build_tree(
    orgs: &[golish_db::models::Organization],
    status: &HashMap<Uuid, OrgRunStatusDto>,
    weakness: &HashMap<Uuid, OrgWeaknessScore>,
) -> Vec<OrgTreeNode> {
    fn node_for(
        org: &golish_db::models::Organization,
        all: &[golish_db::models::Organization],
        status: &HashMap<Uuid, OrgRunStatusDto>,
        weakness: &HashMap<Uuid, OrgWeaknessScore>,
    ) -> OrgTreeNode {
        let children = all
            .iter()
            .filter(|o| o.parent_id == Some(org.id))
            .map(|c| node_for(c, all, status, weakness))
            .collect();
        OrgTreeNode {
            organization_id: org.id.to_string(),
            name: org.name.clone(),
            parent_id: org.parent_id.map(|u| u.to_string()),
            ownership_percent: extract_ownership(&org.intel),
            in_scope: true,
            status: status
                .get(&org.id)
                .copied()
                .unwrap_or(OrgRunStatusDto::Pending),
            weakness: weakness.get(&org.id).cloned(),
            children,
        }
    }
    orgs.iter()
        .filter(|o| o.parent_id.is_none())
        .map(|root| node_for(root, orgs, status, weakness))
        .collect()
}

/// GUI · 读 engagement 快照（org 树 + 覆盖状态 + 薄弱度）。`to_stage` 决定覆盖判定看
/// 哪个阶段的真值（默认 target_intel）；`mode` 只回显（前端按它排序，不影响数据）。
#[tauri::command]
pub async fn engagement_get_snapshot(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    mode: Option<String>,
    to_stage: Option<String>,
) -> Result<EngagementSnapshot, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let to = to_stage
        .as_deref()
        .and_then(StageKind::try_parse)
        .unwrap_or(StageKind::TargetIntel);

    let orgs = golish_db::repo::engagement_truth::list_orgs(pool, pp).await?;

    let weights = WeaknessWeights::default();
    let mut status: HashMap<Uuid, OrgRunStatusDto> = HashMap::new();
    let mut weakness: HashMap<Uuid, OrgWeaknessScore> = HashMap::new();
    for o in &orgs {
        let covered = org_stage_has_truth(pool, o.id, to).await.unwrap_or(false);
        status.insert(
            o.id,
            if covered {
                OrgRunStatusDto::Passed
            } else {
                OrgRunStatusDto::Pending
            },
        );
        if let Ok(c) = fetch_weakness_counts(pool, o.id).await {
            weakness.insert(
                o.id,
                OrgWeaknessScore {
                    organization_id: o.id.to_string(),
                    cve_hits: c.cve_hits,
                    login_surfaces: c.login_surfaces,
                    open_ports: c.open_ports,
                    certs: c.certs,
                    subdomains: c.subdomains,
                    total: weakness_score(&c, &weights),
                    note: None,
                },
            );
        }
    }

    let total_orgs = orgs.len();
    let root_count = orgs.iter().filter(|o| o.parent_id.is_none()).count();
    let covered = status
        .values()
        .filter(|s| {
            matches!(
                s,
                OrgRunStatusDto::Passed | OrgRunStatusDto::SkippedAlreadyComplete
            )
        })
        .count();
    let tree = build_tree(&orgs, &status, &weakness);

    Ok(EngagementSnapshot {
        project_path: pp.to_string(),
        mode: mode
            .as_deref()
            .and_then(FleetMode::parse)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| FleetMode::Checklist.as_str().to_string()),
        root_count,
        total_orgs,
        covered,
        blocked: 0,
        failed: 0,
        tree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org(name: &str, id: Uuid, parent: Option<Uuid>) -> golish_db::models::Organization {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "project_path": "/tmp/p",
            "name": name,
            "parent_id": parent,
            "description": "",
            "owner": "",
            "sort_order": 0,
            "created_at": "2026-06-12T00:00:00Z",
            "updated_at": "2026-06-12T00:00:00Z",
        }))
        .expect("test org deserializes")
    }

    #[test]
    fn build_tree_nests_children_under_roots() {
        let root_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let orgs = vec![
            org("root", root_id, None),
            org("child", child_id, Some(root_id)),
        ];
        let mut status = HashMap::new();
        status.insert(root_id, OrgRunStatusDto::Passed);
        status.insert(child_id, OrgRunStatusDto::Pending);
        let tree = build_tree(&orgs, &status, &HashMap::new());
        assert_eq!(tree.len(), 1, "one root");
        assert_eq!(tree[0].name, "root");
        assert!(matches!(tree[0].status, OrgRunStatusDto::Passed));
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "child");
        assert!(matches!(
            tree[0].children[0].status,
            OrgRunStatusDto::Pending
        ));
    }

    #[test]
    fn extract_ownership_reads_discovery_field() {
        let intel = serde_json::json!({
            "asset_intel_discovery": { "ownershipPercent": 65.0 }
        });
        assert_eq!(extract_ownership(&intel), Some(65.0));
        assert_eq!(extract_ownership(&serde_json::json!({})), None);
    }
}
