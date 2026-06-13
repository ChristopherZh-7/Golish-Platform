//! engagement 前端数据契约（ts-rs 导出 `frontend/lib/generated/`，I5）。
//!
//! 这些 DTO 是前端 engagement 总览（scoping 对话升级后的总览面，Phase C）的
//! **唯一类型来源**——前端禁手写镜像（I5）。内部调度逻辑用
//! `scheduler::OrgRunStatus`（紧凑 enum），序列化对外用本文件的 DTO；转换在
//! query command 层做。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 单个 org-run 的对外状态。前 4 个镜像 `scheduler::OrgRunStatus`（运行时终态）；
/// `Pending` 是快照专用——重开 / 刷新后运行时终态不可得，快照只能从 DB 真值判
/// 「已覆盖（Passed）」vs「尚未收集（Pending）」；活跃工人会话的运行时态由
/// 前端池覆盖（Phase B）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum OrgRunStatusDto {
    Passed,
    Blocked,
    Failed,
    SkippedAlreadyComplete,
    /// 快照：该 org 尚无该阶段 DB 真值（还没收集到）。
    Pending,
}

impl From<crate::engagement::scheduler::OrgRunStatus> for OrgRunStatusDto {
    fn from(s: crate::engagement::scheduler::OrgRunStatus) -> Self {
        use crate::engagement::scheduler::OrgRunStatus as S;
        match s {
            S::Passed => Self::Passed,
            S::Blocked => Self::Blocked,
            S::Failed => Self::Failed,
            S::SkippedAlreadyComplete => Self::SkippedAlreadyComplete,
        }
    }
}

/// 一个 org 的薄弱度评分（各维 DB 真值计数 + 加权总分 + 可空 AI 评注）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrgWeaknessScore {
    pub organization_id: String,
    pub cve_hits: i64,
    pub login_surfaces: i64,
    pub open_ports: i64,
    pub certs: i64,
    pub subdomains: i64,
    pub total: i64,
    /// AI 评注（人话解释为何薄弱）；裁决靠上面的计数，不靠这句。
    pub note: Option<String>,
}

/// org 树节点（母/子、持股比例、纳入/排除、状态、薄弱度、子节点）。
/// 供 scoping 审批 + 总览折叠树用。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrgTreeNode {
    pub organization_id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub ownership_percent: Option<f64>,
    pub in_scope: bool,
    pub status: OrgRunStatusDto,
    pub weakness: Option<OrgWeaknessScore>,
    pub children: Vec<OrgTreeNode>,
}

/// engagement 级快照（前端总览的顶层数据 = 「范围已锁定」信号载体：
/// `root_count > 0` 即 scoping 已落库建树）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct EngagementSnapshot {
    pub project_path: String,
    /// 调度模式字符串（checklist | funnel）。
    pub mode: String,
    pub root_count: usize,
    pub total_orgs: usize,
    pub covered: usize,
    pub blocked: usize,
    pub failed: usize,
    /// 根森林（每棵 = 一家根公司 + 它的子公司子树）。
    pub tree: Vec<OrgTreeNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_dto_from_internal() {
        use crate::engagement::scheduler::OrgRunStatus;
        let dto: OrgRunStatusDto = OrgRunStatus::Blocked.into();
        assert!(matches!(dto, OrgRunStatusDto::Blocked));
    }

    #[test]
    fn engagement_snapshot_round_trips() {
        let snap = EngagementSnapshot {
            project_path: "/tmp/p".into(),
            mode: "funnel".into(),
            root_count: 1,
            total_orgs: 2,
            covered: 1,
            blocked: 1,
            failed: 0,
            tree: vec![OrgTreeNode {
                organization_id: "00000000-0000-0000-0000-000000000001".into(),
                name: "root".into(),
                parent_id: None,
                ownership_percent: None,
                in_scope: true,
                status: OrgRunStatusDto::Passed,
                weakness: Some(OrgWeaknessScore {
                    organization_id: "00000000-0000-0000-0000-000000000001".into(),
                    cve_hits: 0,
                    login_surfaces: 2,
                    open_ports: 3,
                    certs: 1,
                    subdomains: 5,
                    total: 40,
                    note: None,
                }),
                children: vec![],
            }],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: EngagementSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tree.len(), 1);
        assert_eq!(back.tree[0].name, "root");
        // camelCase 字段名校验。
        assert!(json.contains("projectPath"));
        assert!(json.contains("totalOrgs"));
        assert!(json.contains("loginSurfaces"));
    }
}
