//! P1 · harness resume checkpoint, persisted to `operation_state.state_blob`.
//!
//! Minimal snapshot to resume a harness operation after a process kill: which
//! stage is current (+ its `stage_runs` id so the prior run can be marked
//! terminal on advance), the planned subtask titles to rebuild the queue, and
//! how many already completed. Full LLM history lives in `message_chains`, not
//! here — this is just the harness control state.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessResumeState {
    pub profile: String,
    pub current_stage: String,
    pub current_stage_run_id: Option<Uuid>,
    pub queue_titles: Vec<String>,
    pub completed_count: usize,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

fn default_schema_v() -> u32 {
    1
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
}
