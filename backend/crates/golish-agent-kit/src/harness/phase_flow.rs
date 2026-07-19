//! Phase-aware 流转（设计 2026-06-03 §6/§7）.
//!
//! 在 stage 级 gate 之上判定：
//!   - 「大阶段是否跑完」= 它的（投影后）成员小阶段是否都已 gate PASS（决策甲）；
//!   - 「该跨到哪个大阶段」= 按投影后 [`PhaseMap`] 顺序取下一个 phase；
//!   - 「跨入下一大阶段前要不要审批」= 下一 phase 的 `entry_approval`（天然 de-dup，
//!     每个 phase 至多一个入口审批 key）。
//!
//! 纯逻辑、不碰 DB。运行时（subtask loop / graph-flow 引擎）在 gate PASS 后调
//! [`decide_phase_step`] 决定是 stay-in-phase 继续派成员、还是跨 phase（可能先审批）、
//! 还是收尾。

use std::collections::HashSet;

use super::phase::{Phase, PhaseMap};
use super::profile::Profile;
use super::types::StageKind;

/// Verification may satisfy the vuln phase only from a non-empty exact DB
/// snapshot set. Deliverable findings/summary and process-local wave state are
/// intentionally absent from this contract.
pub fn verification_truth_is_complete(
    truth: Option<&super::attack_execution::VerificationTruthSet>,
) -> bool {
    truth.is_some_and(|truth| {
        super::attack_execution::validate_verification_truth_set(truth).is_ok()
    })
}

/// 给定一组「已 gate PASS 的 stage」，phase 是否完成 = 它的（投影后）成员全部 PASS.
///
/// 空成员的 phase 视为未完成（不应出现：投影会剔除空 phase）。
pub fn phase_is_complete(phase: &Phase, gate_passed: &HashSet<StageKind>) -> bool {
    !phase.stages.is_empty() && phase.stages.iter().all(|s| gate_passed.contains(s))
}

/// 当前 stage 所在 phase 之后的下一个 phase（按投影后 [`PhaseMap`] 顺序）.
///
/// None = 当前已是最后一个 phase（或 stage 不在任何 phase）。
pub fn next_phase(map: &PhaseMap, current: StageKind) -> Option<&Phase> {
    let idx = map.phases.iter().position(|p| p.contains(current))?;
    map.phases.get(idx + 1)
}

/// 跨入 `target_phase` 前需要的审批 key（None = 不需要 phase 级审批）.
pub fn phase_entry_approval(target_phase: &Phase) -> Option<&str> {
    target_phase.entry_approval.as_deref()
}

/// 当前 stage gate PASS 后、结合「已 PASS 集合」得到的 phase 级下一步.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseStep {
    /// 当前 phase 还没跑完（还有成员 stage 未 PASS）→ 留在本 phase 继续派成员 stage.
    StayInPhase,
    /// 当前 phase 跑完、有下一 phase → 跨入；`approval` = 跨入前需要的审批（None=直接进）.
    EnterPhase {
        phase_id: String,
        approval: Option<String>,
    },
    /// 当前 phase 跑完、无下一 phase → operation 完成.
    Complete,
}

/// 综合决策：当前 stage 的 gate 已 PASS 后，结合「已 PASS 集合」判断 phase 级下一步.
pub fn decide_phase_step(
    map: &PhaseMap,
    current: StageKind,
    gate_passed: &HashSet<StageKind>,
) -> PhaseStep {
    let Some(cur_phase) = map.phase_of(current) else {
        // 不在任何 phase（被投影剪掉）→ 防御性收尾，避免卡死游标.
        return PhaseStep::Complete;
    };
    if !phase_is_complete(cur_phase, gate_passed) {
        return PhaseStep::StayInPhase;
    }
    match next_phase(map, current) {
        None => PhaseStep::Complete,
        Some(next) => PhaseStep::EnterPhase {
            phase_id: next.id.clone(),
            approval: phase_entry_approval(next).map(|s| s.to_string()),
        },
    }
}

/// 运行时便捷：当前 stage gate PASS 后，跨 phase 是否需要先审批（返回审批 key）.
///
/// 仅当 [`decide_phase_step`] == [`PhaseStep::EnterPhase`] 且其 `approval` 为 Some 时
/// 返回 Some；StayInPhase / Complete / 无入口审批均返回 None。
pub fn pending_phase_approval(
    map: &PhaseMap,
    current: StageKind,
    gate_passed: &HashSet<StageKind>,
) -> Option<String> {
    match decide_phase_step(map, current, gate_passed) {
        PhaseStep::EnterPhase { approval, .. } => approval,
        _ => None,
    }
}

/// 线性-DAG 运行时用：给定「当前 stage」与「拓扑选出的下一 stage」，若这次推进
/// **跨大阶段**，返回目标 phase 的 `entry_approval`（None = 同阶段内推进 / 跨阶段但
/// 无入口审批 / phase 未知）。
///
/// 线性主干下「大阶段是否跑完」是隐式的：只有当某 phase 最后一个成员 stage 过了 gate，
/// 拓扑 `next` 才会落到下一个 phase 的成员上——故用 `phase_of(current) != phase_of(next)`
/// 检测跨界即可，无需另维护 gate-passed 集合。de-dup 天然成立：每个 phase 至多一个入口审批。
pub fn crossing_phase_approval(
    map: &PhaseMap,
    current: StageKind,
    next: StageKind,
) -> Option<String> {
    match (map.phase_of(current), map.phase_of(next)) {
        (Some(c), Some(n)) if c.id != n.id => n.entry_approval.clone(),
        _ => None,
    }
}

/// Metadata-level declaration that crossing into `next` carries a generic phase
/// approval key: cross-phase + target `entry_approval` + profile policy enabled.
///
/// 镜像 [`super::stage_transition::stage_entry_requires_approval`] 的 policy 闸语义
/// （`before_active_scan || before_scope_expansion` 任一为真即视为开），但锚在 **phase
/// 边界**而非 per-stage。TaskOrchestrator applies the stricter runtime policy:
/// this generic declaration can open routine HITL only for a Scoping-origin
/// crossing; post-Scoping transitions auto-advance after typed barriers.
pub fn phase_crossing_requires_approval(
    map: &PhaseMap,
    current: StageKind,
    next: StageKind,
    profile: &Profile,
) -> bool {
    if crossing_phase_approval(map, current, next).is_none() {
        return false;
    }
    profile
        .approval_policy
        .as_ref()
        .map(|p| p.before_active_scan || p.before_scope_expansion)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::resources::load_embedded_phase_map;

    fn passed(stages: &[StageKind]) -> HashSet<StageKind> {
        stages.iter().copied().collect()
    }

    #[test]
    fn stay_in_phase_until_all_members_pass() {
        let map = load_embedded_phase_map().unwrap();
        // prep = [scoping, target_intel]; 只过了 scoping → 留在 prep.
        let step = decide_phase_step(&map, StageKind::Scoping, &passed(&[StageKind::Scoping]));
        assert_eq!(step, PhaseStep::StayInPhase);
        assert_eq!(
            pending_phase_approval(&map, StageKind::Scoping, &passed(&[StageKind::Scoping])),
            None
        );
    }

    #[test]
    fn enter_active_recon_requires_active_scan_approval() {
        let map = load_embedded_phase_map().unwrap();
        // prep 全过 → 跨入 active_recon，需 active_scan 审批.
        let gp = passed(&[StageKind::Scoping, StageKind::TargetIntel]);
        let step = decide_phase_step(&map, StageKind::TargetIntel, &gp);
        assert_eq!(
            step,
            PhaseStep::EnterPhase {
                phase_id: "active_recon".to_string(),
                approval: Some("active_scan".to_string()),
            }
        );
        assert_eq!(
            pending_phase_approval(&map, StageKind::TargetIntel, &gp),
            Some("active_scan".to_string())
        );
    }

    #[test]
    fn enter_vuln_requires_exploit_validation_approval() {
        let map = load_embedded_phase_map().unwrap();
        let gp = passed(&[
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
        ]);
        let step = decide_phase_step(&map, StageKind::Enumeration, &gp);
        assert_eq!(
            step,
            PhaseStep::EnterPhase {
                phase_id: "vuln".to_string(),
                approval: Some("exploit_validation".to_string()),
            }
        );
    }

    #[test]
    fn enter_post_exploit_has_no_phase_entry_approval() {
        let map = load_embedded_phase_map().unwrap();
        // vuln 全过 → 跨入 post_exploit；post_exploit 无 entry_approval.
        // vuln phase = [vuln_triage, attack_candidate, verification]（设计 2026-07-02），
        // 三个成员全 PASS 才算大阶段跑完。
        let gp = passed(&[
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
            StageKind::Verification,
        ]);
        let step = decide_phase_step(&map, StageKind::Verification, &gp);
        assert_eq!(
            step,
            PhaseStep::EnterPhase {
                phase_id: "post_exploit".to_string(),
                approval: None,
            }
        );
    }

    #[test]
    fn last_phase_completes_only_when_all_members_pass() {
        let map = load_embedded_phase_map().unwrap();
        // closeout = [reporting, cleanup]. 两者全过 → Complete.
        let all = passed(&[StageKind::Reporting, StageKind::Cleanup]);
        assert_eq!(
            decide_phase_step(&map, StageKind::Cleanup, &all),
            PhaseStep::Complete
        );
        // reporting 过、cleanup 未过 → 仍 StayInPhase（红队收尾未清理完）.
        let only_reporting = passed(&[StageKind::Reporting]);
        assert_eq!(
            decide_phase_step(&map, StageKind::Reporting, &only_reporting),
            PhaseStep::StayInPhase
        );
    }

    #[test]
    fn crossing_phase_approval_only_fires_across_phase_boundaries() {
        let map = load_embedded_phase_map().unwrap();
        // 同 phase 内推进（prep: scoping→target_intel）→ None.
        assert_eq!(
            crossing_phase_approval(&map, StageKind::Scoping, StageKind::TargetIntel),
            None
        );
        // 跨界 prep→active_recon（target_intel→eas）→ active_scan.
        assert_eq!(
            crossing_phase_approval(
                &map,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface
            ),
            Some("active_scan".to_string())
        );
        // 同 phase active_recon 内（eas→enumeration）→ None.
        assert_eq!(
            crossing_phase_approval(
                &map,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration
            ),
            None
        );
        // 跨界 active_recon→vuln（enumeration→vuln_triage）→ exploit_validation.
        assert_eq!(
            crossing_phase_approval(&map, StageKind::Enumeration, StageKind::VulnTriage),
            Some("exploit_validation".to_string())
        );
        // 跨界 vuln→post_exploit（verification→access_validation）→ 无入口审批 None.
        assert_eq!(
            crossing_phase_approval(&map, StageKind::Verification, StageKind::AccessValidation),
            None
        );
    }

    #[test]
    fn phase_crossing_requires_approval_gates_on_profile_policy() {
        use crate::harness::resources::load_embedded_profile;
        let map = load_embedded_phase_map().unwrap();
        // pentest profile: approval_policy 打开 + 允许 vuln 阶段.
        let pentest = load_embedded_profile("pentest").unwrap().unwrap();
        // 跨界 active_recon→vuln + policy on → true.
        assert!(phase_crossing_requires_approval(
            &map,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            &pentest
        ));
        // 同 phase 内（eas→enumeration）→ 永远 false（无跨界）.
        assert!(!phase_crossing_requires_approval(
            &map,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            &pentest
        ));
        // 跨界但目标 phase 无 entry_approval（vuln→post_exploit）→ false.
        assert!(!phase_crossing_requires_approval(
            &map,
            StageKind::Verification,
            StageKind::AccessValidation,
            &pentest
        ));
    }
}
