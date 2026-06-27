//! DB-backed continuity preflight for cross-session operation adoption.
//!
//! This layer reads only durable, deterministic tables through `DbRepoProvider`
//! and converts them into the IO-free harness continuity snapshot. It never
//! writes adoption state; the orchestrator records the chosen plan only after
//! the user explicitly chooses reuse.

use std::collections::HashSet;

use anyhow::Context;
use chrono::Utc;
use uuid::Uuid;

use crate::db_traits::DbRepoProvider;
use crate::harness::org_gate::{completion_is_fresh, STAGE_COMPLETION_TTL_SECS};
use crate::harness::{
    base_operation_graph, plan_adoption_cursor, ContinuityAdoptionPlan, ContinuitySnapshot,
    StageKind, StageReuseStatus, StageReuseSummary,
};

/// Build a continuity adoption plan from durable DB state.
///
/// `engagement_root` is optional because a new chat session may not have a
/// bound operation yet. When absent, the snapshot may read legacy in-scope
/// orgs to decide whether later stage facts exist, but it must not adopt
/// `scoping`: skipping scope without a root would make downstream gates use
/// the whole embedded DB instead of the current engagement subtree.
pub async fn build_existing_db_continuity_plan(
    repo: &dyn DbRepoProvider,
    profile_id: &str,
    engagement_root: Option<Uuid>,
) -> anyhow::Result<Option<ContinuityAdoptionPlan>> {
    let profile = match crate::harness::load_embedded_profile(profile_id)
        .with_context(|| format!("load profile {profile_id}"))?
    {
        Some(profile) => profile,
        None => crate::harness::load_embedded_profile("assessment")?
            .context("load fallback assessment profile")?,
    };
    let graph = base_operation_graph().context("load operation graph")?;
    let dag = graph.project(&profile.allowed_stage_set());

    let legacy_orgs = repo
        .in_scope_org_ids(None)
        .await
        .context("read in-scope organizations for continuity preflight")?;
    let org_ids = if let Some(root) = engagement_root {
        repo.org_subtree_ids(root)
            .await
            .context("read engagement org subtree for continuity preflight")?
    } else {
        legacy_orgs
    };
    if org_ids.is_empty() {
        return Ok(None);
    }

    let mut stages = Vec::new();
    if dag.contains(StageKind::Scoping) {
        stages.push(scoping_summary(org_ids.len(), engagement_root));
    }

    for stage in [
        StageKind::TargetIntel,
        StageKind::ExternalAttackSurface,
        StageKind::Enumeration,
    ] {
        if !dag.contains(stage) {
            continue;
        }
        let rows = repo
            .org_stage_completions_get(stage.as_str(), &org_ids)
            .await
            .with_context(|| format!("read {} completion ledger", stage.as_str()))?;
        stages.push(stage_summary_from_completion_rows(stage, &org_ids, rows));
    }

    let snapshot = ContinuitySnapshot {
        scope_units: org_ids.len(),
        stages,
    };
    Ok(non_empty_adoption_cursor(&dag, snapshot))
}

fn non_empty_adoption_cursor(
    dag: &crate::harness::operation_graph::AllowedDag,
    snapshot: ContinuitySnapshot,
) -> Option<ContinuityAdoptionPlan> {
    plan_adoption_cursor(dag, snapshot).filter(|plan| !plan.adopted_stages.is_empty())
}

fn scoping_summary(org_count: usize, engagement_root: Option<Uuid>) -> StageReuseSummary {
    let (status, detail) = if engagement_root.is_some() {
        (
            StageReuseStatus::Reusable,
            format!(
                "found {org_count} in-scope organization(s) in the engagement subtree; user confirmation is required before adopting scope"
            ),
        )
    } else {
        (
            StageReuseStatus::Missing,
            format!(
                "found {org_count} legacy in-scope organization(s), but no engagement root is bound; scoping must run to bind the current task before reuse"
            ),
        )
    };

    StageReuseSummary {
        stage: StageKind::Scoping,
        status,
        detail,
    }
}

fn stage_summary_from_completion_rows(
    stage: StageKind,
    org_ids: &[Uuid],
    rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>)>,
) -> StageReuseSummary {
    let now = Utc::now();
    let expected: HashSet<Uuid> = org_ids.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut fresh = HashSet::new();
    for (org_id, passed_at) in rows {
        if !expected.contains(&org_id) {
            continue;
        }
        seen.insert(org_id);
        if completion_is_fresh(passed_at, now, STAGE_COMPLETION_TTL_SECS) {
            fresh.insert(org_id);
        }
    }

    let total = expected.len();
    let seen_count = seen.len();
    let fresh_count = fresh.len();
    let status = if total > 0 && fresh_count == total {
        StageReuseStatus::Reusable
    } else if seen_count == 0 {
        StageReuseStatus::Missing
    } else if seen_count == total && fresh_count == 0 {
        StageReuseStatus::Stale
    } else {
        StageReuseStatus::Partial
    };

    StageReuseSummary {
        stage,
        status,
        detail: format!(
            "{} completion ledger: {fresh_count}/{total} fresh, {seen_count}/{total} present",
            stage.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn completion_rows_all_fresh_are_reusable() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let summary = stage_summary_from_completion_rows(
            StageKind::TargetIntel,
            &[a, b],
            vec![(a, Utc::now()), (b, Utc::now())],
        );

        assert_eq!(summary.status, StageReuseStatus::Reusable);
        assert!(summary.detail.contains("2/2 fresh"));
    }

    #[test]
    fn completion_rows_distinguish_partial_and_stale() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let partial = stage_summary_from_completion_rows(
            StageKind::Enumeration,
            &[a, b],
            vec![(a, Utc::now())],
        );
        assert_eq!(partial.status, StageReuseStatus::Partial);

        let stale = stage_summary_from_completion_rows(
            StageKind::Enumeration,
            &[a, b],
            vec![
                (a, Utc::now() - Duration::days(30)),
                (b, Utc::now() - Duration::days(30)),
            ],
        );
        assert_eq!(stale.status, StageReuseStatus::Stale);
    }

    #[test]
    fn scoping_requires_bound_engagement_root_before_adoption() {
        let summary = scoping_summary(23, None);

        assert_eq!(summary.status, StageReuseStatus::Missing);
        assert!(summary.detail.contains("no engagement root is bound"));
    }

    #[test]
    fn scoping_can_be_reused_with_bound_engagement_root() {
        let summary = scoping_summary(13, Some(Uuid::new_v4()));

        assert_eq!(summary.status, StageReuseStatus::Reusable);
        assert!(summary.detail.contains("engagement subtree"));
    }

    #[test]
    fn missing_scoping_root_does_not_offer_empty_adoption_plan() {
        let graph = base_operation_graph().expect("operation graph");
        let profile = crate::harness::load_embedded_profile("assessment")
            .expect("load profile")
            .expect("assessment profile");
        let dag = graph.project(&profile.allowed_stage_set());
        let snapshot = ContinuitySnapshot {
            scope_units: 23,
            stages: vec![
                scoping_summary(23, None),
                StageReuseSummary {
                    stage: StageKind::TargetIntel,
                    status: StageReuseStatus::Reusable,
                    detail: "target_intel completion ledger: 23/23 fresh, 23/23 present"
                        .to_string(),
                },
            ],
        };

        assert!(non_empty_adoption_cursor(&dag, snapshot).is_none());
    }
}
