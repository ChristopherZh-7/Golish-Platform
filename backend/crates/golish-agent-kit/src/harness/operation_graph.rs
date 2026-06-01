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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::profile::load_profile_from_json;

    const BASE_GRAPH_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn base() -> OperationGraph {
        load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph parses + validates")
    }

    fn assessment_dag() -> AllowedDag {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        base().project(&p.allowed_stage_set())
    }

    #[test]
    fn base_graph_has_12_nodes_13_edges() {
        let g = base();
        assert_eq!(g.nodes.len(), 12);
        assert_eq!(g.edges.len(), 13);
    }

    #[test]
    fn base_graph_loads_via_include() {
        // 内置加载与显式 from_json 等价.
        let g = base_operation_graph().expect("built-in base graph");
        assert_eq!(g.nodes.len(), 12);
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
}
