//! Deterministic parsing and census rules for durable Campaign consults.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const VERIFICATION_CAMPAIGN_ROLE_IDS: [&str; 13] = [
    "verification_lead",
    "verification_pentester",
    "verification_researcher",
    "verification_poc_designer",
    "verification_auth_specialist",
    "verification_api_specialist",
    "verification_business_logic_specialist",
    "verification_injection_specialist",
    "verification_evidence_analyst",
    "verification_independent_critic",
    "verification_refiner",
    "verification_adviser",
    "verification_reflector",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsultLaneState {
    Queued,
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl ConsultLaneState {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCampaignArtifactV1 {
    pub schema: String,
    pub campaign_id: Uuid,
    pub round_id: Uuid,
    pub consult_lane_id: Uuid,
    pub objective_id: Uuid,
    pub role_id: String,
    pub input_projection_hash: String,
    pub artifact_kind: String,
    pub disposition: String,
    pub obligation_ids: Vec<String>,
    pub coverage_member_hashes: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub residual_codes: Vec<String>,
    pub bounded_observations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCampaignArtifact {
    artifact: VerificationCampaignArtifactV1,
    artifact_hash: String,
    proposal_identity: Uuid,
}

impl ParsedCampaignArtifact {
    pub fn artifact(&self) -> &VerificationCampaignArtifactV1 {
        &self.artifact
    }
    pub fn artifact_hash(&self) -> &str {
        &self.artifact_hash
    }
    pub const fn proposal_identity(&self) -> Uuid {
        self.proposal_identity
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CampaignArtifactError {
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_JSON_INVALID")]
    JsonInvalid,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_SCHEMA_UNKNOWN")]
    SchemaUnknown,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_ROLE_FORBIDDEN")]
    RoleForbidden,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_KIND_FORBIDDEN")]
    ArtifactKindForbidden,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_IDENTITY_INVALID")]
    IdentityInvalid,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_HASH_INVALID")]
    HashInvalid,
    #[error("VERIFICATION_CAMPAIGN_ARTIFACT_SET_INVALID")]
    SetInvalid,
    #[error("VERIFICATION_CAMPAIGN_CONSULT_CENSUS_INVALID")]
    CensusInvalid,
}

impl CampaignArtifactError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::JsonInvalid => "VERIFICATION_CAMPAIGN_ARTIFACT_JSON_INVALID",
            Self::SchemaUnknown => "VERIFICATION_CAMPAIGN_ARTIFACT_SCHEMA_UNKNOWN",
            Self::RoleForbidden => "VERIFICATION_CAMPAIGN_ARTIFACT_ROLE_FORBIDDEN",
            Self::ArtifactKindForbidden => "VERIFICATION_CAMPAIGN_ARTIFACT_KIND_FORBIDDEN",
            Self::IdentityInvalid => "VERIFICATION_CAMPAIGN_ARTIFACT_IDENTITY_INVALID",
            Self::HashInvalid => "VERIFICATION_CAMPAIGN_ARTIFACT_HASH_INVALID",
            Self::SetInvalid => "VERIFICATION_CAMPAIGN_ARTIFACT_SET_INVALID",
            Self::CensusInvalid => "VERIFICATION_CAMPAIGN_CONSULT_CENSUS_INVALID",
        }
    }
}

pub fn is_verification_campaign_role(role_id: &str) -> bool {
    VERIFICATION_CAMPAIGN_ROLE_IDS.contains(&role_id)
}

pub fn parse_campaign_artifact(
    role_id: &str,
    input_projection_hash: &str,
    bytes: &[u8],
) -> Result<ParsedCampaignArtifact, CampaignArtifactError> {
    if !is_verification_campaign_role(role_id) {
        return Err(CampaignArtifactError::RoleForbidden);
    }
    let mut artifact: VerificationCampaignArtifactV1 =
        serde_json::from_slice(bytes).map_err(|_| CampaignArtifactError::JsonInvalid)?;
    if artifact.schema != "verification_campaign_artifact.v1" {
        return Err(CampaignArtifactError::SchemaUnknown);
    }
    if artifact.role_id != role_id {
        return Err(CampaignArtifactError::RoleForbidden);
    }
    if artifact.campaign_id.is_nil()
        || artifact.round_id.is_nil()
        || artifact.consult_lane_id.is_nil()
        || artifact.objective_id.is_nil()
    {
        return Err(CampaignArtifactError::IdentityInvalid);
    }
    if !valid_hash(input_projection_hash) || artifact.input_projection_hash != input_projection_hash
    {
        return Err(CampaignArtifactError::HashInvalid);
    }
    let expected_kind =
        artifact_kind_for_role(role_id).ok_or(CampaignArtifactError::RoleForbidden)?;
    if artifact.artifact_kind != expected_kind {
        return Err(CampaignArtifactError::ArtifactKindForbidden);
    }
    if artifact.disposition != "proposed" {
        return Err(CampaignArtifactError::SetInvalid);
    }
    canonicalize_exact_set(&mut artifact.obligation_ids)?;
    canonicalize_exact_set(&mut artifact.coverage_member_hashes)?;
    canonicalize_exact_set(&mut artifact.evidence_refs)?;
    canonicalize_exact_set(&mut artifact.residual_codes)?;
    if artifact
        .coverage_member_hashes
        .iter()
        .any(|value| !valid_hash(value))
    {
        return Err(CampaignArtifactError::HashInvalid);
    }
    if artifact.bounded_observations.len() > 32
        || artifact
            .bounded_observations
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 1_024)
    {
        return Err(CampaignArtifactError::SetInvalid);
    }
    let artifact_hash = hash_json("verification_campaign_artifact.v1", &artifact);
    let proposal_identity = Uuid::new_v5(&artifact.consult_lane_id, artifact_hash.as_bytes());
    Ok(ParsedCampaignArtifact {
        artifact,
        artifact_hash,
        proposal_identity,
    })
}

/// Recheck the model artifact against the exact redacted request packet frozen
/// before provider dispatch. The lane id is checked by the caller because it is
/// derived from this packet's member hash and is intentionally not self-hashed.
pub fn campaign_artifact_matches_frozen_request(
    artifact: &VerificationCampaignArtifactV1,
    request_packet: &serde_json::Value,
) -> bool {
    fn strings(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
        value?
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect()
    }
    request_packet
        .get("campaign_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        == Some(artifact.campaign_id)
        && request_packet
            .get("round_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(artifact.round_id)
        && request_packet
            .get("objective_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(artifact.objective_id)
        && request_packet
            .get("role_id")
            .and_then(serde_json::Value::as_str)
            == Some(artifact.role_id.as_str())
        && request_packet
            .get("input_projection_hash")
            .and_then(serde_json::Value::as_str)
            == Some(artifact.input_projection_hash.as_str())
        && request_packet
            .get("artifact_kind")
            .and_then(serde_json::Value::as_str)
            == Some(artifact.artifact_kind.as_str())
        && strings(request_packet.get("obligation_ids")) == Some(artifact.obligation_ids.clone())
        && strings(request_packet.get("coverage_member_hashes"))
            == Some(artifact.coverage_member_hashes.clone())
        && strings(request_packet.get("residual_codes")) == Some(artifact.residual_codes.clone())
}

fn artifact_kind_for_role(role_id: &str) -> Option<&'static str> {
    match role_id {
        "verification_lead" => Some("strategy_decision_or_terminal_intent"),
        "verification_evidence_analyst" => Some("evidence_analysis"),
        "verification_independent_critic" => Some("independent_critique"),
        "verification_refiner" => Some("typed_plan_delta"),
        "verification_adviser" | "verification_reflector" => Some("bounded_recovery_advice"),
        value if is_verification_campaign_role(value) => Some("consult_proposal"),
        _ => None,
    }
}

fn canonicalize_exact_set(values: &mut [String]) -> Result<(), CampaignArtifactError> {
    if values
        .iter()
        .any(|value| value.trim().is_empty() || value.trim() != value || value.len() > 512)
    {
        return Err(CampaignArtifactError::SetInvalid);
    }
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CampaignArtifactError::SetInvalid);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsultLaneTerminal {
    pub consult_lane_id: Uuid,
    pub state: ConsultLaneState,
    pub artifact_hash: Option<String>,
}

pub fn seal_consult_census(
    expected_lane_ids: &[Uuid],
    terminals: &[ConsultLaneTerminal],
) -> Result<String, CampaignArtifactError> {
    if !(1..=3).contains(&expected_lane_ids.len())
        || expected_lane_ids.iter().any(Uuid::is_nil)
        || terminals.iter().any(|terminal| {
            terminal.consult_lane_id.is_nil()
                || !terminal.state.terminal()
                || (terminal.state == ConsultLaneState::Completed
                    && terminal
                        .artifact_hash
                        .as_deref()
                        .is_none_or(|hash| !valid_hash(hash)))
                || (terminal.state != ConsultLaneState::Completed
                    && terminal.artifact_hash.is_some())
        })
    {
        return Err(CampaignArtifactError::CensusInvalid);
    }
    let expected = expected_lane_ids.iter().copied().collect::<BTreeSet<_>>();
    let actual = terminals
        .iter()
        .map(|terminal| terminal.consult_lane_id)
        .collect::<BTreeSet<_>>();
    if expected.len() != expected_lane_ids.len()
        || actual.len() != terminals.len()
        || expected != actual
    {
        return Err(CampaignArtifactError::CensusInvalid);
    }
    let mut canonical = terminals.to_vec();
    canonical.sort_by_key(|terminal| terminal.consult_lane_id);
    Ok(hash_json(
        "verification_campaign_consult_census.v1",
        &canonical
            .iter()
            .map(|terminal| {
                (
                    terminal.consult_lane_id,
                    terminal.state,
                    terminal.artifact_hash.as_deref(),
                )
            })
            .collect::<Vec<_>>(),
    ))
}

fn valid_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn hash_json<T: Serialize>(domain: &str, value: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(value).expect("typed Campaign artifact must serialize"));
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    #[test]
    fn verification_campaign_artifact_identity_ignores_completion_order() {
        let lane = Uuid::from_u128(3);
        let value = json!({
            "schema":"verification_campaign_artifact.v1",
            "campaign_id":Uuid::from_u128(1),
            "round_id":Uuid::from_u128(2),
            "consult_lane_id":lane,
            "objective_id":Uuid::from_u128(4),
            "role_id":"verification_api_specialist",
            "input_projection_hash":hash('a'),
            "artifact_kind":"consult_proposal",
            "disposition":"proposed",
            "obligation_ids":["b","a"],
            "coverage_member_hashes":[hash('c'),hash('b')],
            "evidence_refs":[],
            "residual_codes":["z","a"],
            "bounded_observations":["bounded"]
        });
        let first = parse_campaign_artifact(
            "verification_api_specialist",
            &hash('a'),
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap();
        let second = parse_campaign_artifact(
            "verification_api_specialist",
            &hash('a'),
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap();
        assert_eq!(first.proposal_identity(), second.proposal_identity());
        assert_eq!(first.artifact().obligation_ids, ["a", "b"]);
    }

    #[test]
    fn verification_campaign_role_cannot_submit_another_roles_artifact() {
        let value = json!({
            "schema":"verification_campaign_artifact.v1",
            "campaign_id":Uuid::from_u128(1),"round_id":Uuid::from_u128(2),
            "consult_lane_id":Uuid::from_u128(3),"objective_id":Uuid::from_u128(4),
            "role_id":"verification_lead","input_projection_hash":hash('a'),
            "artifact_kind":"consult_proposal","disposition":"proposed",
            "obligation_ids":[],"coverage_member_hashes":[],"evidence_refs":[],
            "residual_codes":[],"bounded_observations":[]
        });
        let error = parse_campaign_artifact(
            "verification_lead",
            &hash('a'),
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            "VERIFICATION_CAMPAIGN_ARTIFACT_KIND_FORBIDDEN"
        );
    }

    #[test]
    fn verification_campaign_census_is_exact_and_bounded() {
        let lanes = [Uuid::from_u128(1), Uuid::from_u128(2)];
        let terminals = [
            ConsultLaneTerminal {
                consult_lane_id: lanes[1],
                state: ConsultLaneState::Failed,
                artifact_hash: None,
            },
            ConsultLaneTerminal {
                consult_lane_id: lanes[0],
                state: ConsultLaneState::Completed,
                artifact_hash: Some(hash('a')),
            },
        ];
        assert!(valid_hash(
            &seal_consult_census(&lanes, &terminals).unwrap()
        ));
        assert_eq!(
            seal_consult_census(&[lanes[0]], &terminals)
                .unwrap_err()
                .code(),
            "VERIFICATION_CAMPAIGN_CONSULT_CENSUS_INVALID"
        );
    }
}
