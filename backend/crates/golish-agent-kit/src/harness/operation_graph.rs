//! Operation DAG · Base graph loader + Profile 投影 + 流转 (Doc 3 §3.1 / §3.3 / §6.2).
//!
//! Phase 2 第 2 层引擎: 把静态 `resources/harness/graph/operation_graph.json` 接进运行时.
//!
//! - [`load_operation_graph_from_json`] : JSON → [`OperationGraph`] (nodes + edges), 含校验
//! - [`OperationGraph::project`]        : Base Graph → [`AllowedDag`] (Doc 3 §3.3 profile 投影)
//! - [`AllowedDag::next_stages`]        : 沿边给出下一可达 stage 候选 (Doc 3 §6.2 step 10)
//!
//! 本模块**只**管拓扑: 加载图、按 profile 投影、算下一 stage 候选. 纯内存 + 可单测,
//! 不碰 DB (那是 `operation_state` repo) 也不做授权 / 审批 (那是
//! `pre_action_authorizer` + `Profile::approval_policy`). next_stages 返回的是
//! **拓扑候选**, 调用方再叠加 authz / approval 闸.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::types::StageKind;

/// operation_graph.json `edges[*]` 元素.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageEdge {
    pub from: StageKind,
    pub to: StageKind,
}

impl StageEdge {
    pub fn new(from: StageKind, to: StageKind) -> Self {
        Self { from, to }
    }
}

/// Doc 3 §3.1 Base Operation Graph · `resources/harness/graph/operation_graph.json` 映射.
///
/// serde 默认忽略未知字段, 所以 `$schema` / `$comment` 注释字段不会报错.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationGraph {
    pub nodes: Vec<StageKind>,
    pub edges: Vec<StageEdge>,
}

#[derive(Debug, Error)]
pub enum OperationGraphError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("edge references stage not declared in nodes[]: {0:?}")]
    UnknownNodeInEdge(StageKind),
    #[error("operation graph is not a DAG: cycle involves stage {0:?}")]
    Cycle(StageKind),
}

/// 计算 DAG 切片 (`from`..=`to`) 时的错误 (headless 单/区间阶段实跑 · 方案 2).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SliceError {
    #[error("target stage not in projected DAG: {0:?}")]
    ToNotInDag(StageKind),
    #[error("from stage not in projected DAG: {0:?}")]
    FromNotInDag(StageKind),
    #[error("from stage {from:?} cannot reach to stage {to:?} along the DAG")]
    FromCannotReachTo { from: StageKind, to: StageKind },
}

/// 静态 JSON 字符串 → [`OperationGraph`], 并校验 (未知节点 + 无环).
///
/// 真正从 disk 读由调用方做 (`std::fs::read_to_string`); 单测可无 IO 直接 fixture 验证.
pub fn load_operation_graph_from_json(raw: &str) -> Result<OperationGraph, OperationGraphError> {
    let graph: OperationGraph = serde_json::from_str(raw)?;
    graph.validate()?;
    Ok(graph)
}

/// 加载内置的 Base Operation Graph (随二进制编译进来).
///
/// 运行时接线用; 等价于对 `operation_graph.json` 调 [`load_operation_graph_from_json`].
pub fn base_operation_graph() -> Result<OperationGraph, OperationGraphError> {
    const BASE_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    load_operation_graph_from_json(BASE_JSON)
}

impl OperationGraph {
    fn node_set(&self) -> HashSet<StageKind> {
        self.nodes.iter().copied().collect()
    }

    /// 邻接表 from -> [to...]; 每个声明的节点都有条目 (可能为空 Vec).
    /// `to` 按 `edges[]` 出现顺序排列, 保证 next_stages 候选顺序确定.
    fn adjacency(&self) -> HashMap<StageKind, Vec<StageKind>> {
        let mut adj: HashMap<StageKind, Vec<StageKind>> = HashMap::new();
        for &n in &self.nodes {
            adj.entry(n).or_default();
        }
        for e in &self.edges {
            adj.entry(e.from).or_default().push(e.to);
        }
        adj
    }

    /// 校验: ① 每条边两端都在 nodes[] ② 图无环 (Kahn 拓扑排序).
    pub fn validate(&self) -> Result<(), OperationGraphError> {
        let nodes = self.node_set();
        for e in &self.edges {
            if !nodes.contains(&e.from) {
                return Err(OperationGraphError::UnknownNodeInEdge(e.from));
            }
            if !nodes.contains(&e.to) {
                return Err(OperationGraphError::UnknownNodeInEdge(e.to));
            }
        }
        if let Some(stage) = self.first_cyclic_stage() {
            return Err(OperationGraphError::Cycle(stage));
        }
        Ok(())
    }

    /// Kahn 拓扑排序; 返回仍处于环中的某个 stage (无环则 None).
    /// 仅在 [`Self::validate`] 内调用, 此时所有边端点已确认在 nodes[].
    fn first_cyclic_stage(&self) -> Option<StageKind> {
        let mut in_degree: HashMap<StageKind, usize> =
            self.nodes.iter().map(|&n| (n, 0usize)).collect();
        for e in &self.edges {
            if let Some(d) = in_degree.get_mut(&e.to) {
                *d += 1;
            }
        }
        let adj = self.adjacency();
        let mut queue: Vec<StageKind> = self
            .nodes
            .iter()
            .copied()
            .filter(|n| in_degree.get(n).copied().unwrap_or(0) == 0)
            .collect();
        let mut visited = 0usize;
        while let Some(n) = queue.pop() {
            visited += 1;
            if let Some(neighbors) = adj.get(&n) {
                for &m in neighbors {
                    if let Some(d) = in_degree.get_mut(&m) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push(m);
                        }
                    }
                }
            }
        }
        if visited == self.nodes.len() {
            None
        } else {
            self.nodes
                .iter()
                .copied()
                .find(|n| in_degree.get(n).copied().unwrap_or(0) > 0)
        }
    }

    /// Doc 3 §3.3 · 把 Base Graph 投影成 profile 可达子图.
    ///
    /// 只保留 `allowed` 集合里的节点, 以及**两端都在** `allowed` 的边
    /// (任一端被 forbidden 的边连带剪掉). 传入 [`super::profile::Profile::allowed_stage_set`].
    pub fn project(&self, allowed: &HashSet<StageKind>) -> AllowedDag {
        let nodes: Vec<StageKind> = self
            .nodes
            .iter()
            .copied()
            .filter(|n| allowed.contains(n))
            .collect();
        let kept: HashSet<StageKind> = nodes.iter().copied().collect();
        let edges: Vec<StageEdge> = self
            .edges
            .iter()
            .copied()
            .filter(|e| kept.contains(&e.from) && kept.contains(&e.to))
            .collect();
        AllowedDag::new(nodes, edges)
    }
}

/// Profile 投影后的可达 Operation DAG (Doc 3 §3.3).
///
/// 持有节点 / 边 + 预算好的邻接表; 流转引擎用 [`Self::next_stages`] 选下一 stage.
#[derive(Debug, Clone)]
pub struct AllowedDag {
    pub nodes: Vec<StageKind>,
    pub edges: Vec<StageEdge>,
    adjacency: HashMap<StageKind, Vec<StageKind>>,
}

impl AllowedDag {
    fn new(nodes: Vec<StageKind>, edges: Vec<StageEdge>) -> Self {
        let mut adjacency: HashMap<StageKind, Vec<StageKind>> = HashMap::new();
        for &n in &nodes {
            adjacency.entry(n).or_default();
        }
        for e in &edges {
            adjacency.entry(e.from).or_default().push(e.to);
        }
        Self {
            nodes,
            edges,
            adjacency,
        }
    }

    /// Doc 3 §6.2 step 10 · 当前 stage 沿边可达的下一 stage 候选 (拓扑层面).
    ///
    /// 顺序与 `operation_graph.json` 中边的声明顺序一致. 不在图里的 stage 返回空.
    pub fn next_stages(&self, current: StageKind) -> Vec<StageKind> {
        self.adjacency.get(&current).cloned().unwrap_or_default()
    }

    /// stage 是否在本可达子图内.
    pub fn contains(&self, stage: StageKind) -> bool {
        self.adjacency.contains_key(&stage)
    }

    /// 是否为终点 (无出边). 不在图里的 stage 视为终点.
    pub fn is_terminal(&self, stage: StageKind) -> bool {
        self.adjacency.get(&stage).is_none_or(|v| v.is_empty())
    }

    /// 入口 stage (无入边); operation 起始候选. 顺序同 nodes[].
    pub fn entry_points(&self) -> Vec<StageKind> {
        let with_incoming: HashSet<StageKind> = self.edges.iter().map(|e| e.to).collect();
        self.nodes
            .iter()
            .copied()
            .filter(|n| !with_incoming.contains(n))
            .collect()
    }

    /// 终点 stage (无出边). 顺序同 nodes[].
    pub fn terminals(&self) -> Vec<StageKind> {
        self.nodes
            .iter()
            .copied()
            .filter(|&n| self.is_terminal(n))
            .collect()
    }

    /// `start` 及其沿正向边可达的所有后继 (含自身). `start` 不在图里返回空集.
    ///
    /// 方案 2 切片用: `descendants_inclusive(from)` = 「from 往后能到的阶段」.
    pub fn descendants_inclusive(&self, start: StageKind) -> HashSet<StageKind> {
        let mut seen = HashSet::new();
        if !self.contains(start) {
            return seen;
        }
        let mut stack = vec![start];
        while let Some(s) = stack.pop() {
            if seen.insert(s) {
                if let Some(neigh) = self.adjacency.get(&s) {
                    for &n in neigh {
                        if !seen.contains(&n) {
                            stack.push(n);
                        }
                    }
                }
            }
        }
        seen
    }

    /// `target` 及其沿反向边可达的所有前驱 (含自身). `target` 不在图里返回空集.
    ///
    /// 方案 2 切片用: `ancestors_inclusive(to)` = 「能走到 to 的阶段」(即 to 的上游闭包).
    pub fn ancestors_inclusive(&self, target: StageKind) -> HashSet<StageKind> {
        let mut seen = HashSet::new();
        if !self.contains(target) {
            return seen;
        }
        let mut rev: HashMap<StageKind, Vec<StageKind>> = HashMap::new();
        for e in &self.edges {
            rev.entry(e.to).or_default().push(e.from);
        }
        let mut stack = vec![target];
        while let Some(s) = stack.pop() {
            if seen.insert(s) {
                if let Some(preds) = rev.get(&s) {
                    for &p in preds {
                        if !seen.contains(&p) {
                            stack.push(p);
                        }
                    }
                }
            }
        }
        seen
    }

    /// 方案 2 · headless 单/区间阶段实跑的 DAG 切片.
    ///
    /// 返回 `from`..=`to` 路径上的全部阶段 = `ancestors_inclusive(to)` (∩
    /// `descendants_inclusive(from)` 若给了 `from`). 把它当 allowlist 喂给
    /// orchestrator 投影 (`allowed ∩ allowlist`) 后, 子图入口=`from`/entry、终点=`to`
    /// (to 的下游被剪掉), 跑完 `to` 因无后继自然 Complete 停下.
    ///
    /// - `from = None` ⇒ 从 DAG 入口起 (= `to` 的全部上游).
    /// - `--only X` ⇒ `slice(Some(X), X)` = `{X}` (单节点).
    pub fn slice(
        &self,
        from: Option<StageKind>,
        to: StageKind,
    ) -> Result<HashSet<StageKind>, SliceError> {
        if !self.contains(to) {
            return Err(SliceError::ToNotInDag(to));
        }
        let mut set = self.ancestors_inclusive(to);
        if let Some(f) = from {
            if !self.contains(f) {
                return Err(SliceError::FromNotInDag(f));
            }
            let fwd = self.descendants_inclusive(f);
            if !fwd.contains(&to) {
                return Err(SliceError::FromCannotReachTo { from: f, to });
            }
            set.retain(|s| fwd.contains(s));
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::profile::load_profile_from_json;

    const BASE_GRAPH_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");
    const PENTEST_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/pentest.json");

    fn base() -> OperationGraph {
        load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph parses + validates")
    }

    fn assessment_dag() -> AllowedDag {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        base().project(&p.allowed_stage_set())
    }

    fn pentest_dag() -> AllowedDag {
        let p = load_profile_from_json(PENTEST_JSON).expect("pentest profile");
        base().project(&p.allowed_stage_set())
    }

    #[test]
    fn base_graph_has_13_nodes_17_edges() {
        let g = base();
        assert_eq!(g.nodes.len(), 13);
        // 12 linear stage-flow edges + 5 bail-to-reporting shortcuts
        // (external_attack_surface / enumeration / vuln_triage / attack_candidate
        // / verification). attack_candidate splits the old vuln_triage->verification
        // edge into vuln_triage->attack_candidate->verification (+1 linear, +1 bail).
        assert_eq!(g.edges.len(), 17);
    }

    #[test]
    fn base_graph_loads_via_include() {
        // 内置加载与显式 from_json 等价.
        let g = base_operation_graph().expect("built-in base graph");
        assert_eq!(g.nodes.len(), 13);
    }

    #[test]
    fn base_graph_is_acyclic() {
        // load_* 已 validate; 这里直接再确认一次 first_cyclic_stage 为 None.
        assert!(base().first_cyclic_stage().is_none());
    }

    #[test]
    fn unknown_node_in_edge_is_rejected() {
        let g = OperationGraph {
            nodes: vec![StageKind::Scoping, StageKind::TargetIntel],
            // Enumeration 不在 nodes[] -> UnknownNodeInEdge
            edges: vec![StageEdge::new(StageKind::Scoping, StageKind::Enumeration)],
        };
        match g.validate() {
            Err(OperationGraphError::UnknownNodeInEdge(StageKind::Enumeration)) => {}
            other => panic!("expected UnknownNodeInEdge(Enumeration), got {other:?}"),
        }
    }

    #[test]
    fn cycle_is_rejected() {
        let g = OperationGraph {
            nodes: vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::Enumeration,
            ],
            edges: vec![
                StageEdge::new(StageKind::Scoping, StageKind::TargetIntel),
                StageEdge::new(StageKind::TargetIntel, StageKind::Enumeration),
                StageEdge::new(StageKind::Enumeration, StageKind::Scoping),
            ],
        };
        assert!(matches!(g.validate(), Err(OperationGraphError::Cycle(_))));
    }

    #[test]
    fn project_assessment_yields_5_nodes_5_edges() {
        let dag = assessment_dag();
        assert_eq!(dag.nodes.len(), 5, "assessment allows 5 stages");
        // scoping->target_intel, target_intel->eas, eas->enumeration,
        // eas->reporting, enumeration->reporting
        assert_eq!(dag.edges.len(), 5);
        for n in [
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::Reporting,
        ] {
            assert!(dag.contains(n), "assessment DAG should contain {n:?}");
        }
    }

    #[test]
    fn project_prunes_forbidden_stages_and_edges() {
        let dag = assessment_dag();
        // vuln_triage 被 forbidden -> 节点不在, enumeration->vuln_triage 边连带剪掉.
        assert!(!dag.contains(StageKind::VulnTriage));
        assert!(!dag.contains(StageKind::Cleanup));
        assert!(!dag
            .next_stages(StageKind::Enumeration)
            .contains(&StageKind::VulnTriage));
    }

    #[test]
    fn next_stages_external_attack_surface_branches() {
        let dag = assessment_dag();
        // edges 声明顺序: eas->enumeration 在 eas->reporting 之前.
        assert_eq!(
            dag.next_stages(StageKind::ExternalAttackSurface),
            vec![StageKind::Enumeration, StageKind::Reporting]
        );
    }

    #[test]
    fn next_stages_enumeration_branches_to_vuln_triage_before_reporting() {
        let dag = pentest_dag();
        // Branch order is runtime semantics: progress -> first/main edge, no
        // progress -> last/bail edge. Attack-capable profiles must continue from
        // enumeration into vuln_triage when enumeration produced a testable surface.
        assert_eq!(
            dag.next_stages(StageKind::Enumeration),
            vec![StageKind::VulnTriage, StageKind::Reporting]
        );
    }

    #[test]
    fn next_stages_linear_recon_path() {
        let dag = assessment_dag();
        assert_eq!(
            dag.next_stages(StageKind::Scoping),
            vec![StageKind::TargetIntel]
        );
        assert_eq!(
            dag.next_stages(StageKind::TargetIntel),
            vec![StageKind::ExternalAttackSurface]
        );
        assert_eq!(
            dag.next_stages(StageKind::Enumeration),
            vec![StageKind::Reporting]
        );
    }

    #[test]
    fn reporting_is_terminal_scoping_is_entry() {
        let dag = assessment_dag();
        assert!(dag.next_stages(StageKind::Reporting).is_empty());
        assert!(dag.is_terminal(StageKind::Reporting));
        assert_eq!(dag.entry_points(), vec![StageKind::Scoping]);
        assert_eq!(dag.terminals(), vec![StageKind::Reporting]);
    }

    #[test]
    fn stage_not_in_dag_has_no_next_and_is_terminal() {
        let dag = assessment_dag();
        // VulnTriage 被投影剪掉 -> 不在图, next 空, 视为 terminal.
        assert!(dag.next_stages(StageKind::VulnTriage).is_empty());
        assert!(dag.is_terminal(StageKind::VulnTriage));
        assert!(!dag.contains(StageKind::VulnTriage));
    }

    #[test]
    fn stage_edge_serde_snake_case() {
        let e = StageEdge::new(StageKind::Scoping, StageKind::TargetIntel);
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, r#"{"from":"scoping","to":"target_intel"}"#);
        let back: StageEdge = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    // ---- 方案 2 · DAG 切片 (headless 单/区间阶段实跑) ----

    #[test]
    fn ancestors_inclusive_of_eas_is_recon_prefix() {
        let dag = assessment_dag();
        let anc = dag.ancestors_inclusive(StageKind::ExternalAttackSurface);
        assert_eq!(
            anc,
            HashSet::from([
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
            ])
        );
    }

    #[test]
    fn descendants_inclusive_of_target_intel_reaches_reporting() {
        let dag = assessment_dag();
        let desc = dag.descendants_inclusive(StageKind::TargetIntel);
        // target_intel -> eas -> {enumeration, reporting}; enumeration -> reporting.
        assert_eq!(
            desc,
            HashSet::from([
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::Reporting,
            ])
        );
    }

    #[test]
    fn slice_only_single_stage_is_just_that_stage() {
        let dag = assessment_dag();
        let s = dag
            .slice(Some(StageKind::TargetIntel), StageKind::TargetIntel)
            .expect("target_intel is in the assessment DAG");
        assert_eq!(s, HashSet::from([StageKind::TargetIntel]));
    }

    #[test]
    fn slice_to_target_intel_from_entry_is_scoping_plus_intel() {
        let dag = assessment_dag();
        let s = dag
            .slice(None, StageKind::TargetIntel)
            .expect("target_intel reachable from entry");
        assert_eq!(
            s,
            HashSet::from([StageKind::Scoping, StageKind::TargetIntel])
        );
    }

    #[test]
    fn slice_to_eas_from_entry_is_recon_prefix() {
        let dag = assessment_dag();
        let s = dag
            .slice(None, StageKind::ExternalAttackSurface)
            .expect("eas reachable from entry");
        assert_eq!(
            s,
            HashSet::from([
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
            ])
        );
    }

    #[test]
    fn slice_to_reporting_covers_all_projected_nodes() {
        let dag = assessment_dag();
        let s = dag.slice(None, StageKind::Reporting).expect("reporting");
        let all: HashSet<StageKind> = dag.nodes.iter().copied().collect();
        assert_eq!(s, all);
    }

    #[test]
    fn slice_to_stage_not_in_dag_errs() {
        let dag = assessment_dag();
        assert_eq!(
            dag.slice(None, StageKind::VulnTriage),
            Err(SliceError::ToNotInDag(StageKind::VulnTriage))
        );
    }

    #[test]
    fn slice_from_stage_not_in_dag_errs() {
        let dag = assessment_dag();
        assert_eq!(
            dag.slice(Some(StageKind::VulnTriage), StageKind::Reporting),
            Err(SliceError::FromNotInDag(StageKind::VulnTriage))
        );
    }

    #[test]
    fn slice_from_cannot_reach_to_errs() {
        let dag = assessment_dag();
        // reporting is downstream of scoping; you cannot slice "from reporting to scoping".
        assert_eq!(
            dag.slice(Some(StageKind::Reporting), StageKind::Scoping),
            Err(SliceError::FromCannotReachTo {
                from: StageKind::Reporting,
                to: StageKind::Scoping,
            })
        );
    }

    #[test]
    fn project_with_single_stage_allowlist_yields_single_node_run() {
        // 方案 2 mechanism (what run_executor_driven does): profile.allowed ∩ {only}
        // → project → single-node DAG whose sole node is both entry and terminal
        // (the executor runs it once, then RunOutcome::Completed → stops).
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        let allowed: HashSet<StageKind> = p
            .allowed_stage_set()
            .intersection(&HashSet::from([StageKind::TargetIntel]))
            .copied()
            .collect();
        let dag = base().project(&allowed);
        assert_eq!(dag.nodes, vec![StageKind::TargetIntel]);
        assert!(dag.edges.is_empty());
        assert_eq!(dag.entry_points(), vec![StageKind::TargetIntel]);
        assert_eq!(dag.terminals(), vec![StageKind::TargetIntel]);
    }

    #[test]
    fn project_with_slice_allowlist_caps_terminal_at_to() {
        // slice scoping..=target_intel → project → scoping entry, target_intel
        // terminal (its downstream eas is dropped → run stops after intel).
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        let dag_full = base().project(&p.allowed_stage_set());
        let slice = dag_full.slice(None, StageKind::TargetIntel).unwrap();
        let allowed: HashSet<StageKind> = p
            .allowed_stage_set()
            .intersection(&slice)
            .copied()
            .collect();
        let dag = base().project(&allowed);
        assert_eq!(dag.entry_points(), vec![StageKind::Scoping]);
        assert_eq!(dag.terminals(), vec![StageKind::TargetIntel]);
        assert!(
            dag.next_stages(StageKind::TargetIntel).is_empty(),
            "to has no successors in the slice → run finishes intel then stops"
        );
    }
}
