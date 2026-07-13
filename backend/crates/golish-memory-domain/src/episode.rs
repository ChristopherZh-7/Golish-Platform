use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scope::OperationScope;
use crate::source_ref::SourceRef;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeVerdict {
    Passed,
    Blocked,
    Exhausted,
    Failed,
    Superseded,
}

impl EpisodeVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Blocked => "blocked",
            Self::Exhausted => "exhausted",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StageEpisode {
    pub episode_id: Uuid,
    pub scope: OperationScope,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub candidate_attempt_id: Option<Uuid>,
    pub stage_kind: String,
    pub wave: Option<i32>,
    pub verdict: EpisodeVerdict,
    pub deliverable_submission_id: Option<Uuid>,
    pub handoff_id: Option<Uuid>,
    pub reason_codes: Vec<String>,
    pub fact_refs: Vec<SourceRef>,
    pub evidence_ids: Vec<i64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

impl StageEpisode {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.scope.validate().map_err(|_| "memory_episode_scope")?;
        if self.stage_kind.trim().is_empty() {
            return Err("memory_episode_stage_kind_empty");
        }
        if self.ended_at < self.started_at {
            return Err("memory_episode_window_invalid");
        }
        if self.evidence_ids.iter().any(|id| *id <= 0) {
            return Err("memory_episode_evidence_invalid");
        }
        if self
            .fact_refs
            .iter()
            .any(|source| source.validate().is_err())
        {
            return Err("memory_episode_fact_ref_invalid");
        }
        Ok(())
    }
}
