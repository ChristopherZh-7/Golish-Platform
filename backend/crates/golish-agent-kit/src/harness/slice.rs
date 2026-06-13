//! Profile-projected stage-slice resolution, shared by every "run a slice of
//! the DAG" entry point: the headless CLI (`golish --stage-run`) and the
//! engagement worker sessions (chat task mode with a worker scope, 设计
//! 2026-06-13-engagement-scoping-fanout §6.3). Moved here from
//! `golish/src/stage_run/mod.rs` verbatim so both callers resolve slices
//! identically.

use std::collections::HashSet;

use super::operation_graph::base_operation_graph;
use super::resources::load_embedded_profile;
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
    let graph = base_operation_graph().map_err(|e| format!("load operation graph: {e}"))?;
    let profile = load_embedded_profile(profile_id)
        .map_err(|e| format!("load profile {profile_id}: {e}"))?
        .ok_or_else(|| format!("unknown harness profile: {profile_id}"))?;
    let allowed = profile.allowed_stage_set();
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
}
