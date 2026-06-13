//! Shared per-org stage fan-out helpers.
//!
//! The CLI `golish --stage-run --include-subsidiaries` (see
//! `backend/crates/golish/src/stage_run/mod.rs`) and the chat `stage_run` agent
//! tool both need to take an engagement org tree and run one stage **once per
//! org** (parent + direct children), each org isolated and gated on its own
//! (design `docs/design/2026-06-13-stage-run-fanout-design.md`, D1/D9). These
//! pure helpers are the shared core so both surfaces build the per-org units
//! identically (no behaviour drift between CLI and chat).

use golish_db::models::Organization;

use crate::harness::StageKind;

/// Keep only the direct children of `parent` (the scoping-built org tree).
///
/// Pure filter so per-subsidiary dispatch is unit-testable; the input list is
/// already project-scoped by `organizations::list` (AGENTS.md I2 — IDOR).
pub fn filter_child_orgs(orgs: Vec<Organization>, parent: uuid::Uuid) -> Vec<Organization> {
    orgs.into_iter()
        .filter(|o| o.parent_id == Some(parent))
        .collect()
}

/// Objective for one subsidiary's stage run. Carries the child's REAL
/// `organization_id` (so the agent calls `recon_*` / `manage_targets` against it
/// without guessing) and pins the collection scope to THIS subsidiary only.
pub fn build_child_objective(child: &Organization, parent_name: &str, to: StageKind) -> String {
    format!(
        "Run the {} stage for this engagement. Organization: {} (organization_id: {}). \
         This organization is a subsidiary of {} (already landed in the org tree during \
         scoping); collect for THIS subsidiary only — discover its own assets (domains, \
         IPs) and register them as in-scope targets bound to this organization_id.",
        to.as_str(),
        child.name,
        child.id,
        parent_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal Organization for pure-function tests (serde fills the
    /// `#[serde(default)]` profile fields).
    fn test_org(name: &str, parent_id: Option<uuid::Uuid>) -> Organization {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(),
            "project_path": "/tmp/p",
            "name": name,
            "parent_id": parent_id,
            "description": "",
            "owner": "",
            "sort_order": 0,
            "created_at": "2026-06-12T00:00:00Z",
            "updated_at": "2026-06-12T00:00:00Z",
        }))
        .expect("test org deserializes")
    }

    #[test]
    fn filter_child_orgs_keeps_direct_children_only() {
        let parent = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        let orgs = vec![
            test_org("root", None),
            test_org("child-a", Some(parent)),
            test_org("other-child", Some(other)),
            test_org("child-b", Some(parent)),
        ];
        let children = filter_child_orgs(orgs, parent);
        let names: Vec<&str> = children.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["child-a", "child-b"]);
    }

    #[test]
    fn child_objective_names_child_id_and_parent() {
        let child = test_org("默安子公司", Some(uuid::Uuid::new_v4()));
        let obj = build_child_objective(&child, "默安科技", StageKind::TargetIntel);
        assert!(obj.contains("Run the target_intel stage"));
        assert!(obj.contains(&format!("organization_id: {}", child.id)));
        assert!(obj.contains("默安子公司"));
        assert!(obj.contains("subsidiary of 默安科技"));
        assert!(obj.contains("THIS subsidiary only"));
    }
}
