//! Backend engagement fleet driver (方案 C / fleet Phase B,计划
//! `docs/superpowers/plans/2026-06-14-engagement-fleet-scheduler-convergence.md`).
//!
//! 把前端那套两阶段 JS 调度器（`frontend/lib/engagement/{pool,runPool}.ts`）的
//! **调度逻辑**搬到后端，让 CLI（headless）和 GUI 跑**同一条** per-org 调度：经
//! [`crate::engagement::scheduler::run_fleet_scheduler`] 逐 org 一个完整 `run_stage`
//! （独立 gate + org 隔离）。
//!
//! 两阶段（对齐前端 `STAGE_SLICES`）：
//! - **recon**：`target_intel..=enumeration`，对**每个** in-scope org 各跑一次。
//! - **attack**：`vuln_triage..=reporting`，只对 recon 已覆盖（passed/skip）的 org。
//!
//! 与前端旧版的差别（即收敛点）：前端 recon 以「家族」为单位、子公司在一次 run 内
//! 靠 in-stage 工具扇出；这里统一成**每 org 一个 OrgRunTask**（root 与子公司平级各跑
//! 各的 run_stage），这才是「调度层统一、每 org 真 gate」。executor 在跑每个 org 时
//! emit [`HarnessTraceKind::StageRunOrgProgress`]，前端单卡（StageRunView）照旧渲染。

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use uuid::Uuid;

use golish_agent_kit::harness::{load_embedded_stage_spec, resolve_slice, StageKind};
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_db::models::Organization;

use crate::ai::agent_bridge::AgentBridge;
use crate::engagement::scheduler::{
    run_fleet_scheduler, FleetConfig, FleetMode, FleetReport, OrgCompletionOracle, OrgRunExecutor,
    OrgRunTask, WeaknessScorer,
};

/// recon 阶段切片端点（对齐前端 `STAGE_SLICES.recon_family`）。
const RECON_FROM: StageKind = StageKind::TargetIntel;
const RECON_TO: StageKind = StageKind::Enumeration;
/// attack 阶段切片端点（对齐前端 `STAGE_SLICES.attack_org`）。
const ATTACK_FROM: StageKind = StageKind::VulnTriage;
const ATTACK_TO: StageKind = StageKind::Reporting;

/// `snake_case` stage id → 展示用 Title Case（`target_intel` → `Target Intel`）。
fn title_case(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// recon 阶段每 org 的目标（pin org_id + 限定本 org，对齐前端 `buildWorkerPrompt`）。
fn recon_objective(org: &Organization) -> String {
    format!(
        "对组织「{}」(organization_id={}) 执行信息收集：从 {} 阶段做到 {} 阶段（含），\
         过每个阶段的 gate 后停止。纪律：只收集这个组织自身的资产，不要碰其它组织；\
         严格按证据账本工作，查过为空就如实标 checked_empty，绝不编造。",
        org.name,
        org.id,
        RECON_FROM.as_str(),
        RECON_TO.as_str(),
    )
}

/// attack 阶段每 org 的目标（信息收集数据已在库，按 org 隔离直接据此研判到报告）。
fn attack_objective(org: &Organization) -> String {
    format!(
        "对组织「{}」(organization_id={}) 执行漏洞研判到报告：从 {} 阶段做到 {} 阶段（含），\
         过最后的报告 gate 后停止。信息收集阶段的资产/端点/指纹数据已在数据库中（按该组织\
         隔离），直接据此工作。纪律：只打这个组织名下 in-scope 的资产；每个发现都要有可追溯\
         证据；查过为空就如实标 checked_empty，绝不编造。",
        org.name,
        org.id,
        ATTACK_FROM.as_str(),
        ATTACK_TO.as_str(),
    )
}

/// 构造一阶段的 per-org 任务集（纯函数，便于单测）。`filter` 决定哪些 org 入选
/// （recon = 全部；attack = 仅 recon 已覆盖）。slice 由 profile 解析。
fn build_phase_tasks(
    orgs: &[Organization],
    profile_id: &str,
    from: StageKind,
    to: StageKind,
    objective: fn(&Organization) -> String,
    include: impl Fn(&Organization) -> bool,
) -> Result<Vec<OrgRunTask>> {
    let (entry, allowlist) = resolve_slice(profile_id, Some(from), to).map_err(|e| anyhow!(e))?;
    Ok(orgs
        .iter()
        .filter(|o| include(o))
        .map(|o| OrgRunTask {
            org_id: o.id,
            org_name: o.name.clone(),
            parent_id: o.parent_id,
            entry_stage: entry,
            to_stage: to,
            allowlist: allowlist.clone(),
            objective: objective(o),
        })
        .collect())
}

/// recon 阶段任务：每个 in-scope org 一个（`target_intel..=enumeration`）。
fn recon_fleet_tasks(orgs: &[Organization], profile_id: &str) -> Result<Vec<OrgRunTask>> {
    build_phase_tasks(orgs, profile_id, RECON_FROM, RECON_TO, recon_objective, |_| {
        true
    })
}

/// attack 阶段任务：只对 recon 已覆盖的 org（`vuln_triage..=reporting`）。
fn attack_fleet_tasks(
    orgs: &[Organization],
    covered: &HashSet<Uuid>,
    profile_id: &str,
) -> Result<Vec<OrgRunTask>> {
    build_phase_tasks(
        orgs,
        profile_id,
        ATTACK_FROM,
        ATTACK_TO,
        attack_objective,
        |o| covered.contains(&o.id),
    )
}

/// 共享的 per-org 执行器：把「跑一个 org 的 stage 切片」委托给
/// [`crate::stage_run::orchestrate`]（= 一个完整 `run_stage`，独立 gate + org 隔离）。
/// CLI（`emit_progress=false`）与 GUI 后端 fleet（`emit_progress=true`）共用它 ——
/// 这是方案 C 「per-org 原语统一」的落点。
pub(crate) struct OrgFleetExecutor {
    pub bridge: Arc<AgentBridge>,
    pub db_pool: Arc<sqlx::PgPool>,
    pub session_id: String,
    pub profile_id: String,
    pub subsidiary_threshold: u8,
    /// GUI 单卡（StageRunView）靠 `StageRunOrgProgress` 渲染；CLI 无卡 → false。
    pub emit_progress: bool,
}

impl OrgFleetExecutor {
    /// emit 一条 org 进度事件（喂前端单卡）。stage/role 标签从 to_stage 的 spec 取。
    fn emit(&self, task: &OrgRunTask, status: &str, activity: Option<String>) {
        let (role_label, coverage_axis) = load_embedded_stage_spec(task.to_stage)
            .map(|s| (s.specialist.unwrap_or_default(), s.coverage_axis))
            .unwrap_or_default();
        self.bridge.emit_event(AiEvent::HarnessTrace {
            operation_id: self.session_id.clone(),
            stage: task.to_stage.as_str().to_string(),
            agent_path: "main".to_string(),
            trace: HarnessTraceKind::StageRunOrgProgress {
                org_id: task.org_id.to_string(),
                org_name: task.org_name.clone(),
                ownership_percent: None,
                status: status.to_string(),
                coverage: Vec::new(),
                evidence_count: 0,
                activity,
                stage_label: title_case(task.to_stage.as_str()),
                role_label: title_case(&role_label),
                coverage_axis,
            },
        });
    }
}

#[async_trait::async_trait]
impl OrgRunExecutor for OrgFleetExecutor {
    async fn run_org(&self, task: &OrgRunTask) -> Result<String> {
        if self.emit_progress {
            self.emit(task, "running", Some(format!("running {}", task.to_stage.as_str())));
        }
        let result = crate::stage_run::orchestrate(
            &self.bridge,
            &self.db_pool,
            &self.session_id,
            &self.profile_id,
            task.entry_stage,
            task.allowlist.clone(),
            &task.objective,
            Some(task.org_id),
            // fleet 里每个 org 都是独立任务（root 与子公司平级）→ 单 org run，不再
            // 在 run 内开 SUBSIDIARY 扇出（避免双重扇出）。
            false,
            self.subsidiary_threshold,
        )
        .await;
        if self.emit_progress {
            let status = if result.is_ok() { "passed" } else { "blocked" };
            self.emit(task, status, None);
        }
        result
    }
}

/// T1/T4.1 行为保持：照跑所有 org（embedded PG / 同会话跨次持久，重跑会重跑全部）。
/// DB 真值续跑 oracle（`org_stage_has_truth`）是 T3 的硬化项，接上后这里替换即可。
pub(crate) struct AlwaysRunOracle;

#[async_trait::async_trait]
impl OrgCompletionOracle for AlwaysRunOracle {
    async fn is_already_complete(&self, _org_id: Uuid, _to: StageKind) -> bool {
        false
    }
}

/// checklist 模式不评分（funnel 才用）；仅满足 [`run_fleet_scheduler`] 签名。
pub(crate) struct NoopScorer;

#[async_trait::async_trait]
impl WeaknessScorer for NoopScorer {
    async fn score(&self, _org_id: Uuid) -> i64 {
        0
    }
}

/// 一次 engagement fleet 跑完的两阶段聚合（recon + attack）。
#[derive(Debug, Clone, Default)]
pub struct EngagementFleetReport {
    pub recon: FleetReport,
    pub attack: FleetReport,
}

/// 后端 engagement fleet 驱动（方案 C 的核心，对应前端 `runPool.ts`）。假定 scoping
/// 已锁定范围、org 树已落库；对在库的全部 in-scope org 跑 recon（每 org 各跑），再对
/// recon 已覆盖的 org 跑 attack。两阶段都走 [`run_fleet_scheduler`]（checklist 母先子
/// 后、K 受控并发、失败隔离）。`emit_progress=true` 时逐 org emit 单卡进度事件。
#[allow(clippy::too_many_arguments)]
pub async fn run_engagement_fleet(
    bridge: Arc<AgentBridge>,
    db_pool: Arc<sqlx::PgPool>,
    session_id: &str,
    profile_id: &str,
    project_path: &str,
    concurrency: usize,
    subsidiary_threshold: u8,
    emit_progress: bool,
) -> Result<EngagementFleetReport> {
    let orgs = golish_db::repo::organizations::list(&db_pool, project_path).await?;
    if orgs.is_empty() {
        return Ok(EngagementFleetReport::default());
    }

    let executor = OrgFleetExecutor {
        bridge,
        db_pool: db_pool.clone(),
        session_id: session_id.to_string(),
        profile_id: profile_id.to_string(),
        subsidiary_threshold,
        emit_progress,
    };
    // 共享 bridge 下并行 run_stage 不安全（覆盖阶段态/历史/取消）→ 实际并发钳到 1，
    // 直到 per-run bridge 隔离落地（设计目标 2）。`concurrency` 入参先保留待启用。
    let _ = concurrency;
    let config = FleetConfig {
        concurrency: 1,
        mode: FleetMode::Checklist,
    };

    // Phase 1 · recon（每个 org 各跑一次 target_intel..=enumeration）。
    let recon_tasks = recon_fleet_tasks(&orgs, profile_id)?;
    let recon = run_fleet_scheduler(config, recon_tasks, &executor, &AlwaysRunOracle, &NoopScorer)
        .await;

    // Phase 2 · attack（只对 recon 已覆盖的 org 跑 vuln_triage..=reporting）。
    let covered: HashSet<Uuid> = recon
        .outcomes
        .iter()
        .filter(|o| o.status.is_covered())
        .map(|o| o.org_id)
        .collect();
    let attack_tasks = attack_fleet_tasks(&orgs, &covered, profile_id)?;
    let attack = if attack_tasks.is_empty() {
        FleetReport::default()
    } else {
        run_fleet_scheduler(config, attack_tasks, &executor, &AlwaysRunOracle, &NoopScorer).await
    };

    Ok(EngagementFleetReport { recon, attack })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用 serde 兜 `#[serde(default)]` profile 字段，构造测试 org（同 stage_fanout 测试）。
    fn test_org(name: &str, parent_id: Option<Uuid>) -> Organization {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(),
            "project_path": "/tmp/p",
            "name": name,
            "parent_id": parent_id,
            "description": "",
            "owner": "",
            "sort_order": 0,
            "created_at": "2026-06-14T00:00:00Z",
            "updated_at": "2026-06-14T00:00:00Z",
        }))
        .expect("test org deserializes")
    }

    #[test]
    fn title_case_splits_and_capitalizes() {
        assert_eq!(title_case("target_intel"), "Target Intel");
        assert_eq!(title_case("recon"), "Recon");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn recon_tasks_one_per_org_with_recon_slice() {
        let root = test_org("默安科技", None);
        let child = test_org("默安子公司", Some(root.id));
        let orgs = vec![root.clone(), child.clone()];
        let tasks = recon_fleet_tasks(&orgs, "red_team").expect("red_team resolves recon slice");
        assert_eq!(tasks.len(), 2, "one recon task per in-scope org");
        // 切片端点固定到 enumeration，且 allowlist 不含 attack 阶段。
        for t in &tasks {
            assert_eq!(t.to_stage, RECON_TO);
            assert!(t.allowlist.contains(&StageKind::Enumeration));
            assert!(
                !t.allowlist.contains(&StageKind::Reporting),
                "recon slice must not reach attack stages"
            );
        }
        // objective pin 了 org_id + 阶段名。
        let root_task = tasks.iter().find(|t| t.org_id == root.id).unwrap();
        assert!(root_task.objective.contains(&root.id.to_string()));
        assert!(root_task.objective.contains("target_intel"));
        assert!(root_task.objective.contains("enumeration"));
    }

    #[test]
    fn attack_tasks_only_for_covered_orgs() {
        let a = test_org("A", None);
        let b = test_org("B", None);
        let orgs = vec![a.clone(), b.clone()];
        let covered: HashSet<Uuid> = [a.id].into_iter().collect();
        let tasks = attack_fleet_tasks(&orgs, &covered, "red_team").expect("attack slice resolves");
        assert_eq!(tasks.len(), 1, "only recon-covered orgs get an attack task");
        assert_eq!(tasks[0].org_id, a.id);
        assert_eq!(tasks[0].to_stage, ATTACK_TO);
        assert!(tasks[0].allowlist.contains(&StageKind::Reporting));
        assert!(tasks[0].objective.contains("vuln_triage"));
    }

    #[test]
    fn attack_tasks_empty_when_nothing_covered() {
        let a = test_org("A", None);
        let orgs = vec![a];
        let empty: HashSet<Uuid> = HashSet::new();
        let tasks = attack_fleet_tasks(&orgs, &empty, "red_team").expect("resolves");
        assert!(tasks.is_empty(), "no recon coverage → no attack tasks");
    }

    #[test]
    fn recon_tasks_preserve_input_order_mother_first() {
        // organizations::list 已按 parent NULLS FIRST 排序；checklist 保持该序。
        let root = test_org("root", None);
        let child = test_org("child", Some(root.id));
        let orgs = vec![root.clone(), child.clone()];
        let tasks = recon_fleet_tasks(&orgs, "red_team").unwrap();
        assert_eq!(tasks[0].org_id, root.id, "mother first");
        assert_eq!(tasks[1].org_id, child.id, "child after");
    }
}
