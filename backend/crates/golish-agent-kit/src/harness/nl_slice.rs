//! NlSlice · 终态 4 字段 (Doc 3 §6).
//!
//! **禁止继续加字段** (§14.1 + §18 警告). 如需更多状态, 抽 SubtaskContext 新结构,
//! 不在 NlSlice 上扩.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::types::StageKind;

/// Doc 3 §6 NlSlice 终态.
///
/// 4 个字段:
///   - subtask_id            : 唯一 subtask 标识 (uuid v4)
///   - stage_kind            : 子任务归属哪个 stage
///   - sealed_origin_session : evidence 来源 session 锁定 (Doc 3 §10.2 重 classifier 用)
///   - deliverable_schema_id : 期望提交的 deliverable schema id
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NlSlice {
    pub subtask_id: Uuid,
    pub stage_kind: StageKind,
    pub sealed_origin_session: String,
    pub deliverable_schema_id: String,
}

impl NlSlice {
    pub fn new(
        subtask_id: Uuid,
        stage_kind: StageKind,
        sealed_origin_session: impl Into<String>,
        deliverable_schema_id: impl Into<String>,
    ) -> Self {
        Self {
            subtask_id,
            stage_kind,
            sealed_origin_session: sealed_origin_session.into(),
            deliverable_schema_id: deliverable_schema_id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nl_slice_round_trip_serde() {
        let s = NlSlice::new(
            Uuid::new_v4(),
            StageKind::ExternalAttackSurface,
            "session-abc",
            "ExternalAttackSurfaceDeliverable",
        );
        let json = serde_json::to_string(&s).unwrap();
        let back: NlSlice = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn nl_slice_constructor_assigns_fields() {
        let id = Uuid::new_v4();
        let s = NlSlice::new(id, StageKind::TargetIntel, "sess", "TargetIntelDeliverable");
        assert_eq!(s.subtask_id, id);
        assert_eq!(s.stage_kind, StageKind::TargetIntel);
        assert_eq!(s.sealed_origin_session, "sess");
        assert_eq!(s.deliverable_schema_id, "TargetIntelDeliverable");
    }
}
