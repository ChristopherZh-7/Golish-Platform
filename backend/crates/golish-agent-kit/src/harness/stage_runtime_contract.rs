//! Declarative Runtime Memory ownership contract embedded in a [`StageSpec`].
//!
//! This module describes which durable identities own a specialist stage. It
//! does not select the deployment rollout and cannot weaken database fencing;
//! the operation-frozen runtime contract and compound repository remain the
//! execution authority.

use serde::{Deserialize, Serialize};

/// Closed V2 runtime contract for one stage specification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageRuntimeContract {
    pub schema_version: u16,
    pub unit_identity: RuntimeUnitIdentity,
    pub scope_source: RuntimeScopeSource,
    pub requires_worker_lease: bool,
    pub publishes_handoff_after_final_seal: bool,
}

/// A runtime Unit is uniquely owned by one stage execution and organization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUnitIdentity {
    StageExecutionOrganization,
}

/// Specialist fan-out consumes only the immutable operation scope snapshot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeScopeSource {
    FrozenOperationSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_v2_contract_json_round_trips() {
        let raw = r#"{
            "schema_version": 2,
            "unit_identity": "stage_execution_organization",
            "scope_source": "frozen_operation_snapshot",
            "requires_worker_lease": true,
            "publishes_handoff_after_final_seal": true
        }"#;
        let contract: StageRuntimeContract = serde_json::from_str(raw).expect("parse contract");
        assert_eq!(contract.schema_version, 2);
        assert_eq!(
            contract.unit_identity,
            RuntimeUnitIdentity::StageExecutionOrganization
        );
        assert_eq!(
            contract.scope_source,
            RuntimeScopeSource::FrozenOperationSnapshot
        );
        assert!(contract.requires_worker_lease);
        assert!(contract.publishes_handoff_after_final_seal);
    }
}
