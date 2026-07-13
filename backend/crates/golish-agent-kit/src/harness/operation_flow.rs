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
/// A [`TransitionDecision::Branch`] routes by `made_progress` via
/// [`branch_target`] (no findings → bail to reporting); non-branch decisions
/// resolve via [`TransitionDecision::advance_target`].
pub fn chosen_next_stage(decision: &TransitionDecision, made_progress: bool) -> Option<StageKind> {
    match decision {
        TransitionDecision::Branch(candidates) => branch_target(candidates, made_progress),
        other => other.advance_target(),
    }
}

/// 一个 stage 在「流转路由」视角下的产出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StageFlowOutcome {
    /// 该 stage 的确定性 gate 是否通过。`false` → 在此 stage 暂停返工。
    pub gate_allowed: bool,
    /// 该 stage 是否取得实质进展（如发现攻击面 / findings）。分支 stage 据此决定
    /// 走主路还是 bail 到 reporting。
    pub made_progress: bool,
    /// 波次循环信号（设计 2026-07-02-attack-stage §3.5）：`verification` 阶段收尾时由
    /// 服务者（`consume_gate_outcome`，握有本波交付物的 candidates + 跨波去重集 + 燃料/
    /// 深度上限）经 [`super::chain_wave::decide_chain_wave`] 判定——`true` = 本波产出了
    /// 新的、未测过且在预算内的攻击假设，应把游标覆写回 `attack_candidate` 开新一波。
    /// 其它阶段恒 `false`。图节点只据此信号路由，决策逻辑不进 DB-free 的图层。加性字段
    /// （serde 缺省 false），旧 checkpoint / 现有构造点零回归。
    #[serde(default)]
    pub reopen_wave: bool,
    /// `true` means the V2 consolidation transaction already applied the
    /// durable fuel/depth policy and advanced (or closed) the global Wave
    /// cursor. Graph-local counters must not override that committed decision.
    #[serde(default)]
    pub durable_wave_cursor: bool,
}

impl StageFlowOutcome {
    /// gate 过且有进展（默认乐观值）。
    pub fn pass_with_progress() -> Self {
        Self {
            gate_allowed: true,
            made_progress: true,
            reopen_wave: false,
            durable_wave_cursor: false,
        }
    }
    /// gate 过但没有进展（触发 bail-to-reporting）。
    pub fn pass_no_progress() -> Self {
        Self {
            gate_allowed: true,
            made_progress: false,
            reopen_wave: false,
            durable_wave_cursor: false,
        }
    }
    /// gate 没过（在该 stage interrupt/返工）。
    pub fn blocked() -> Self {
        Self {
            gate_allowed: false,
            made_progress: false,
            reopen_wave: false,
            durable_wave_cursor: false,
        }
    }

    /// gate 过、有进展、且 `verification` 判定应开新一波（游标覆写回 attack_candidate）。
    pub fn pass_reopen_wave() -> Self {
        Self {
            gate_allowed: true,
            made_progress: true,
            reopen_wave: true,
            durable_wave_cursor: false,
        }
    }
}

/// Convert exact persisted Verification truth into graph flow. Progress means
/// at least one proof-backed, Finding-linked, exact-lineage `verified` Attempt;
/// merely having approved/refuted/blocked Candidates is not enough to enter
/// AccessValidation. V2 never opens a process-local chain wave from deliverable
/// candidates; the durable FactDelta consolidation transaction exclusively owns
/// that cursor change.
pub fn exact_verification_flow_outcome(
    truth: &super::attack_execution::VerificationTruthSet,
) -> StageFlowOutcome {
    let allowed = super::attack_execution::validate_verification_truth_set(truth).is_ok();
    StageFlowOutcome {
        gate_allowed: allowed,
        made_progress: allowed
            && truth
                .snapshots
                .iter()
                .flat_map(|snapshot| &snapshot.attempts)
                .any(|attempt| {
                    attempt.status == "verified"
                        && !attempt.proof_evidence_ids.is_empty()
                        && attempt.finding_id.is_some()
                        && attempt.finding_lineage_exact
                }),
        reopen_wave: false,
        durable_wave_cursor: true,
    }
}

/// metalcraft [`Reducer`] 状态：operation 流转的累积视图。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OperationFlowState {
    /// 调用方预置的「每个 stage 会产出什么」。增量 2+ 改为各 stage 真跑时实时填入。
    pub seeded: HashMap<StageKind, StageFlowOutcome>,
    /// 已执行的 stage，按顺序。
    pub visited: Vec<StageKind>,
    /// 已执行 stage 记录的产出（节点运行时从 `seeded` 拷入）—— 条件边读它选下一步。
    pub applied: HashMap<StageKind, StageFlowOutcome>,
    /// 波次循环（设计 2026-07-02-attack-stage §2.3 / §3.5）：当前波次（0 起）。每次
    /// `verification → attack_candidate` 游标覆写（开新波）时 +1，图层据它对燃料/深度
    /// 上限做终止判定（跨 resume 随 checkpoint 持久化）。加性字段，serde 缺省 0（旧
    /// checkpoint 反序列化不破，I10）。
    #[serde(default)]
    pub wave: u32,
    /// 瞬时路由标记：`verification` 节点跑完后写入——`true` = 开新波（游标覆写回
    /// `attack_candidate`），`false` = 走 DAG 正常后继（reporting/access_validation）。
    /// 决策来源是服务者（`consume_gate_outcome`）经 [`super::chain_wave::decide_chain_wave`]
    /// 对本波交付物 candidates 去重判定后，随 [`StageFlowOutcome::reopen_wave`] 回传；节点
    /// 再叠加燃料/深度上限（读本状态的 `wave`）。只由 `verification` 节点写、其条件边读；
    /// 每次 verification 跑完都会重写，无陈旧标记问题。加性字段，serde 缺省 false。
    #[serde(default)]
    pub reopen_wave: bool,
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
    /// 记录 `stage` 已执行（把其 `seeded` 产出拷进 `applied`）。增量 1 的 seeded 模型用。
    Executed(StageKind),
    /// 记录 `stage` 已执行并附上其**真实**产出。增量 4a 的 `StageRunner` 模型用——
    /// 产出来自真跑而非预置。
    ExecutedWith(StageKind, StageFlowOutcome),
    /// 返工/resume 时注入：覆盖某 stage 的预置产出（如「现在 gate 过了」）。
    SetOutcome(StageKind, StageFlowOutcome),
    /// 波次循环（设计 2026-07-02-attack-stage §3.5）：`verification` 跑完且判定开新波
    /// （服务者的 `reopen_wave` 信号 + 图层燃料/深度上限通过）——记录产出、`wave += 1`、
    /// 置 `reopen_wave`。`verification` 条件边随后读 `reopen_wave` 把游标覆写回
    /// `attack_candidate`。
    OpenNextWave(StageKind, StageFlowOutcome),
    /// 波次循环：`verification` 跑完但不开新波（无新假设 / 达上限）——记录产出并把
    /// `reopen_wave` 归位 `false`，让条件边走 DAG 正常后继（reporting）。
    CloseWaves(StageKind, StageFlowOutcome),
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
            FlowUpdate::ExecutedWith(stage, outcome) => {
                self.visited.push(stage);
                self.applied.insert(stage, outcome);
            }
            FlowUpdate::SetOutcome(stage, outcome) => {
                self.seeded.insert(stage, outcome);
            }
            FlowUpdate::OpenNextWave(stage, outcome) => {
                self.visited.push(stage);
                self.applied.insert(stage, outcome);
                self.wave = self.wave.saturating_add(1);
                self.reopen_wave = true;
            }
            FlowUpdate::CloseWaves(stage, outcome) => {
                self.visited.push(stage);
                self.applied.insert(stage, outcome);
                self.reopen_wave = false;
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

    g = add_flow_edges(g, dag);

    if let Some(&entry) = dag.entry_points().first() {
        g = g.set_entry(entry.as_str());
    }
    g.compile()
}

/// Resolve a stage's **normal** (non-wave) DAG successor from its precomputed
/// successor list: none → `END`; one → static; many → [`branch_target`] by
/// `made_progress`. Single-sourced so the wave-loop `verification` edge can fall
/// back to it when no new wave opens (avoids capturing the whole `AllowedDag`).
fn normal_next_from(nexts: &[StageKind], stage: StageKind, state: &OperationFlowState) -> String {
    match nexts {
        [] => END.to_string(),
        [only] => only.as_str().to_string(),
        _ => {
            let progressed = state
                .applied
                .get(&stage)
                .map(|o| o.made_progress)
                .unwrap_or(true);
            branch_target(nexts, progressed)
                .unwrap_or(stage)
                .as_str()
                .to_string()
        }
    }
}

/// Add the stage-flow edges (single successor → static; multiple → conditional
/// via [`branch_target`]; terminal → `END`) shared by both the seeded
/// ([`build_operation_flow_graph`]) and runner-driven ([`build_runner_graph`])
/// builders.
///
/// Wave loop (设计 2026-07-02-attack-stage §2.3): when `attack_candidate` is in
/// the projected DAG, `verification`'s edge becomes a **conditional cursor
/// override** — `reopen_wave` (set by the verification node via
/// [`super::chain_wave::decide_chain_wave`]) routes back to `attack_candidate`
/// (a new wave), otherwise the normal DAG successor. The DAG stays acyclic (no
/// `verification → attack_candidate` edge in `operation_graph.json`); the cycle
/// lives only in this runtime-resolved conditional edge, bounded by the wave
/// fuel/depth caps baked into the decision + the executor's `max_steps`.
fn add_flow_edges(
    mut g: super::graph_engine::Graph<OperationFlowState>,
    dag: &AllowedDag,
) -> super::graph_engine::Graph<OperationFlowState> {
    let wave_loop = dag.nodes.contains(&StageKind::AttackCandidate);
    for &stage in &dag.nodes {
        if wave_loop && stage == StageKind::Verification {
            let verification_nexts = dag.next_stages(StageKind::Verification);
            g = g.add_conditional(stage.as_str(), move |state: &OperationFlowState| {
                if state.reopen_wave {
                    StageKind::AttackCandidate.as_str().to_string()
                } else {
                    normal_next_from(&verification_nexts, StageKind::Verification, state)
                }
            });
            continue;
        }
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
    g
}

/// 构造一个挂了 [`MemoryCheckpointer`] 的 [`Executor`]（支持 run + resume）。
pub fn operation_flow_executor(
    dag: &AllowedDag,
) -> Result<Executor<OperationFlowState>, GraphError> {
    Ok(Executor::new(build_operation_flow_graph(dag)?)
        .with_checkpointer(Arc::new(MemoryCheckpointer::new())))
}

/// 增量 4a · 控制反转抽象：让 metalcraft [`Executor`] 真正驱动 operation。
///
/// 一个 `StageRunner` 知道「如何把某个 stage 端到端跑完」（其 subtasks + gate），
/// 并报告 [`StageFlowOutcome`]。真 orchestrator 在增量 4b 实现它；图节点体只持有
/// `Arc<dyn StageRunner>`（解决 metalcraft `Node` 闭包拿不到 orchestrator `&mut self`
/// 的难题）。
#[async_trait::async_trait]
pub trait StageRunner: Send + Sync {
    /// 端到端跑完一个 stage（subtasks + gate），返回流转所需的产出。
    async fn run_stage(&self, stage: StageKind) -> StageFlowOutcome;
}

fn verification_wave_update(
    current_wave: u32,
    stage: StageKind,
    outcome: StageFlowOutcome,
) -> FlowUpdate {
    let next_wave = current_wave.saturating_add(1);
    let wave_cap =
        super::chain_wave::DEFAULT_MAX_WAVES.min(super::chain_wave::DEFAULT_MAX_CHAIN_DEPTH);
    let within_caps = next_wave <= wave_cap;
    if outcome.reopen_wave && (outcome.durable_wave_cursor || within_caps) {
        FlowUpdate::OpenNextWave(stage, outcome)
    } else {
        FlowUpdate::CloseWaves(stage, outcome)
    }
}

/// 增量 4a · 构造一张**由 [`StageRunner`] 真驱动**的 metalcraft 图：每个 stage 节点体
/// 调 `runner.run_stage(stage)`，产出 → [`FlowUpdate::ExecutedWith`]；gate 未过 →
/// `Interrupt`（暂停返工，可 resume）。分支/终点边复用 [`add_flow_edges`]。
///
/// 仍是 additive：把 live `run()` 切到「Executor 驱动这张图」是增量 4c（flag 闸）。
pub fn build_runner_graph(
    dag: &AllowedDag,
    runner: Arc<dyn StageRunner>,
) -> Result<CompiledGraph<OperationFlowState>, GraphError> {
    let mut g = super::graph_engine::Graph::<OperationFlowState>::new();

    // Wave loop (设计 2026-07-02-attack-stage §3.5): when attack_candidate is in
    // the projected DAG, the `verification` node consumes the servicer's
    // `reopen_wave` signal (computed by `consume_gate_outcome` via
    // `decide_chain_wave` over the deliverable's candidates + cross-wave dedupe)
    // and additionally enforces a hard graph-layer fuel/depth cap on `state.wave`
    // — defense-in-depth so a runaway signal can never loop past the budget.
    // OpenNextWave → the conditional edge (add_flow_edges) overwrites the cursor
    // back to attack_candidate; else the DAG's normal successor. Until the
    // runtime sets `reopen_wave` (verification deliverable candidates), it is
    // always false → live routing is byte-identical.
    let wave_loop = dag.nodes.contains(&StageKind::AttackCandidate);

    for &stage in &dag.nodes {
        let s = stage;
        let runner = Arc::clone(&runner);
        if wave_loop && stage == StageKind::Verification {
            g = g.add_node(stage.as_str(), move |state: OperationFlowState| {
                let runner = Arc::clone(&runner);
                async move {
                    let outcome = runner.run_stage(s).await;
                    if !outcome.gate_allowed {
                        return Ok(NodeOutcome::interrupt(format!(
                            "gate blocked at stage '{}' — hold for rework",
                            s.as_str()
                        )));
                    }
                    // Hard graph-layer cap: the next wave (`state.wave + 1`) must
                    // stay inside the tighter of the fuel/depth budgets (mirrors
                    // `decide_chain_wave`). Independent of the servicer's signal so
                    // the loop is provably bounded even if the signal misbehaves.
                    let update = verification_wave_update(state.wave, s, outcome);
                    Ok(NodeOutcome::Update(update))
                }
            });
            continue;
        }
        g = g.add_node(stage.as_str(), move |_state: OperationFlowState| {
            let runner = Arc::clone(&runner);
            async move {
                let outcome = runner.run_stage(s).await;
                if outcome.gate_allowed {
                    Ok(NodeOutcome::Update(FlowUpdate::ExecutedWith(s, outcome)))
                } else {
                    Ok(NodeOutcome::interrupt(format!(
                        "gate blocked at stage '{}' — hold for rework",
                        s.as_str()
                    )))
                }
            }
        });
    }

    g = add_flow_edges(g, dag);

    if let Some(&entry) = dag.entry_points().first() {
        g = g.set_entry(entry.as_str());
    }
    g.compile()
}

/// C-2 · one stage-execution request a graph node sends to the external servicer
/// (the orchestrator's `&mut self` loop) when the Executor truly owns the loop.
pub struct StageRunRequest {
    pub stage: StageKind,
    /// Channel the servicer replies on with the stage's flow outcome.
    pub reply: tokio::sync::oneshot::Sender<StageFlowOutcome>,
}

/// C-2 · a [`StageRunner`] that delegates `run_stage` over a channel to an
/// external servicer instead of running the stage itself.
///
/// This is the key that lets the metalcraft [`Executor`] **truly own the
/// top-level loop** (= user-chosen option C) while stage execution stays where
/// it must live — the orchestrator's `&mut self` method holding the borrowed
/// `&dyn AgentExecutor`. The node body only captures a `Send + Sync + 'static`
/// channel sender (no executor borrow, no `&mut self`), sidestepping the
/// 'static-node ↔ borrowed-executor lifetime mismatch. The servicer (orchestrator)
/// runs the metalcraft `Executor` and a request-servicing loop concurrently
/// (`tokio::select!`), running each requested stage with `&mut self`.
pub struct ChannelStageRunner {
    tx: tokio::sync::mpsc::Sender<StageRunRequest>,
}

impl ChannelStageRunner {
    pub fn new(tx: tokio::sync::mpsc::Sender<StageRunRequest>) -> Self {
        Self { tx }
    }
}

#[async_trait::async_trait]
impl StageRunner for ChannelStageRunner {
    async fn run_stage(&self, stage: StageKind) -> StageFlowOutcome {
        let (reply, rx) = tokio::sync::oneshot::channel();
        // Servicer gone / not listening → treat as a blocked gate so the flow
        // halts safely (Interrupt) rather than silently advancing.
        if self
            .tx
            .send(StageRunRequest { stage, reply })
            .await
            .is_err()
        {
            return StageFlowOutcome::blocked();
        }
        rx.await.unwrap_or_else(|_| StageFlowOutcome::blocked())
    }
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
    const PENTEST_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/pentest.json");

    fn assessment_dag() -> AllowedDag {
        let g = load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph");
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        g.project(&p.allowed_stage_set())
    }

    fn pentest_dag() -> AllowedDag {
        let g = load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph");
        let p = load_profile_from_json(PENTEST_JSON).expect("pentest profile");
        g.project(&p.allowed_stage_set())
    }

    /// MockRunner outcomes that steer the pentest DAG's branch edges onto the
    /// attack path (eas→enumeration→vuln_triage→attack_candidate→verification).
    /// Enumeration is `findings_allowed=false`, but DB/ledger-backed content
    /// enumeration still counts as progress; progress routes to the main edge
    /// (`vuln_triage`), while no progress bails to reporting.
    fn attack_path_outcomes() -> HashMap<StageKind, StageFlowOutcome> {
        seed(&[
            (
                StageKind::ExternalAttackSurface,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::Enumeration,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::VulnTriage,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::AttackCandidate,
                StageFlowOutcome::pass_with_progress(),
            ),
            (
                StageKind::Verification,
                StageFlowOutcome::pass_with_progress(),
            ),
        ])
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
    fn chosen_next_stage_branch_routes_by_progress() {
        let branch = TransitionDecision::Branch(vec![StageKind::Enumeration, StageKind::Reporting]);
        // No progress → bail to reporting (the win the legacy first-candidate
        // path never took).
        assert_eq!(
            chosen_next_stage(&branch, false),
            Some(StageKind::Reporting)
        );
        // Progress → main path (first candidate).
        assert_eq!(
            chosen_next_stage(&branch, true),
            Some(StageKind::Enumeration)
        );
        // Non-branch decisions resolve via advance_target.
        assert_eq!(
            chosen_next_stage(&TransitionDecision::Advance(StageKind::TargetIntel), false),
            Some(StageKind::TargetIntel)
        );
        assert_eq!(chosen_next_stage(&TransitionDecision::Hold, true), None);
        assert_eq!(chosen_next_stage(&TransitionDecision::Complete, true), None);
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

    // ── 增量 4a · StageRunner-driven graph (control inversion foundation) ──

    struct MockRunner {
        outcomes: HashMap<StageKind, StageFlowOutcome>,
    }

    #[async_trait::async_trait]
    impl StageRunner for MockRunner {
        async fn run_stage(&self, stage: StageKind) -> StageFlowOutcome {
            self.outcomes
                .get(&stage)
                .copied()
                .unwrap_or_else(StageFlowOutcome::pass_with_progress)
        }
    }

    #[tokio::test]
    async fn runner_graph_executor_drives_full_recon_path() {
        // The Executor drives the whole operation by calling the runner per
        // stage (control inversion): every stage makes progress → linear path.
        let dag = assessment_dag();
        let runner = Arc::new(MockRunner {
            outcomes: HashMap::new(),
        });
        let graph = build_runner_graph(&dag, runner).expect("graph");
        match Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run")
        {
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
    async fn runner_graph_bails_when_runner_reports_no_progress() {
        let dag = assessment_dag();
        let runner = Arc::new(MockRunner {
            outcomes: seed(&[(
                StageKind::ExternalAttackSurface,
                StageFlowOutcome::pass_no_progress(),
            )]),
        });
        let graph = build_runner_graph(&dag, runner).expect("graph");
        match Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run")
        {
            RunOutcome::Completed(s) => {
                assert!(!s.visited.contains(&StageKind::Enumeration));
                assert!(s.visited.contains(&StageKind::Reporting));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn runner_graph_interrupts_when_runner_reports_blocked_gate() {
        let dag = assessment_dag();
        let runner = Arc::new(MockRunner {
            outcomes: seed(&[(
                StageKind::ExternalAttackSurface,
                StageFlowOutcome::blocked(),
            )]),
        });
        let graph = build_runner_graph(&dag, runner).expect("graph");
        match Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run")
        {
            RunOutcome::Interrupted { resume_from, .. } => {
                assert_eq!(resume_from, StageKind::ExternalAttackSurface.as_str())
            }
            other => panic!("expected Interrupted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn channel_runner_lets_executor_drive_via_external_servicer() {
        // C-2: the Executor owns the loop; a separate servicer task replies to
        // each stage request (mirrors the orchestrator's &mut self servicing).
        let dag = assessment_dag();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<StageRunRequest>(8);
        let servicer = tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                let _ = req.reply.send(StageFlowOutcome::pass_with_progress());
            }
        });

        let runner = Arc::new(ChannelStageRunner::new(tx));
        let graph = build_runner_graph(&dag, runner).expect("graph");
        let out = Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run");

        match out {
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
        servicer.abort();
    }

    // ── 波次循环（chain-wave cursor override, 设计 2026-07-02-attack-stage §3.5） ──

    /// attack_path outcomes with `verification` reporting the servicer's
    /// `reopen_wave=true` signal (as `consume_gate_outcome` would after
    /// `decide_chain_wave` finds a new candidate). The graph-layer fuel/depth cap
    /// bounds the loop even though the mock signals reopen every time.
    fn attack_path_outcomes_reopening() -> HashMap<StageKind, StageFlowOutcome> {
        let mut m = attack_path_outcomes();
        m.insert(
            StageKind::Verification,
            StageFlowOutcome::pass_reopen_wave(),
        );
        m
    }

    #[tokio::test]
    async fn verification_reopen_signal_loops_bounded_by_depth_cap() {
        // The servicer signals reopen every verification; the graph node's hard
        // fuel/depth cap (DEFAULT_MAX_CHAIN_DEPTH = 3) bounds it: waves open at
        // state.wave 0,1,2 then stop when the next wave would exceed depth 3.
        let dag = pentest_dag();
        let runner = Arc::new(MockRunner {
            outcomes: attack_path_outcomes_reopening(),
        });
        let graph = build_runner_graph(&dag, runner).expect("graph");
        match Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run")
        {
            RunOutcome::Completed(s) => {
                let vc = s
                    .visited
                    .iter()
                    .filter(|&&v| v == StageKind::Verification)
                    .count();
                let ac = s
                    .visited
                    .iter()
                    .filter(|&&v| v == StageKind::AttackCandidate)
                    .count();
                // 3 reopens (waves 1,2,3) + the initial pass = 4 verifications;
                // attack_candidate runs initially + once per reopen = 4.
                assert_eq!(vc, 4, "verification runs 4× (initial + 3 bounded waves)");
                assert_eq!(ac, 4, "attack_candidate runs 4× (initial + 3 reopened)");
                assert_eq!(s.wave, 3, "wave counter stops at the depth cap");
                assert_eq!(*s.visited.last().unwrap(), StageKind::Reporting);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn durable_v2_cursor_is_not_overridden_by_process_local_wave_cap() {
        let cap = super::super::chain_wave::DEFAULT_MAX_WAVES
            .min(super::super::chain_wave::DEFAULT_MAX_CHAIN_DEPTH);
        let legacy = StageFlowOutcome::pass_reopen_wave();
        assert!(matches!(
            verification_wave_update(cap, StageKind::Verification, legacy),
            FlowUpdate::CloseWaves(StageKind::Verification, _)
        ));

        let mut durable = StageFlowOutcome::pass_reopen_wave();
        durable.durable_wave_cursor = true;
        assert!(matches!(
            verification_wave_update(cap, StageKind::Verification, durable),
            FlowUpdate::OpenNextWave(StageKind::Verification, _)
        ));
    }

    #[tokio::test]
    async fn verification_advances_when_no_reopen_signal() {
        // No reopen signal (the live default until the runtime sets it from the
        // verification deliverable) → verification takes its normal DAG successor
        // (reporting), byte-identical to the pre-wave flow.
        let dag = pentest_dag();
        let runner = Arc::new(MockRunner {
            outcomes: attack_path_outcomes(),
        });
        let graph = build_runner_graph(&dag, runner).expect("graph");
        match Executor::new(graph)
            .run(OperationFlowState::default(), "op")
            .await
            .expect("run")
        {
            RunOutcome::Completed(s) => {
                assert_eq!(
                    s.visited
                        .iter()
                        .filter(|&&v| v == StageKind::Verification)
                        .count(),
                    1,
                    "no wave loop without a reopen signal"
                );
                assert_eq!(s.wave, 0);
                assert_eq!(*s.visited.last().unwrap(), StageKind::Reporting);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn open_next_wave_update_increments_wave_and_sets_flag() {
        let mut s = OperationFlowState::default();
        s.apply(FlowUpdate::OpenNextWave(
            StageKind::Verification,
            StageFlowOutcome::pass_reopen_wave(),
        ));
        assert_eq!(s.wave, 1);
        assert!(s.reopen_wave);
        s.apply(FlowUpdate::CloseWaves(
            StageKind::Verification,
            StageFlowOutcome::pass_with_progress(),
        ));
        assert!(!s.reopen_wave, "CloseWaves clears the reopen flag");
    }

    #[test]
    fn flow_state_serde_round_trips() {
        // DB checkpointer (C-4) persists OperationFlowState as JSON.
        let mut s = OperationFlowState::default();
        s.apply(FlowUpdate::ExecutedWith(
            StageKind::Scoping,
            StageFlowOutcome::pass_with_progress(),
        ));
        let json = serde_json::to_string(&s).expect("serialize");
        let back: OperationFlowState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.visited, vec![StageKind::Scoping]);
        assert!(back.applied.contains_key(&StageKind::Scoping));
    }
}
