//! Per-org stage-run executor + scheduler adapters (relocated from the former
//! `engagement::fleet_run`). Keeps `stage_run` self-contained: the CLI
//! subsidiary fan-out (and any future caller) drives
//! [`crate::stage_run::scheduler::run_fleet_scheduler`] through
//! [`OrgFleetExecutor`], which delegates one org's stage slice to
//! [`crate::stage_run::orchestrate`] (= a full `run_stage`, independent gate +
//! org isolation).

use std::sync::Arc;

use anyhow::Result;
use uuid::Uuid;

use golish_agent_kit::harness::{load_embedded_stage_spec, StageKind};
use golish_core::events::{AiEvent, HarnessTraceKind};

use crate::ai::agent_bridge::AgentBridge;
use crate::stage_run::scheduler::{
    FleetProgress, OrgCompletionOracle, OrgRunExecutor, OrgRunOutcome, OrgRunStatus, OrgRunTask,
    WeaknessScorer,
};

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

/// 共享的 per-org 执行器：把「跑一个 org 的 stage 切片」委托给
/// [`crate::stage_run::orchestrate`]（= 一个完整 `run_stage`，独立 gate + org 隔离）。
/// CLI（`emit_progress=false`）与 GUI 后端 fleet（`emit_progress=true`）共用它。
pub(crate) struct OrgFleetExecutor {
    pub bridge: Arc<AgentBridge>,
    pub db_pool: Arc<sqlx::PgPool>,
    pub session_id: String,
    pub profile_id: String,
    pub workspace: std::path::PathBuf,
    pub subsidiary_threshold: u8,
    /// The per-org child-operation adapter is a LegacyV1 compatibility seam.
    /// V2-writing CLI runs are represented by one frozen operation/snapshot and
    /// must never reach `run_org`.
    pub runtime_memory_contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract,
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
                // CLI fleet runs `orchestrate` (a full run_stage) directly rather
                // than dispatching a tracked per-org sub-agent, so there is no
                // sub-agent request id to link a UI drill-in to (and CLI sets
                // `emit_progress=false` anyway). None per the field contract.
                agent_request_id: None,
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
        anyhow::ensure!(
            self.runtime_memory_contract
                == golish_agent_kit::runtime_memory::RuntimeMemoryContract::LegacyV1,
            "OrgFleetExecutor child operations are LegacyV1-only"
        );
        if self.emit_progress {
            self.emit(
                task,
                "running",
                Some(format!("running {}", task.to_stage.as_str())),
            );
        }
        let result = crate::stage_run::orchestrate(
            &self.bridge,
            &self.db_pool,
            &self.session_id,
            &self.profile_id,
            &self.workspace,
            task.entry_stage,
            task.allowlist.clone(),
            &task.objective,
            Some(task.org_id),
            // 每个 org 都是独立任务（root 与子公司平级）→ 单 org run，不在 run 内再开
            // SUBSIDIARY 扇出（避免双重扇出）。
            false,
            self.subsidiary_threshold,
            None,
        )
        .await;
        if self.emit_progress {
            let status = if result.is_ok() { "passed" } else { "blocked" };
            self.emit(task, status, None);
        }
        result
    }
}

/// 照跑所有 org（embedded PG / 同会话跨次持久，重跑会重跑全部）。DB 真值续跑 oracle
/// （`org_stage_has_truth`）接上后替换即可。
pub(crate) struct AlwaysRunOracle;

#[async_trait::async_trait]
impl OrgCompletionOracle for AlwaysRunOracle {
    async fn is_already_complete(&self, _org_id: Uuid, _to: StageKind) -> bool {
        false
    }
}

/// checklist 模式不评分（funnel 才用）；仅满足 [`crate::stage_run::scheduler::run_fleet_scheduler`] 签名。
pub(crate) struct NoopScorer;

#[async_trait::async_trait]
impl WeaknessScorer for NoopScorer {
    async fn score(&self, _org_id: Uuid) -> i64 {
        0
    }
}

/// headless（CLI `--stage-run` / 无单卡）逐 org 进度：在每个 org 进 executor 前后打
/// `[stage-run] ── <label> i/N: 名 → … ──`，恢复手写循环换成调度器后丢的逐子进度可见性。
/// `label` 让调用方区分语义：子公司扇出 = "subsidiary"，全 org = "org"。
pub(crate) struct CliFleetProgress {
    pub label: &'static str,
}

impl FleetProgress for CliFleetProgress {
    fn on_org_start(&self, index: usize, total: usize, task: &OrgRunTask) {
        eprintln!(
            "[stage-run] ── {} {index}/{total}: {} → running {} ──",
            self.label,
            task.org_name,
            task.to_stage.as_str(),
        );
    }

    fn on_org_done(&self, index: usize, total: usize, outcome: &OrgRunOutcome) {
        let tag = match outcome.status {
            OrgRunStatus::Passed => "PASS",
            OrgRunStatus::SkippedAlreadyComplete => "SKIP(done)",
            OrgRunStatus::Blocked => "BLOCK",
            OrgRunStatus::Failed => "FAIL",
        };
        eprintln!(
            "[stage-run] ── {} {index}/{total}: {} → {tag} ──",
            self.label, outcome.org_name,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_case_splits_and_capitalizes() {
        assert_eq!(title_case("target_intel"), "Target Intel");
        assert_eq!(title_case("recon"), "Recon");
        assert_eq!(title_case(""), "");
    }
}
