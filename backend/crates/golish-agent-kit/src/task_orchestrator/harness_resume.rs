//! P1 · harness resume checkpoint, persisted to `operation_state.state_blob`.
//!
//! Minimal snapshot to resume a harness operation after a process kill: which
//! stage is current (+ its `stage_runs` id so the prior run can be marked
//! terminal on advance), the planned subtask titles to rebuild the queue, and
//! how many already completed. Full LLM history lives in `message_chains`, not
//! here — this is just the harness control state.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Server-owned sibling namespace that survives both legacy graph checkpoints
/// and V2-only legacy-namespace cleanup. It records only fresh typed-launch
/// authority; model/tool payloads never populate it.
pub const FRESH_LAUNCH_AUTHORITY_NAMESPACE: &str = "fresh_launch_authority";
const FRESH_LAUNCH_AUTHORITY_SCHEMA_V: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessResumeState {
    pub profile: String,
    pub current_stage: String,
    pub current_stage_run_id: Option<Uuid>,
    pub queue_titles: Vec<String>,
    pub completed_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity_adoption: Option<crate::harness::ContinuityAdoptionPlan>,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

fn default_schema_v() -> u32 {
    1
}

/// Merge the fresh invocation's target-authority bit without replacing any
/// graph/worker/producer sibling checkpoint.
pub fn state_blob_with_current_invocation_target_authority(
    mut existing: serde_json::Value,
    authority: bool,
) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    existing
        .as_object_mut()
        .expect("object normalized above")
        .insert(
            FRESH_LAUNCH_AUTHORITY_NAMESPACE.to_string(),
            serde_json::json!({
                "schema_v": FRESH_LAUNCH_AUTHORITY_SCHEMA_V,
                "current_invocation_target_authority": authority,
            }),
        );
    existing
}

/// Restore the fresh-only target authority for exact resume. Absence is the
/// backward-compatible GUI/unconfirmed path; a present but malformed marker is
/// rejected instead of being treated as unrestricted `None`.
pub fn current_invocation_target_authority_from_state_blob(
    state_blob: &serde_json::Value,
) -> anyhow::Result<Option<bool>> {
    let Some(marker) = state_blob.get(FRESH_LAUNCH_AUTHORITY_NAMESPACE) else {
        return Ok(None);
    };
    anyhow::ensure!(
        marker.get("schema_v").and_then(serde_json::Value::as_u64)
            == Some(FRESH_LAUNCH_AUTHORITY_SCHEMA_V),
        "fresh launch authority marker has an unsupported schema"
    );
    let authority = marker
        .get("current_invocation_target_authority")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            anyhow::anyhow!("fresh launch authority marker is missing its target bit")
        })?;
    Ok(Some(authority))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let run = Uuid::new_v4();
        let s = HarnessResumeState {
            profile: "assessment".into(),
            current_stage: "external_attack_surface".into(),
            current_stage_run_id: Some(run),
            queue_titles: vec!["recon".into(), "enumerate".into()],
            completed_count: 1,
            continuity_adoption: None,
            schema_v: 1,
        };
        let v = serde_json::to_value(&s).expect("to_value");
        let back: HarnessResumeState = serde_json::from_value(v).expect("from_value");
        assert_eq!(back.current_stage, "external_attack_surface");
        assert_eq!(back.completed_count, 1);
        assert_eq!(back.current_stage_run_id, Some(run));
        assert_eq!(
            back.queue_titles,
            vec!["recon".to_string(), "enumerate".to_string()]
        );
    }

    #[test]
    fn missing_schema_v_defaults_to_1() {
        // Forward-compat: a blob written before schema_v existed still loads.
        let v = serde_json::json!({
            "profile": "assessment",
            "current_stage": "scoping",
            "current_stage_run_id": null,
            "queue_titles": [],
            "completed_count": 0
        });
        let back: HarnessResumeState = serde_json::from_value(v).expect("from_value");
        assert_eq!(back.schema_v, 1);
    }

    #[test]
    fn continuity_adoption_is_optional_for_old_blobs() {
        let v = serde_json::json!({
            "profile": "assessment",
            "current_stage": "target_intel",
            "current_stage_run_id": null,
            "queue_titles": [],
            "completed_count": 0,
            "schema_v": 1
        });
        let back: HarnessResumeState = serde_json::from_value(v).expect("from_value");
        assert!(back.continuity_adoption.is_none());
    }

    #[test]
    fn fresh_target_authority_marker_round_trips_and_preserves_siblings() {
        let original = serde_json::json!({
            "profile": "red_team",
            "graph_flow": {"next_node": "target_intel"},
            "stage_run_workers": {"target_intel": {}},
        });
        let marked = state_blob_with_current_invocation_target_authority(original.clone(), false);

        assert_eq!(marked.get("profile"), original.get("profile"));
        assert_eq!(marked.get("graph_flow"), original.get("graph_flow"));
        assert_eq!(
            marked.get("stage_run_workers"),
            original.get("stage_run_workers")
        );
        assert_eq!(
            current_invocation_target_authority_from_state_blob(&marked)
                .expect("valid server marker"),
            Some(false)
        );
        assert_eq!(
            current_invocation_target_authority_from_state_blob(&original)
                .expect("old GUI/unconfirmed blob"),
            None
        );
    }

    #[test]
    fn malformed_fresh_target_authority_marker_fails_closed() {
        let malformed = serde_json::json!({
            (FRESH_LAUNCH_AUTHORITY_NAMESPACE): {
                "schema_v": 1,
                "current_invocation_target_authority": "false",
            }
        });
        assert!(current_invocation_target_authority_from_state_blob(&malformed).is_err());
    }
}
