//! Stage-agnostic engagement 调度内核（原 fleet scheduler，纯逻辑搬回）。
//! 设计：`docs/design/2026-06-12-engagement-fleet-orchestration.md` §3.3/§4.1（内核语义）
//! + `docs/design/2026-06-13-engagement-scoping-fanout-redesign.md` §6.4（搬回与新形态）。
//!
//! Phase A 仅搬回备用（query/weakness 复用其中 `FleetMode`/`OrgRunStatus`/trait）；
//! Phase B 的前端会话工人池经命令层注入 executor/oracle/scorer 后驱动。
//!
//! 【头号铁律 §4.1】本文件**永远不**对具体 [`StageKind`] 变体做分支判断
//! （没有 `if entry == StageKind::Pentest {...}`）。entry/to/allowlist 全是
//! 不透明透传参数 —— 加新阶段 = 加 stage JSON + gate 规则，本文件零改动。
//! 守卫见 `scheduler_is_stage_agnostic` 测试。
//!
//! 调度内核与「怎么跑一个 org-run」彻底解耦：执行 / 续跑判定 / 薄弱度评分都经
//! trait 注入，故本文件零 IO、可纯单测。受控并发用 `buffer_unordered`（不 spawn，
//! 借用 trait 引用即可，无 `'static` 约束）。

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use uuid::Uuid;

use golish_agent_kit::harness::StageKind;

/// 一个 org-run 的任务描述。`entry_stage`/`to_stage`/`allowlist` 对调度器不透明。
#[derive(Debug, Clone)]
pub struct OrgRunTask {
    pub org_id: Uuid,
    pub org_name: String,
    pub parent_id: Option<Uuid>,
    pub entry_stage: StageKind,
    pub to_stage: StageKind,
    pub allowlist: HashSet<StageKind>,
    pub objective: String,
}

/// 调度模式（设计 §6）。两种模式共用同一调度内核，只改「排序 + 停止条件 + 视图」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetMode {
    /// 清单：静态序（输入序，母先子后），目标 = 全 org 覆盖。
    Checklist,
    /// 漏斗：薄弱度降序，目标 = 优先打薄弱口（人工收割 / 预算耗尽停）。
    Funnel,
}

impl FleetMode {
    /// 解析 CLI `--mode`。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "checklist" | "list" => Some(Self::Checklist),
            "funnel" => Some(Self::Funnel),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checklist => "checklist",
            Self::Funnel => "funnel",
        }
    }
}

/// 单个 org-run 的终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgRunStatus {
    Passed,
    Blocked,
    Failed,
    /// 续跑：DB 真值已覆盖该 org 该阶段 → 不重跑。
    SkippedAlreadyComplete,
}

impl OrgRunStatus {
    /// 完整性计数口径：已 PASS 或「续跑跳过」都算覆盖到位。
    pub fn is_covered(self) -> bool {
        matches!(self, Self::Passed | Self::SkippedAlreadyComplete)
    }
}

/// 单个 org-run 的执行结果。
#[derive(Debug, Clone)]
pub struct OrgRunOutcome {
    pub org_id: Uuid,
    pub org_name: String,
    pub status: OrgRunStatus,
    pub detail: Option<String>,
}

/// 调度配置。
#[derive(Debug, Clone, Copy)]
pub struct FleetConfig {
    /// 受控并发度。设计目标 2（per-run bridge 隔离后）；CLI 当前默认 1（共享 bridge 安全）。
    pub concurrency: usize,
    pub mode: FleetMode,
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            concurrency: 2,
            mode: FleetMode::Checklist,
        }
    }
}

/// 注入：怎么跑一个 org-run（解耦 bridge/PG，让调度器可纯单测）。
#[async_trait]
pub trait OrgRunExecutor: Send + Sync {
    async fn run_org(&self, task: &OrgRunTask) -> anyhow::Result<String>;
}

/// 注入：某 org 是否已满足完整性（DB 真值）—— 断点续跑判定。
#[async_trait]
pub trait OrgCompletionOracle: Send + Sync {
    async fn is_already_complete(&self, org_id: Uuid, to_stage: StageKind) -> bool;
}

/// 注入：org 薄弱度评分（funnel 排序用）。checklist 模式不调用。
#[async_trait]
pub trait WeaknessScorer: Send + Sync {
    async fn score(&self, org_id: Uuid) -> i64;
}

/// 按模式排序任务（纯函数，IO-free）。checklist 保持输入序（母先子后由调用方保证）；
/// funnel 按预算好的薄弱度分降序（同分按名稳定）。`scores` 缺项视作 0。
pub fn order_tasks(
    tasks: Vec<OrgRunTask>,
    mode: FleetMode,
    scores: &HashMap<Uuid, i64>,
) -> Vec<OrgRunTask> {
    match mode {
        FleetMode::Checklist => tasks,
        FleetMode::Funnel => {
            let mut t = tasks;
            t.sort_by(|a, b| {
                let sa = scores.get(&a.org_id).copied().unwrap_or(0);
                let sb = scores.get(&b.org_id).copied().unwrap_or(0);
                sb.cmp(&sa).then_with(|| a.org_name.cmp(&b.org_name))
            });
            t
        }
    }
}

/// org-run `Err` → 状态。BLOCK 终态（orchestrate 返 "stage blocked"）→ Blocked；
/// 其余 → Failed。stage-agnostic：只看错误文本，不看阶段。
pub fn classify_run_error(err: &anyhow::Error) -> OrgRunStatus {
    if err.to_string().to_ascii_lowercase().contains("block") {
        OrgRunStatus::Blocked
    } else {
        OrgRunStatus::Failed
    }
}

/// engagement 级聚合报告。
#[derive(Debug, Clone, Default)]
pub struct FleetReport {
    pub outcomes: Vec<OrgRunOutcome>,
}

impl FleetReport {
    pub fn covered(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status.is_covered())
            .count()
    }

    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    pub fn blocked(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == OrgRunStatus::Blocked)
            .count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == OrgRunStatus::Failed)
            .count()
    }

    /// engagement 级完整 = 每个 org 都 covered（设计 §2「漏一个 = 不完整」）。
    pub fn is_complete(&self) -> bool {
        !self.outcomes.is_empty() && self.outcomes.iter().all(|o| o.status.is_covered())
    }

    pub fn render(&self) -> String {
        let mut out = String::from("\n────────── engagement fleet report ──────────\n");
        for o in &self.outcomes {
            let tag = match o.status {
                OrgRunStatus::Passed => "PASS",
                OrgRunStatus::SkippedAlreadyComplete => "SKIP(done)",
                OrgRunStatus::Blocked => "BLOCK",
                OrgRunStatus::Failed => "FAIL",
            };
            out.push_str(&format!("  [{tag}] {}", o.org_name));
            if let Some(d) = &o.detail {
                out.push_str(&format!("  — {d}"));
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "engagement: {}/{} orgs covered{}\n",
            self.covered(),
            self.total(),
            if self.is_complete() {
                ""
            } else {
                " — INCOMPLETE (every org must pass; see I8)"
            }
        ));
        out
    }
}

/// 跑整支舰队：funnel 先评分排序 → 受控并发执行（续跑跳过已完整 + 失败隔离）。
/// 借用 executor/oracle/scorer（`buffer_unordered` 不 spawn，无 `'static` 要求）。
pub async fn run_fleet_scheduler(
    config: FleetConfig,
    tasks: Vec<OrgRunTask>,
    executor: &dyn OrgRunExecutor,
    oracle: &dyn OrgCompletionOracle,
    scorer: &dyn WeaknessScorer,
) -> FleetReport {
    // 1) funnel 模式预算薄弱度分（checklist 跳过，零额外查询）。
    let scores: HashMap<Uuid, i64> = if config.mode == FleetMode::Funnel {
        let mut m = HashMap::new();
        for t in &tasks {
            m.insert(t.org_id, scorer.score(t.org_id).await);
        }
        m
    } else {
        HashMap::new()
    };

    // 2) 排序。
    let ordered = order_tasks(tasks, config.mode, &scores);

    // 3) 受控并发执行：续跑跳过 + 失败隔离（一个 Err 不中断其余）。
    let concurrency = config.concurrency.max(1);
    let outcomes: Vec<OrgRunOutcome> = stream::iter(ordered.into_iter().map(|task| async move {
        if oracle.is_already_complete(task.org_id, task.to_stage).await {
            return OrgRunOutcome {
                org_id: task.org_id,
                org_name: task.org_name,
                status: OrgRunStatus::SkippedAlreadyComplete,
                detail: None,
            };
        }
        match executor.run_org(&task).await {
            Ok(_) => OrgRunOutcome {
                org_id: task.org_id,
                org_name: task.org_name,
                status: OrgRunStatus::Passed,
                detail: None,
            },
            Err(e) => OrgRunOutcome {
                org_id: task.org_id,
                org_name: task.org_name,
                status: classify_run_error(&e),
                detail: Some(format!("{e:#}")),
            },
        }
    }))
    .buffer_unordered(concurrency)
    .collect()
    .await;

    FleetReport { outcomes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn task(name: &str, stage: StageKind) -> OrgRunTask {
        OrgRunTask {
            org_id: Uuid::new_v4(),
            org_name: name.into(),
            parent_id: None,
            entry_stage: stage,
            to_stage: stage,
            allowlist: HashSet::from([stage]),
            objective: format!("run {name}"),
        }
    }

    /// mock：按 org_name 决定 Ok / Err / blocked。
    struct MockExec {
        calls: Mutex<Vec<Uuid>>,
    }
    #[async_trait]
    impl OrgRunExecutor for MockExec {
        async fn run_org(&self, t: &OrgRunTask) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(t.org_id);
            match t.org_name.as_str() {
                "boom" => Err(anyhow::anyhow!("network exploded")),
                "blocked" => Err(anyhow::anyhow!("stage blocked")),
                _ => Ok("ok".into()),
            }
        }
    }

    struct AllIncomplete;
    #[async_trait]
    impl OrgCompletionOracle for AllIncomplete {
        async fn is_already_complete(&self, _: Uuid, _: StageKind) -> bool {
            false
        }
    }

    struct SkipNamed(Vec<Uuid>);
    #[async_trait]
    impl OrgCompletionOracle for SkipNamed {
        async fn is_already_complete(&self, id: Uuid, _: StageKind) -> bool {
            self.0.contains(&id)
        }
    }

    struct ZeroScore;
    #[async_trait]
    impl WeaknessScorer for ZeroScore {
        async fn score(&self, _: Uuid) -> i64 {
            0
        }
    }

    #[tokio::test]
    async fn failure_isolation_and_status_classification() {
        let tasks = vec![
            task("ok-1", StageKind::TargetIntel),
            task("boom", StageKind::TargetIntel),
            task("blocked", StageKind::TargetIntel),
            task("ok-2", StageKind::TargetIntel),
        ];
        let exec = MockExec {
            calls: Mutex::new(vec![]),
        };
        let report = run_fleet_scheduler(
            FleetConfig {
                concurrency: 1,
                mode: FleetMode::Checklist,
            },
            tasks,
            &exec,
            &AllIncomplete,
            &ZeroScore,
        )
        .await;
        // 全部跑到（失败不中断兄弟）。
        assert_eq!(exec.calls.lock().unwrap().len(), 4);
        let by = |n: &str| {
            report
                .outcomes
                .iter()
                .find(|o| o.org_name == n)
                .unwrap()
                .status
        };
        assert_eq!(by("ok-1"), OrgRunStatus::Passed);
        assert_eq!(by("boom"), OrgRunStatus::Failed);
        assert_eq!(by("blocked"), OrgRunStatus::Blocked);
        assert_eq!(by("ok-2"), OrgRunStatus::Passed);
        assert!(!report.is_complete());
        assert_eq!(report.covered(), 2);
        assert_eq!(report.blocked(), 1);
        assert_eq!(report.failed(), 1);
    }

    #[tokio::test]
    async fn resume_skips_already_complete_without_running() {
        let done = task("done", StageKind::TargetIntel);
        let pending = task("pending", StageKind::TargetIntel);
        let done_id = done.org_id;
        let exec = MockExec {
            calls: Mutex::new(vec![]),
        };
        let report = run_fleet_scheduler(
            FleetConfig {
                concurrency: 1,
                mode: FleetMode::Checklist,
            },
            vec![done, pending],
            &exec,
            &SkipNamed(vec![done_id]),
            &ZeroScore,
        )
        .await;
        // 已完整的不进 executor。
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
        let by = |n: &str| {
            report
                .outcomes
                .iter()
                .find(|o| o.org_name == n)
                .unwrap()
                .status
        };
        assert_eq!(by("done"), OrgRunStatus::SkippedAlreadyComplete);
        assert_eq!(by("pending"), OrgRunStatus::Passed);
        // covered 计数含「续跑跳过」。
        assert!(report.is_complete());
    }

    #[test]
    fn funnel_orders_by_weakness_desc() {
        let a = task("a", StageKind::TargetIntel);
        let b = task("b", StageKind::TargetIntel);
        let c = task("c", StageKind::TargetIntel);
        let scores = HashMap::from([(a.org_id, 10), (b.org_id, 99), (c.org_id, 50)]);
        let ordered = order_tasks(vec![a, b, c], FleetMode::Funnel, &scores);
        let names: Vec<&str> = ordered.iter().map(|t| t.org_name.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"]); // 99 > 50 > 10
    }

    #[test]
    fn checklist_preserves_input_order() {
        let tasks = vec![
            task("root", StageKind::Scoping),
            task("child-1", StageKind::TargetIntel),
            task("child-2", StageKind::TargetIntel),
        ];
        let ordered = order_tasks(tasks, FleetMode::Checklist, &HashMap::new());
        let names: Vec<&str> = ordered.iter().map(|t| t.org_name.as_str()).collect();
        assert_eq!(names, vec!["root", "child-1", "child-2"]);
    }

    #[test]
    fn mode_parse_round_trip() {
        assert_eq!(FleetMode::parse("checklist"), Some(FleetMode::Checklist));
        assert_eq!(FleetMode::parse("FUNNEL"), Some(FleetMode::Funnel));
        assert_eq!(FleetMode::parse("nope"), None);
    }

    /// 【§4.1 守卫】scheduler 把 stage 当不透明值：把同一组任务的 stage 全换成
    /// 另一组 `StageKind`，排序结果逐一相同 → 证明调度决策不依赖具体阶段。
    #[test]
    fn scheduler_is_stage_agnostic() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let mk = |s: StageKind| {
            vec![
                OrgRunTask {
                    org_id: id1,
                    org_name: "x".into(),
                    parent_id: None,
                    entry_stage: s,
                    to_stage: s,
                    allowlist: HashSet::from([s]),
                    objective: "x".into(),
                },
                OrgRunTask {
                    org_id: id2,
                    org_name: "y".into(),
                    parent_id: None,
                    entry_stage: s,
                    to_stage: s,
                    allowlist: HashSet::from([s]),
                    objective: "y".into(),
                },
            ]
        };
        let scores = HashMap::from([(id1, 5), (id2, 9)]);
        let ids = |v: &[OrgRunTask]| v.iter().map(|t| t.org_id).collect::<Vec<_>>();
        let o_intel = order_tasks(mk(StageKind::TargetIntel), FleetMode::Funnel, &scores);
        let o_enum = order_tasks(mk(StageKind::Enumeration), FleetMode::Funnel, &scores);
        let o_scope = order_tasks(mk(StageKind::Scoping), FleetMode::Funnel, &scores);
        // 不同阶段，排序完全一致（只受 score 影响，不受 stage 影响）。
        assert_eq!(ids(&o_intel), ids(&o_enum));
        assert_eq!(ids(&o_intel), ids(&o_scope));
    }

    #[test]
    fn classify_error_block_vs_fail() {
        assert_eq!(
            classify_run_error(&anyhow::anyhow!("stage blocked")),
            OrgRunStatus::Blocked
        );
        assert_eq!(
            classify_run_error(&anyhow::anyhow!("connection reset")),
            OrgRunStatus::Failed
        );
    }
}
