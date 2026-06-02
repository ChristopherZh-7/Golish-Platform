//! Engine v2 · P2 增量 1 — operation 流转的 metalcraft 图模型（additive）。
//!
//! 把 profile 投影后的 [`AllowedDag`] 编译成一张 metalcraft
//! [`graph_engine::CompiledGraph`]，用 metalcraft [`graph_engine::Executor`] +
//! **条件边** 驱动阶段流转，从而：
//!   - 激活拓扑里**早已声明、却被旧驱动忽略的 bail-to-reporting 短路边**
//!     （旧 `stage_transition::advance_target` 对 `Branch` 永远取 `candidates[0]`）；
//!   - 通过 [`graph_engine::Checkpointer`] 拿到 interrupt/resume（gate BLOCK = 在该
//!     stage 暂停返工，可恢复）。
//!
//! **本模块是 additive 的**：只被自身 + `harness/mod.rs` 引用，**不**接入 live
//! `drive_stage_transition`。把 live 流转切到这里（flag 闸）是增量 2，DB-backed
//! checkpointer 是增量 3 —— 见
//! `docs/superpowers/plans/2026-06-02-engine-v2-p2-metalcraft-graph-executor.md`。
//!
//! 路由约定（条件边）：分支 stage 的后继按 `operation_graph.json` 边声明顺序排列，
//! 线性主路边在前、bail-to-reporting 短路边在后。故 **有进展 → `candidates[0]`（主路）**，
//! **无进展 → `candidates.last()`（bail 到 reporting）**。

use std::collections::HashMap;
use std::sync::Arc;

use super::graph_engine::{
    CompiledGraph, Executor, GraphError, MemoryCheckpointer, NodeOutcome, Reducer, END,
};
use super::operation_graph::AllowedDag;
use super::stage_transition::TransitionDecision;
use super::types::StageKind;

/// Feature flag: route the **live** stage transition through the metalcraft
/// graph-flow conditional routing (bail-to-reporting when a stage makes no
/// progress) instead of the legacy "always take the first branch candidate".
///
/// **Default OFF** — opt in with `GOLISH_HARNESS_GRAPH_FLOW=1` (or `true`/`on`/
/// `yes`, case-insensitive). Cached once at first read (LazyLock), same pattern
/// as [`super::stage_mode_enabled`]. OFF reproduces legacy behaviour exactly, so
/// wiring this into `drive_stage_transition` is a zero-risk additive change.
pub fn graph_flow_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_graph_flow_flag(std::env::var("GOLISH_HARNESS_GRAPH_FLOW").ok().as_deref())
    });
    *ENABLED
}

/// Pure parser (env-independent → fully unit-testable). Default OFF: only an
/// explicit truthy value turns it on.
fn parse_graph_flow_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// Branch routing rule, single-sourced for both the metalcraft conditional edge
/// (see [`build_operation_flow_graph`]) and the live driver
/// ([`chosen_next_stage`]).
///
/// `candidates` are the topological successors in `operation_graph.json` edge
/// declaration order (linear main-path edge first, bail-to-reporting shortcut
/// last). **Made progress → `candidates[0]` (main path); no progress →
/// `candidates.last()` (bail).** Empty → `None`.
pub fn branch_target(candidates: &[StageKind], made_progress: bool) -> Option<StageKind> {
    if made_progress {
        candidates.first().copied()
    } else {
        candidates.last().copied()
    }
}

/// Decide which stage the cursor advances to after a gate outcome.
///
/// - `graph_flow == false` → identical to the legacy
///   [`TransitionDecision::advance_target`] (branch takes the first candidate).
/// - `graph_flow == true` → a [`TransitionDecision::Branch`] routes by
///   `made_progress` via [`branch_target`]; non-branch decisions are unchanged.
pub fn chosen_next_stage(
    decision: &TransitionDecision,
    made_progress: bool,
    graph_flow: bool,
) -> Option<StageKind> {
    match decision {
        TransitionDecision::Branch(candidates) if graph_flow => {
            branch_target(candidates, made_progress)
        }
        other => other.advance_target(),
    }
}

/// 一个 stage 在「流转路由」视角下的产出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageFlowOutcome {
    /// 该 stage 的确定性 gate 是否通过。`false` → 在此 stage 暂停返工。
    pub gate_allowed: bool,
    /// 该 stage 是否取得实质进展（如发现攻击面 / findings）。分支 stage 据此决定
    /// 走主路还是 bail 到 reporting。
    pub made_progress: bool,
}

impl StageFlowOutcome {
    /// gate 过且有进展（默认乐观值）。
    pub fn pass_with_progress() -> Self {
        Self {
            gate_allowed: true,
            made_progress: true,
        }
    }
    /// gate 过但没有进展（触发 bail-to-reporting）。
    pub fn pass_no_progress() -> Self {
        Self {
            gate_allowed: true,
            made_progress: false,
        }
    }
    /// gate 没过（在该 stage interrupt/返工）。
    pub fn blocked() -> Self {
        Self {
            gate_allowed: false,
            made_progress: false,
        }
    }
}

/// metalcraft [`Reducer`] 状态：operation 流转的累积视图。
#[derive(Debug, Clone, Default)]
pub struct OperationFlowState {
    /// 调用方预置的「每个 stage 会产出什么」。增量 2+ 改为各 stage 真跑时实时填入。
    pub seeded: HashMap<StageKind, StageFlowOutcome>,
    /// 已执行的 stage，按顺序。
    pub visited: Vec<StageKind>,
    /// 已执行 stage 记录的产出（节点运行时从 `seeded` 拷入）—— 条件边读它选下一步。
    pub applied: HashMap<StageKind, StageFlowOutcome>,
}

impl OperationFlowState {
    /// 以预置产出构造初始状态。
    pub fn with_seeded(seeded: HashMap<StageKind, StageFlowOutcome>) -> Self {
        Self {
            seeded,
            ..Default::default()
        }
    }

    /// 取某 stage 的预置产出，缺省视为「过且有进展」（不阻流转）。
    fn seeded_outcome(&self, stage: StageKind) -> StageFlowOutcome {
        self.seeded
            .get(&stage)
            .copied()
            .unwrap_or_else(StageFlowOutcome::pass_with_progress)
    }
}

/// 状态变更：节点执行/返工注入。
pub enum FlowUpdate {
    /// 记录 `stage` 已执行（把其 `seeded` 产出拷进 `applied`）。
    Executed(StageKind),
    /// 返工/resume 时注入：覆盖某 stage 的预置产出（如「现在 gate 过了」）。
    SetOutcome(StageKind, StageFlowOutcome),
}

impl Reducer for OperationFlowState {
    type Update = FlowUpdate;
    fn apply(&mut self, update: FlowUpdate) {
        match update {
            FlowUpdate::Executed(stage) => {
                let outcome = self.seeded_outcome(stage);
                self.visited.push(stage);
                self.applied.insert(stage, outcome);
            }
            FlowUpdate::SetOutcome(stage, outcome) => {
                self.seeded.insert(stage, outcome);
            }
        }
    }
}

/// 把 profile 投影后的 [`AllowedDag`] 编译成 metalcraft 图。
///
/// 每个 stage = 一个节点；单后继 → 静态边；多后继 → **条件边**（按 `made_progress`
/// 选主路 / bail）；终点 → `END`。gate 未过的节点 → `Interrupt`（暂停返工，可 resume）。
pub fn build_operation_flow_graph(
    dag: &AllowedDag,
) -> Result<CompiledGraph<OperationFlowState>, GraphError> {
    let mut g = super::graph_engine::Graph::<OperationFlowState>::new();

    for &stage in &dag.nodes {
        let s = stage;
        g = g.add_node(
            stage.as_str(),
            move |state: OperationFlowState| async move {
                let outcome = state.seeded_outcome(s);
                if outcome.gate_allowed {
                    Ok(NodeOutcome::Update(FlowUpdate::Executed(s)))
                } else {
                    Ok(NodeOutcome::interrupt(format!(
                        "gate blocked at stage '{}' — hold for rework",
                        s.as_str()
                    )))
                }
            },
        );
    }

    for &stage in &dag.nodes {
        let nexts = dag.next_stages(stage);
        match nexts.as_slice() {
            [] => {
                g = g.add_edge(stage.as_str(), END);
            }
            [only] => {
                g = g.add_edge(stage.as_str(), only.as_str());
            }
            _ => {
                let candidates = nexts.clone();
                let s = stage;
                g = g.add_conditional(stage.as_str(), move |state: &OperationFlowState| {
                    let progressed = state
                        .applied
                        .get(&s)
                        .map(|o| o.made_progress)
                        .unwrap_or(true);
                    // Same rule the live driver uses (single-sourced).
                    branch_target(&candidates, progressed)
                        .unwrap_or(s)
                        .as_str()
                        .to_string()
                });
            }
        }
    }

    if let Some(&entry) = dag.entry_points().first() {
        g = g.set_entry(entry.as_str());
    }
    g.compile()
}

/// 构造一个挂了 [`MemoryCheckpointer`] 的 [`Executor`]（支持 run + resume）。
pub fn operation_flow_executor(
    dag: &AllowedDag,
) -> Result<Executor<OperationFlowState>, GraphError> {
    Ok(Executor::new(build_operation_flow_graph(dag)?)
        .with_checkpointer(Arc::new(MemoryCheckpointer::new())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::graph_engine::RunOutcome;
    use crate::harness::operation_graph::load_operation_graph_from_json;
    use crate::harness::profile::load_profile_from_json;

    const BASE_GRAPH_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn assessment_dag() -> AllowedDag {
        let g = load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph");
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        g.project(&p.allowed_stage_set())
    }

    fn seed(pairs: &[(StageKind, StageFlowOutcome)]) -> HashMap<StageKind, StageFlowOutcome> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn assessment_dag_compiles_to_metalcraft_graph() {
        let graph = build_operation_flow_graph(&assessment_dag()).expect("compile");
        let m = graph.to_mermaid();
        assert!(m.contains("flowchart TD"), "mermaid: {m}");
        assert!(m.contains("scoping"), "mermaid: {m}");
    }

    #[tokio::test]
    async fn full_progress_walks_complete_recon_path() {
        // Every stage passes with progress → linear main path, no bail.
        let dag = assessment_dag();
        let state = OperationFlowState::with_seeded(seed(&[
            (StageKind::Scoping, StageFlowOutcome::pass_with_progress()),
            (
                StageKind::TargetIntel,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::ExternalAttackSurface,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::Enumeration,
                StageFlowOutcome::pass_with_progress(),
            ),
            (StageKind::Reporting, StageFlowOutcome::pass_with_progress()),
        ]));
        let exec = operation_flow_executor(&dag).expect("executor");
        match exec.run(state, "op").await.expect("run") {
            RunOutcome::Completed(s) => assert_eq!(
                s.visited,
                vec![
                    StageKind::Scoping,
                    StageKind::TargetIntel,
                    StageKind::ExternalAttackSurface,
                    StageKind::Enumeration,
                    StageKind::Reporting,
                ]
            ),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_surface_bails_external_attack_surface_to_reporting() {
        // external_attack_surface passes the gate but finds NO surface → the
        // conditional edge takes the bail-to-reporting shortcut (skips
        // enumeration), the edge the old `advance_target` never used.
        let dag = assessment_dag();
        let state = OperationFlowState::with_seeded(seed(&[
            (
                StageKind::ExternalAttackSurface,
                StageFlowOutcome::pass_no_progress(),
            ),
            // others default to pass_with_progress
        ]));
        let exec = operation_flow_executor(&dag).expect("executor");
        match exec.run(state, "op").await.expect("run") {
            RunOutcome::Completed(s) => {
                assert_eq!(
                    s.visited,
                    vec![
                        StageKind::Scoping,
                        StageKind::TargetIntel,
                        StageKind::ExternalAttackSurface,
                        StageKind::Reporting,
                    ],
                    "no-surface run must bail eas→reporting, skipping enumeration"
                );
                assert!(!s.visited.contains(&StageKind::Enumeration));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn blocked_gate_interrupts_then_resumes_after_rework() {
        // external_attack_surface gate BLOCKs → executor interrupts at eas
        // (rework hold). Injecting a now-passing outcome on resume lets the
        // flow continue to completion — checkpoint + resume end-to-end.
        let dag = assessment_dag();
        let state = OperationFlowState::with_seeded(seed(&[(
            StageKind::ExternalAttackSurface,
            StageFlowOutcome::blocked(),
        )]));
        let exec = operation_flow_executor(&dag).expect("executor");

        match exec.run(state, "op").await.expect("run") {
            RunOutcome::Interrupted {
                resume_from,
                state: s,
                ..
            } => {
                assert_eq!(resume_from, StageKind::ExternalAttackSurface.as_str());
                // eas did not complete → not yet visited; recon prefix did.
                assert_eq!(s.visited, vec![StageKind::Scoping, StageKind::TargetIntel]);
            }
            other => panic!("expected Interrupted at eas, got {other:?}"),
        }

        // Rework fixed it: inject a passing outcome and resume.
        let resumed = exec
            .resume(
                "op",
                Some(FlowUpdate::SetOutcome(
                    StageKind::ExternalAttackSurface,
                    StageFlowOutcome::pass_with_progress(),
                )),
            )
            .await
            .expect("resume");
        match resumed {
            RunOutcome::Completed(s) => {
                assert!(s.visited.contains(&StageKind::ExternalAttackSurface));
                assert_eq!(*s.visited.last().unwrap(), StageKind::Reporting);
            }
            other => panic!("expected Completed after resume, got {other:?}"),
        }
    }

    #[test]
    fn missing_entry_point_fails_compile() {
        // An empty DAG has no entry → compile must error (never panic).
        let empty = load_operation_graph_from_json(BASE_GRAPH_JSON)
            .expect("graph")
            .project(&std::collections::HashSet::new());
        assert!(build_operation_flow_graph(&empty).is_err());
    }

    #[test]
    fn branch_target_routes_by_progress() {
        let candidates = vec![StageKind::Enumeration, StageKind::Reporting];
        assert_eq!(
            branch_target(&candidates, true),
            Some(StageKind::Enumeration),
            "progress → main path (first candidate)"
        );
        assert_eq!(
            branch_target(&candidates, false),
            Some(StageKind::Reporting),
            "no progress → bail (last candidate)"
        );
        assert_eq!(branch_target(&[], true), None);
    }

    #[test]
    fn chosen_next_stage_only_conditional_when_graph_flow_on() {
        let branch = TransitionDecision::Branch(vec![StageKind::Enumeration, StageKind::Reporting]);
        // graph_flow OFF → legacy first-candidate regardless of progress.
        assert_eq!(
            chosen_next_stage(&branch, false, false),
            Some(StageKind::Enumeration)
        );
        // graph_flow ON + no progress → bail to reporting (the win).
        assert_eq!(
            chosen_next_stage(&branch, false, true),
            Some(StageKind::Reporting)
        );
        // graph_flow ON + progress → main path.
        assert_eq!(
            chosen_next_stage(&branch, true, true),
            Some(StageKind::Enumeration)
        );
        // Non-branch decisions are unaffected by the flag.
        assert_eq!(
            chosen_next_stage(
                &TransitionDecision::Advance(StageKind::TargetIntel),
                false,
                true
            ),
            Some(StageKind::TargetIntel)
        );
        assert_eq!(
            chosen_next_stage(&TransitionDecision::Hold, true, true),
            None
        );
        assert_eq!(
            chosen_next_stage(&TransitionDecision::Complete, true, true),
            None
        );
    }

    #[test]
    fn parse_graph_flow_flag_defaults_off() {
        assert!(!parse_graph_flow_flag(None));
        for off in ["0", "false", "off", "no", "", "garbage"] {
            assert!(!parse_graph_flow_flag(Some(off)), "'{off}' must stay OFF");
        }
        for on in ["1", "true", "on", "yes", "TRUE", " On "] {
            assert!(parse_graph_flow_flag(Some(on)), "'{on}' must enable");
        }
    }

    #[tokio::test]
    async fn graph_run_uses_shared_branch_rule_for_bail() {
        // The metalcraft graph's conditional edge and the live driver now share
        // `branch_target`; confirm the graph honours it (no-progress eas bails).
        let dag = assessment_dag();
        let state = OperationFlowState::with_seeded(seed(&[(
            StageKind::ExternalAttackSurface,
            StageFlowOutcome::pass_no_progress(),
        )]));
        let exec = operation_flow_executor(&dag).expect("executor");
        match exec.run(state, "op").await.expect("run") {
            RunOutcome::Completed(s) => {
                assert!(!s.visited.contains(&StageKind::Enumeration));
                assert!(s.visited.contains(&StageKind::Reporting));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }
}
