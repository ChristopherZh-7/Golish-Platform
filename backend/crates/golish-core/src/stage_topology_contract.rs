use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::InvestigationRolloutMode;

const CONTRACT_VERSION: &str = "stage_topology.v1";
const HASH_DOMAIN: &[u8] = b"golish.stage_topology_contract.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageTopologyContract {
    LegacyCandidateVerificationV1,
    UnifiedInvestigationV1,
}

impl StageTopologyContract {
    pub const ALL: [Self; 2] = [
        Self::LegacyCandidateVerificationV1,
        Self::UnifiedInvestigationV1,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyCandidateVerificationV1 => "legacy_candidate_verification_v1",
            Self::UnifiedInvestigationV1 => "unified_investigation_v1",
        }
    }

    pub fn try_parse(value: &str) -> Result<Self, StageTopologyContractError> {
        match value {
            "legacy_candidate_verification_v1" => Ok(Self::LegacyCandidateVerificationV1),
            "unified_investigation_v1" => Ok(Self::UnifiedInvestigationV1),
            _ => Err(StageTopologyContractError::UnknownTopology(
                value.to_owned(),
            )),
        }
    }

    pub const fn graph_resource(self) -> &'static str {
        match self {
            Self::LegacyCandidateVerificationV1 => "operation_graph.json",
            Self::UnifiedInvestigationV1 => "operation_graph_unified_investigation_v1.json",
        }
    }

    pub const fn for_investigation_rollout(mode: InvestigationRolloutMode) -> Self {
        match mode {
            InvestigationRolloutMode::LegacyOnly
            | InvestigationRolloutMode::ShadowRegistry
            | InvestigationRolloutMode::DualReadCompare => Self::LegacyCandidateVerificationV1,
            InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection
            | InvestigationRolloutMode::NewOnly => Self::UnifiedInvestigationV1,
        }
    }

    pub const fn allows_investigation_rollout(self, mode: InvestigationRolloutMode) -> bool {
        matches!(
            (self, mode),
            (
                Self::LegacyCandidateVerificationV1,
                InvestigationRolloutMode::LegacyOnly
                    | InvestigationRolloutMode::ShadowRegistry
                    | InvestigationRolloutMode::DualReadCompare
            ) | (
                Self::UnifiedInvestigationV1,
                InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection
                    | InvestigationRolloutMode::NewOnly
            )
        )
    }

    pub fn canonical_json(self) -> String {
        format!(
            "{{\"contract_version\":\"{CONTRACT_VERSION}\",\"graph_resource\":\"{}\",\"topology\":\"{}\"}}",
            self.graph_resource(),
            self.as_str()
        )
    }

    pub fn contract_sha256(self) -> String {
        let mut digest = Sha256::new();
        digest.update(HASH_DOMAIN);
        digest.update(self.canonical_json().as_bytes());
        format!(
            "sha256:{}",
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }

    pub fn freeze_material(self) -> FrozenStageTopologyContractMaterial {
        FrozenStageTopologyContractMaterial {
            topology: self,
            canonical_json: self.canonical_json(),
            sha256: self.contract_sha256(),
        }
    }
}

impl std::fmt::Display for StageTopologyContract {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageTopologyFreezeSource {
    LegacyBackfillV1,
    DeploymentPairV1,
}

impl StageTopologyFreezeSource {
    pub const ALL: [Self; 2] = [Self::LegacyBackfillV1, Self::DeploymentPairV1];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyBackfillV1 => "legacy_backfill_v1",
            Self::DeploymentPairV1 => "deployment_pair_v1",
        }
    }

    pub fn try_parse(value: &str) -> Result<Self, StageTopologyContractError> {
        match value {
            "legacy_backfill_v1" => Ok(Self::LegacyBackfillV1),
            "deployment_pair_v1" => Ok(Self::DeploymentPairV1),
            _ => Err(StageTopologyContractError::UnknownFreezeSource(
                value.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenStageTopologyContractMaterial {
    pub topology: StageTopologyContract,
    pub canonical_json: String,
    pub sha256: String,
}

impl FrozenStageTopologyContractMaterial {
    pub fn validate(&self) -> Result<(), StageTopologyContractError> {
        let expected = self.topology.freeze_material();
        if *self == expected {
            Ok(())
        } else {
            Err(StageTopologyContractError::MaterialMismatch)
        }
    }

    pub fn validate_for_operation(
        &self,
        source: StageTopologyFreezeSource,
        rollout: InvestigationRolloutMode,
    ) -> Result<(), StageTopologyContractError> {
        self.validate()?;
        let legal = match source {
            StageTopologyFreezeSource::LegacyBackfillV1 => {
                self.topology == StageTopologyContract::LegacyCandidateVerificationV1
            }
            StageTopologyFreezeSource::DeploymentPairV1 => {
                self.topology.allows_investigation_rollout(rollout)
            }
        };
        if legal {
            Ok(())
        } else {
            Err(StageTopologyContractError::OperationPairMismatch)
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StageTopologyContractError {
    #[error("unknown stage topology contract: {0}")]
    UnknownTopology(String),
    #[error("unknown stage topology freeze source: {0}")]
    UnknownFreezeSource(String),
    #[error("stage topology canonical material mismatch")]
    MaterialMismatch,
    #[error("stage topology does not match the frozen operation rollout pair")]
    OperationPairMismatch,
}
