//! Versioned public envelope over the existing Plan B RR projection snapshot.
//!
//! This module does not own a reducer or cursor policy.  It translates the
//! single Plan B `InvestigationReadAuthority` into the stable Plan D envelope
//! while returning the very same repeatable-read transaction to the caller.

use chrono::{DateTime, Utc};
use golish_core::{
    InvestigationContractVersion, InvestigationModePolicy, InvestigationRolloutMode,
};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

use super::types::{
    invalid_payload, InvestigationOperationReadAuthority, InvestigationProjectionError,
    InvestigationProjectionResult, ProjectionStaleReason, INVESTIGATION_PROJECTION_STALE,
};
use super::InvestigationProjectionReadSnapshot;

pub type OperationReadAuthority = InvestigationOperationReadAuthority;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyField<T> {
    Available(T),
    LegacyUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionTemporalStatusV1 {
    Current,
    TemporallyStale,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionAuthorityTimeV1 {
    pub authority_ref_hash: [u8; 32],
    pub effective_valid_until: Option<DateTime<Utc>>,
    pub authority_epoch_hash: [u8; 32],
    pub observed_as_of: DateTime<Utc>,
    pub temporal_status: ProjectionTemporalStatusV1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionHead {
    pub projection_schema_version: u32,
    pub change_seq: i64,
    pub read_at: DateTime<Utc>,
    pub as_of_temporal_cutoff: Option<DateTime<Utc>>,
    pub authority_epoch_set_hash: [u8; 32],
    pub tool_truth_contract: ToolTruthContract,
    pub investigation_contract_version: InvestigationContractVersion,
    pub investigation_rollout_mode: InvestigationRolloutMode,
    pub mode_policy: InvestigationModePolicy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionItem<T> {
    pub head: ProjectionHead,
    pub authority_time: ProjectionAuthorityTimeV1,
    pub data: T,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionPage<T, K> {
    pub head: ProjectionHead,
    pub items: Vec<ProjectionItem<T>>,
    pub next_sort_key: Option<K>,
}

fn tagged_hash_bytes(value: &str) -> InvestigationProjectionResult<[u8; 32]> {
    let hex = value.strip_prefix("sha256:").ok_or_else(|| {
        invalid_payload("projection authority hash is not a tagged SHA-256 digest")
    })?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_payload(
            "projection authority hash is not canonical lowercase SHA-256",
        ));
    }
    let mut result = [0u8; 32];
    for (index, slot) in result.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| invalid_payload("projection authority hash is malformed"))?;
    }
    Ok(result)
}

/// Begin the versioned read model on the existing Plan B RR snapshot.
///
/// The supplied authority must have been captured by the repository and is
/// exact-compared with the fresh server-owned operation contract/cursor salt.
/// A caller cannot assemble a weaker operation selector or override policy.
pub async fn begin_read_snapshot<'a>(
    pool: &'a PgPool,
    authority: &OperationReadAuthority,
    expected_change_seq: Option<i64>,
) -> InvestigationProjectionResult<(Transaction<'a, Postgres>, ProjectionHead)> {
    let snapshot = InvestigationProjectionReadSnapshot::begin(pool, authority.operation_id).await?;
    if &snapshot.authority.operation != authority {
        return Err(InvestigationProjectionError::Contract(
            "INVESTIGATION_AUTHORITY_CORRUPT",
        ));
    }
    let current_change_seq = snapshot.authority.temporal.as_of_change_seq;
    if expected_change_seq.is_some_and(|expected| expected != current_change_seq) {
        return Err(InvestigationProjectionError::Stale {
            code: INVESTIGATION_PROJECTION_STALE,
            current_change_seq,
            reason: ProjectionStaleReason::ChangeSeqAdvanced,
        });
    }

    let tool_truth_contract =
        ToolTruthContract::try_from(snapshot.authority.operation.tool_truth_contract.as_str())
            .map_err(|_| invalid_payload("unknown frozen Tool Truth contract"))?;
    let investigation_contract_version = InvestigationContractVersion::try_from(
        snapshot
            .authority
            .operation
            .investigation_contract_version
            .as_str(),
    )
    .map_err(|_| invalid_payload("unknown frozen Investigation contract"))?;
    let investigation_rollout_mode = InvestigationRolloutMode::try_from(
        snapshot
            .authority
            .operation
            .investigation_rollout_mode
            .as_str(),
    )
    .map_err(|_| invalid_payload("unknown frozen Investigation rollout mode"))?;
    let temporal = &snapshot.authority.temporal;
    let projection_schema_version = u32::try_from(temporal.projection_schema_version)
        .map_err(|_| invalid_payload("projection schema version is negative"))?;
    let head = ProjectionHead {
        projection_schema_version,
        change_seq: temporal.as_of_change_seq,
        read_at: temporal.as_of_temporal_cutoff,
        as_of_temporal_cutoff: Some(temporal.earliest_effective_valid_until),
        authority_epoch_set_hash: tagged_hash_bytes(&temporal.authority_epoch_set_hash)?,
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
        mode_policy: investigation_rollout_mode.policy(),
    };
    Ok((snapshot.tx, head))
}

#[cfg(test)]
mod tests {
    use super::tagged_hash_bytes;

    #[test]
    fn versioned_projection_hash_requires_tagged_lowercase_sha256() {
        assert_eq!(
            tagged_hash_bytes(&format!("sha256:{}", "00".repeat(32))).expect("valid digest"),
            [0; 32],
        );
        assert!(tagged_hash_bytes(&format!("sha256:{}", "AA".repeat(32))).is_err());
        assert!(tagged_hash_bytes(&"00".repeat(32)).is_err());
    }
}
