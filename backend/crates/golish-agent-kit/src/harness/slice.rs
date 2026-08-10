//! Profile-projected stage-slice resolution, shared by every "run a slice of
//! the DAG" entry point: the headless CLI (`golish --stage-run`) and the
//! engagement worker sessions (chat task mode with a worker scope, 设计
//! 2026-06-13-engagement-scoping-fanout §6.3). Moved here from
//! `golish/src/stage_run/mod.rs` verbatim so both callers resolve slices
//! identically.

use std::collections::HashSet;

use super::operation_graph::operation_graph_for_topology;
use super::resources::load_embedded_profile;
use super::stage_topology_contract::StageTopologyContract;
use super::types::StageKind;

/// Resolve the `(entry_stage, allowlist)` for a stage slice against the
/// profile-projected DAG. `entry_stage` is the slice's entry point (where the
/// `operation_state` cursor begins); `allowlist` is fed to
/// `TaskOrchestrator::set_stage_allowlist`.
///
/// Errors are plain strings (callers wrap into their own error types).
pub fn resolve_slice(
    profile_id: &str,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>), String> {
    resolve_slice_for_topology(
        profile_id,
        StageTopologyContract::LegacyCandidateVerificationV1,
        from,
        to,
    )
}

/// Resolve a slice against one exact operation-frozen topology.
///
/// Existing-operation paths (resume/fork/executor) must use this function and
/// pass the validated persisted contract. The legacy [`resolve_slice`] wrapper
/// remains only for compatibility callers that are explicitly legacy-only.
pub fn resolve_slice_for_topology(
    profile_id: &str,
    topology: StageTopologyContract,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>), String> {
    let graph = operation_graph_for_topology(topology)
        .map_err(|e| format!("load operation graph for {topology}: {e}"))?;
    let profile = load_embedded_profile(profile_id)
        .map_err(|e| format!("load profile {profile_id}: {e}"))?
        .ok_or_else(|| format!("unknown harness profile: {profile_id}"))?;
    let allowed = profile
        .allowed_stage_set_for_topology(topology)
        .map_err(|e| format!("project profile {profile_id} for {topology}: {e}"))?;
    let dag = graph.project(&allowed);
    let allowlist = dag
        .slice(from, to)
        .map_err(|e| format!("stage slice ({profile_id}): {e}"))?;
    // Entry = the sliced sub-DAG's entry point (the cursor start).
    let sliced_allowed: HashSet<StageKind> = allowed.intersection(&allowlist).copied().collect();
    let entry = graph
        .project(&sliced_allowed)
        .entry_points()
        .into_iter()
        .next()
        .ok_or_else(|| "sliced DAG has no entry point".to_string())?;
    Ok((entry, allowlist))
}

/// Resolve the pre-create CLI slice without consulting a mutable rollout
/// default. The DB transaction will freeze exactly one topology; this helper
/// returns the union of every closed topology projection that accepts the same
/// requested boundary. The executor immediately reloads the frozen operation
/// and intersects this superset with its one exact graph.
pub fn resolve_slice_for_any_topology(
    profile_id: &str,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>), String> {
    let mut resolved = Vec::new();
    let mut errors = Vec::new();
    for topology in StageTopologyContract::ALL {
        match resolve_slice_for_topology(profile_id, topology, from, to) {
            Ok(slice) => resolved.push((topology, slice)),
            Err(error) => errors.push(format!("{topology}: {error}")),
        }
    }
    let Some((_, (entry, mut allowlist))) = resolved.pop() else {
        return Err(format!(
            "stage slice is invalid for every closed topology: {}",
            errors.join("; ")
        ));
    };
    for (topology, (other_entry, other_allowlist)) in resolved {
        if other_entry != entry {
            return Err(format!(
                "stage slice entry disagrees across closed topologies: {entry} vs {other_entry} ({topology})"
            ));
        }
        allowlist.extend(other_allowlist);
    }
    Ok((entry, allowlist))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--only target_intel` 等价切片：entry 与 allowlist 都是单一阶段。
    #[test]
    fn single_stage_slice_resolves_entry_and_allowlist() {
        let (entry, allowlist) = resolve_slice(
            "red_team",
            Some(StageKind::TargetIntel),
            StageKind::TargetIntel,
        )
        .expect("slice resolves");
        assert_eq!(entry, StageKind::TargetIntel);
        assert_eq!(allowlist, HashSet::from([StageKind::TargetIntel]));
    }

    /// engagement recon 家族工人切片：target_intel..=enumeration（设计 2026-06-13）。
    #[test]
    fn recon_family_slice_spans_intel_to_enumeration() {
        let (entry, allowlist) = resolve_slice(
            "red_team",
            Some(StageKind::TargetIntel),
            StageKind::Enumeration,
        )
        .expect("slice resolves");
        assert_eq!(entry, StageKind::TargetIntel);
        assert!(allowlist.contains(&StageKind::TargetIntel));
        assert!(allowlist.contains(&StageKind::Enumeration));
        assert!(
            !allowlist.contains(&StageKind::Scoping),
            "slice must not reach back to scoping"
        );
    }

    #[test]
    fn unknown_profile_is_an_error() {
        let err = resolve_slice("no-such-profile", None, StageKind::TargetIntel)
            .expect_err("unknown profile rejected");
        assert!(err.contains("unknown harness profile"));
    }

    #[test]
    fn unified_full_slice_replaces_the_complete_legacy_pair() {
        let (entry, allowlist) = resolve_slice_for_topology(
            "red_team",
            StageTopologyContract::UnifiedInvestigationV1,
            None,
            StageKind::Reporting,
        )
        .expect("unified slice resolves");
        assert_eq!(entry, StageKind::Scoping);
        assert!(allowlist.contains(&StageKind::ApplicationUnderstanding));
        assert!(allowlist.contains(&StageKind::Investigation));
        assert!(!allowlist.contains(&StageKind::AttackCandidate));
        assert!(!allowlist.contains(&StageKind::Verification));
    }

    #[test]
    fn pre_create_slice_is_topology_neutral_but_each_exact_slice_is_not() {
        let (_, allowlist) = resolve_slice_for_any_topology("red_team", None, StageKind::Reporting)
            .expect("pre-create slice resolves against the closed catalog");
        for stage in [
            StageKind::AttackCandidate,
            StageKind::Verification,
            StageKind::ApplicationUnderstanding,
            StageKind::Investigation,
        ] {
            assert!(allowlist.contains(&stage), "missing {stage}");
        }

        assert!(resolve_slice_for_topology(
            "red_team",
            StageTopologyContract::LegacyCandidateVerificationV1,
            Some(StageKind::Investigation),
            StageKind::Investigation,
        )
        .is_err());
        assert!(resolve_slice_for_topology(
            "red_team",
            StageTopologyContract::UnifiedInvestigationV1,
            Some(StageKind::AttackCandidate),
            StageKind::AttackCandidate,
        )
        .is_err());
    }
}
