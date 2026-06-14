//! `engagement_run_fleet` 命令（方案 C / fleet Phase B,T4.2,计划
//! `docs/superpowers/plans/2026-06-14-engagement-fleet-scheduler-convergence.md`）。
//!
//! GUI scoping 锁定范围后，前端改调这**一个**命令（取代 `runPool.ts` 的前端 JS 调度）：
//! 后端在该会话的 bridge 上经 [`run_engagement_fleet`] 跑完整个 engagement（recon→
//! attack，每 org 一个 `run_stage` + 独立 gate），逐 org emit `StageRunOrgProgress`
//! 喂前端单卡（StageRunView）——这就是 CLI 与 GUI「调度层统一」的接线点。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use golish_app_core::GolishError;

use crate::engagement::fleet_run::{run_engagement_fleet, EngagementFleetReport};

/// engagement fleet 跑完的对外摘要。逐 org 进度走 `StageRunOrgProgress` 事件（不在此
/// 返回值里）；这里只回两阶段计数 + engagement 是否完整。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct EngagementRunReportDto {
    pub recon_total: usize,
    pub recon_covered: usize,
    pub attack_total: usize,
    pub attack_covered: usize,
    /// recon 全覆盖且（无 attack 任务或 attack 全覆盖）= engagement 完整。
    pub complete: bool,
}

impl EngagementRunReportDto {
    fn from_report(r: &EngagementFleetReport) -> Self {
        let attack_total = r.attack.total();
        Self {
            recon_total: r.recon.total(),
            recon_covered: r.recon.covered(),
            attack_total,
            attack_covered: r.attack.covered(),
            complete: r.recon.is_complete() && (attack_total == 0 || r.attack.is_complete()),
        }
    }
}

/// 后端驱动整支 engagement fleet（取代前端 `runPool.ts` 的 JS 调度）。在 `session_id`
/// 这个会话的 bridge 上串行跑：对在库全部 in-scope org 先 recon、再对 recon 已覆盖的
/// org 跑 attack，逐 org emit `StageRunOrgProgress`（前端单卡照旧渲染）。
#[tauri::command]
pub async fn engagement_run_fleet(
    session_id: String,
    project_path: String,
    subsidiary_threshold_pct: Option<u8>,
    state: tauri::State<'_, golish_agent_app::AgentState>,
) -> Result<EngagementRunReportDto, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| GolishError::SessionNotFound(session_id.clone()))?;

    // engagement 会话已设 harness profile（worker scope 初始化时）；回退到活跃 profile。
    let profile_id = bridge
        .get_harness_profile()
        .await
        .unwrap_or_else(|| golish_agent_kit::harness::active_profile_id().to_string());

    let report = run_engagement_fleet(
        bridge,
        state.db_pool.clone(),
        &session_id,
        &profile_id,
        &project_path,
        // 串行（共享 bridge 下并行 run_stage 不安全）；K 并发待 per-run bridge 隔离。
        1,
        subsidiary_threshold_pct.unwrap_or(51),
        // emit 逐 org 进度喂前端单卡（StageRunView）。
        true,
    )
    .await
    .map_err(|e| GolishError::Internal(format!("engagement fleet run failed: {e:#}")))?;

    Ok(EngagementRunReportDto::from_report(&report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engagement::scheduler::{FleetReport, OrgRunOutcome, OrgRunStatus};

    fn outcome(name: &str, status: OrgRunStatus) -> OrgRunOutcome {
        OrgRunOutcome {
            org_id: uuid::Uuid::new_v4(),
            org_name: name.into(),
            status,
            detail: None,
        }
    }

    #[test]
    fn report_dto_counts_and_complete() {
        let report = EngagementFleetReport {
            recon: FleetReport {
                outcomes: vec![
                    outcome("a", OrgRunStatus::Passed),
                    outcome("b", OrgRunStatus::Passed),
                ],
            },
            attack: FleetReport {
                outcomes: vec![outcome("a", OrgRunStatus::Passed)],
            },
        };
        let dto = EngagementRunReportDto::from_report(&report);
        assert_eq!(dto.recon_total, 2);
        assert_eq!(dto.recon_covered, 2);
        assert_eq!(dto.attack_total, 1);
        assert_eq!(dto.attack_covered, 1);
        assert!(dto.complete);
    }

    #[test]
    fn report_dto_incomplete_when_a_recon_blocked() {
        let report = EngagementFleetReport {
            recon: FleetReport {
                outcomes: vec![
                    outcome("a", OrgRunStatus::Passed),
                    outcome("b", OrgRunStatus::Blocked),
                ],
            },
            attack: FleetReport::default(),
        };
        let dto = EngagementRunReportDto::from_report(&report);
        assert_eq!(dto.recon_covered, 1);
        assert!(!dto.complete, "a blocked recon org → engagement incomplete");
    }
}
