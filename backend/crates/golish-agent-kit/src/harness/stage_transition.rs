//! Stage transition driver · gate 结果 → 下一 stage 决策 (Doc 3 §6.2 step 9-10 / §6.3).
//!
//! Phase 2: 在 [`super::operation_graph`] (拓扑) 之上加"流转决策"——把 gate 是否通过
//! + 当前 stage + 投影后的 [`AllowedDag`] 合成一个**确定性**的下一步决定.
//!
//! 纯函数, 不碰 DB: 真正的游标推进 (`operation_state::advance_stage`) 由调用方按
//! 本决策执行. 审批 (`Profile::approval_policy`) 与每 tool call 授权
//! (`pre_action_authorizer`) 是 **action 时**的关注点, 不在 stage 流转里判定;
//! 故本驱动只回答"gate 之后该去哪", 不回答"这步要不要人工点头".

use super::gate::GateResult;
use super::operation_graph::AllowedDag;
use super::types::StageKind;

/// 一次 stage gate 结束后的流转决定.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDecision {
    /// gate 没过 → 留在当前 stage 返工 (Doc 3 §6.3 blocked → repair subtasks).
    Hold,
    /// gate 过, 当前 stage 无下一格 (终点) → operation 完成.
    Complete,
    /// gate 过, 唯一下一格 → 直接推进到该 stage.
    Advance(StageKind),
    /// gate 过, 多个下一格 → 需要调用方/agent 选一个 (顺序同图中边声明顺序).
    Branch(Vec<StageKind>),
}

impl TransitionDecision {
    /// 若本决定意味着"把游标推进到某个 stage", 返回该 stage; 否则 None.
    ///
    /// - `Advance(s)`  → `Some(s)`
    /// - `Branch(c)`   → `Some(c[0])` (Phase 1 默认取第一候选; 后续由 agent / 策略选)
    /// - `Hold` / `Complete` → `None` (不推进游标)
    pub fn advance_target(&self) -> Option<StageKind> {
        match self {
            TransitionDecision::Advance(s) => Some(*s),
            TransitionDecision::Branch(candidates) => candidates.first().copied(),
            TransitionDecision::Hold | TransitionDecision::Complete => None,
        }
    }
}

/// 核心决策: 当前 stage + gate 是否通过 + 可达子图 → 下一步.
///
/// - `gate_allowed == false` → [`TransitionDecision::Hold`]
/// - gate 过 + 0 个后继 → [`TransitionDecision::Complete`]
/// - gate 过 + 1 个后继 → [`TransitionDecision::Advance`]
/// - gate 过 + N(>1) 个后继 → [`TransitionDecision::Branch`]
pub fn decide_transition(
    current: StageKind,
    gate_allowed: bool,
    dag: &AllowedDag,
) -> TransitionDecision {
    if !gate_allowed {
        return TransitionDecision::Hold;
    }
    let mut next = dag.next_stages(current);
    match next.len() {
        0 => TransitionDecision::Complete,
        1 => TransitionDecision::Advance(next.remove(0)),
        _ => TransitionDecision::Branch(next),
    }
}

/// 便捷封装: 直接吃 [`GateResult`] (读其 `allowed` 字段).
///
/// 末端 hook 拿到 gate decision 后调本函数即可得到流转决定.
pub fn decide_from_gate(
    current: StageKind,
    gate: &GateResult,
    dag: &AllowedDag,
) -> TransitionDecision {
    decide_transition(current, gate.allowed, dag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::operation_graph::load_operation_graph_from_json;
    use crate::harness::profile::load_profile_from_json;

    const BASE_GRAPH_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn assessment_dag() -> AllowedDag {
        let g = load_operation_graph_from_json(BASE_GRAPH_JSON).expect("graph");
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("profile");
        g.project(&p.allowed_stage_set())
    }

    #[test]
    fn gate_blocked_holds_regardless_of_topology() {
        let dag = assessment_dag();
        // 即便当前 stage 有后继, gate 没过也必须 Hold.
        assert_eq!(
            decide_transition(StageKind::Scoping, false, &dag),
            TransitionDecision::Hold
        );
        assert_eq!(
            decide_transition(StageKind::ExternalAttackSurface, false, &dag),
            TransitionDecision::Hold
        );
    }

    #[test]
    fn gate_passed_single_successor_advances() {
        let dag = assessment_dag();
        assert_eq!(
            decide_transition(StageKind::Scoping, true, &dag),
            TransitionDecision::Advance(StageKind::TargetIntel)
        );
        assert_eq!(
            decide_transition(StageKind::TargetIntel, true, &dag),
            TransitionDecision::Advance(StageKind::ExternalAttackSurface)
        );
        assert_eq!(
            decide_transition(StageKind::Enumeration, true, &dag),
            TransitionDecision::Advance(StageKind::Reporting)
        );
    }

    #[test]
    fn gate_passed_multiple_successors_branches() {
        let dag = assessment_dag();
        // external_attack_surface → {enumeration, reporting}
        assert_eq!(
            decide_transition(StageKind::ExternalAttackSurface, true, &dag),
            TransitionDecision::Branch(vec![StageKind::Enumeration, StageKind::Reporting])
        );
    }

    #[test]
    fn gate_passed_terminal_completes() {
        let dag = assessment_dag();
        assert_eq!(
            decide_transition(StageKind::Reporting, true, &dag),
            TransitionDecision::Complete
        );
    }

    #[test]
    fn stage_outside_dag_completes_when_gate_passed() {
        let dag = assessment_dag();
        // VulnTriage 被投影剪掉, 无后继 → 视为 Complete (调用方不应到这, 防御性).
        assert_eq!(
            decide_transition(StageKind::VulnTriage, true, &dag),
            TransitionDecision::Complete
        );
    }

    #[test]
    fn advance_target_picks_next_or_first_branch_else_none() {
        let dag = assessment_dag();
        // Advance → 该 stage
        assert_eq!(
            decide_transition(StageKind::Scoping, true, &dag).advance_target(),
            Some(StageKind::TargetIntel)
        );
        // Branch → 第一候选 (enumeration before reporting)
        assert_eq!(
            decide_transition(StageKind::ExternalAttackSurface, true, &dag).advance_target(),
            Some(StageKind::Enumeration)
        );
        // Hold / Complete → None
        assert_eq!(
            decide_transition(StageKind::Scoping, false, &dag).advance_target(),
            None
        );
        assert_eq!(
            decide_transition(StageKind::Reporting, true, &dag).advance_target(),
            None
        );
    }

    #[test]
    fn decide_from_gate_mirrors_allowed_flag() {
        let dag = assessment_dag();
        let passed = GateResult::pass();
        let blocked = GateResult::block(
            vec!["missing evidence".to_string()],
            crate::harness::types::HarnessRecoveryActions::default(),
        );
        assert_eq!(
            decide_from_gate(StageKind::Scoping, &passed, &dag),
            TransitionDecision::Advance(StageKind::TargetIntel)
        );
        assert_eq!(
            decide_from_gate(StageKind::Scoping, &blocked, &dag),
            TransitionDecision::Hold
        );
    }
}
