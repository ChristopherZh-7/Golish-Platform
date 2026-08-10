//! Exact per-organization read-session identity for unified Investigation.
//!
//! The Main coordinator owns orchestration identity but cannot read raw
//! ContextPack bodies. A host-bound organization session owns one immutable
//! snapshot, context chain, and transcript partition; Main receives only the
//! redacted receipt defined here.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAIN_ORGANIZATION_READ_SESSION_CONTRACT_V1: &str =
    "investigation_main_organization_read_session.v1";

const MAIN_READ_SESSION_NAMESPACE: Uuid = Uuid::from_bytes([
    0x64, 0x20, 0x39, 0x75, 0x78, 0x0c, 0x47, 0x37, 0x81, 0x27, 0x57, 0x6a, 0x6c, 0x1d, 0xf0, 0x35,
]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MainOrganizationReadSessionV1 {
    pub main_read_session_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub session_contract_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMainOrganizationReadSessionV1 {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MainOrganizationReadReceiptV1 {
    pub main_read_session_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: u32,
    pub methodology_result_set_sha256: String,
    pub omission_count: u32,
    pub omission_set_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ReadReceiptDigestMaterial<'a> {
    main_read_session_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    snapshot_id: Uuid,
    snapshot_sha256: &'a str,
    context_item_count: u32,
    context_item_set_sha256: &'a str,
    methodology_hit_count: u32,
    methodology_result_set_sha256: &'a str,
    omission_count: u32,
    omission_set_sha256: &'a str,
}

impl MainOrganizationReadSessionV1 {
    pub fn host_bind(
        input: BindMainOrganizationReadSessionV1,
    ) -> Result<Self, MainReadSessionError> {
        validate_request_id(&input.owning_stage_run_request_id)?;
        for (field, value) in [
            ("operation_id", input.operation_id),
            ("stage_execution_id", input.stage_execution_id),
            ("stage_run_unit_id", input.stage_run_unit_id),
            ("organization_id", input.organization_id),
            ("snapshot_id", input.snapshot_id),
            ("context_chain_id", input.context_chain_id),
            ("transcript_partition_id", input.transcript_partition_id),
        ] {
            if value.is_nil() {
                return Err(MainReadSessionError::InvalidIdentity(field));
            }
        }
        validate_sha256(&input.snapshot_sha256, "snapshot_sha256")?;
        if input.context_chain_id == input.transcript_partition_id {
            return Err(MainReadSessionError::PartitionAlias);
        }
        let identity_bytes = serde_json::to_vec(&(
            input.operation_id,
            input.stage_execution_id,
            &input.owning_stage_run_request_id,
            input.stage_run_unit_id,
            input.organization_id,
            input.snapshot_id,
            &input.snapshot_sha256,
            input.context_chain_id,
            input.transcript_partition_id,
            MAIN_ORGANIZATION_READ_SESSION_CONTRACT_V1,
        ))
        .expect("main read-session identity is serializable");
        let main_read_session_id = Uuid::new_v5(&MAIN_READ_SESSION_NAMESPACE, &identity_bytes);
        Ok(Self {
            main_read_session_id,
            operation_id: input.operation_id,
            stage_execution_id: input.stage_execution_id,
            owning_stage_run_request_id: input.owning_stage_run_request_id,
            stage_run_unit_id: input.stage_run_unit_id,
            organization_id: input.organization_id,
            snapshot_id: input.snapshot_id,
            snapshot_sha256: input.snapshot_sha256,
            context_chain_id: input.context_chain_id,
            transcript_partition_id: input.transcript_partition_id,
            session_contract_version: MAIN_ORGANIZATION_READ_SESSION_CONTRACT_V1.into(),
        })
    }

    pub fn host_receipt(
        &self,
        context_item_count: u32,
        context_item_set_sha256: String,
        methodology_hit_count: u32,
        methodology_result_set_sha256: String,
        omission_count: u32,
        omission_set_sha256: String,
    ) -> Result<MainOrganizationReadReceiptV1, MainReadSessionError> {
        validate_sha256(&context_item_set_sha256, "context_item_set_sha256")?;
        validate_sha256(
            &methodology_result_set_sha256,
            "methodology_result_set_sha256",
        )?;
        validate_sha256(&omission_set_sha256, "omission_set_sha256")?;
        let material = ReadReceiptDigestMaterial {
            main_read_session_id: self.main_read_session_id,
            operation_id: self.operation_id,
            stage_execution_id: self.stage_execution_id,
            stage_run_unit_id: self.stage_run_unit_id,
            organization_id: self.organization_id,
            snapshot_id: self.snapshot_id,
            snapshot_sha256: &self.snapshot_sha256,
            context_item_count,
            context_item_set_sha256: &context_item_set_sha256,
            methodology_hit_count,
            methodology_result_set_sha256: &methodology_result_set_sha256,
            omission_count,
            omission_set_sha256: &omission_set_sha256,
        };
        let receipt_sha256 = sha256_json(&material);
        Ok(MainOrganizationReadReceiptV1 {
            main_read_session_id: self.main_read_session_id,
            operation_id: self.operation_id,
            stage_execution_id: self.stage_execution_id,
            stage_run_unit_id: self.stage_run_unit_id,
            organization_id: self.organization_id,
            snapshot_id: self.snapshot_id,
            snapshot_sha256: self.snapshot_sha256.clone(),
            context_item_count,
            context_item_set_sha256,
            methodology_hit_count,
            methodology_result_set_sha256,
            omission_count,
            omission_set_sha256,
            receipt_sha256,
        })
    }

    pub fn validate_resume(&self, resumed: &Self) -> Result<(), MainReadSessionError> {
        if self != resumed {
            return Err(MainReadSessionError::ResumeIdentityMismatch);
        }
        Ok(())
    }
}

pub fn validate_main_read_session_partition_set(
    sessions: &[MainOrganizationReadSessionV1],
) -> Result<(), MainReadSessionError> {
    let mut session_ids = BTreeSet::new();
    let mut organizations = BTreeSet::new();
    let mut stage_run_units = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    let mut context_chains = BTreeMap::new();
    let mut transcript_partitions = BTreeMap::new();
    let mut stage_run_identity = None::<(Uuid, Uuid, &str)>;
    for session in sessions {
        let identity = (
            session.operation_id,
            session.stage_execution_id,
            session.owning_stage_run_request_id.as_str(),
        );
        if stage_run_identity.is_none() {
            stage_run_identity = Some(identity);
        } else if stage_run_identity != Some(identity) {
            return Err(MainReadSessionError::MixedStageRun);
        }
        if !session_ids.insert(session.main_read_session_id)
            || !organizations.insert(session.organization_id)
        {
            return Err(MainReadSessionError::DuplicateSession);
        }
        if let Some(owner) =
            context_chains.insert(session.context_chain_id, session.organization_id)
        {
            if owner != session.organization_id {
                return Err(MainReadSessionError::CrossOrganizationPartitionReuse(
                    "context_chain_id",
                ));
            }
            return Err(MainReadSessionError::DuplicateSession);
        }
        insert_partition_owner(
            &mut stage_run_units,
            session.stage_run_unit_id,
            session.organization_id,
            "stage_run_unit_id",
        )?;
        insert_partition_owner(
            &mut snapshots,
            session.snapshot_id,
            session.organization_id,
            "snapshot_id",
        )?;
        if let Some(owner) =
            transcript_partitions.insert(session.transcript_partition_id, session.organization_id)
        {
            if owner != session.organization_id {
                return Err(MainReadSessionError::CrossOrganizationPartitionReuse(
                    "transcript_partition_id",
                ));
            }
            return Err(MainReadSessionError::DuplicateSession);
        }
    }
    Ok(())
}

fn insert_partition_owner(
    owners: &mut BTreeMap<Uuid, Uuid>,
    partition_id: Uuid,
    organization_id: Uuid,
    field: &'static str,
) -> Result<(), MainReadSessionError> {
    if let Some(owner) = owners.insert(partition_id, organization_id) {
        if owner != organization_id {
            return Err(MainReadSessionError::CrossOrganizationPartitionReuse(field));
        }
        return Err(MainReadSessionError::DuplicateSession);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MainReadSessionError {
    #[error("invalid main read-session identity: {0}")]
    InvalidIdentity(&'static str),
    #[error("invalid main read-session field: {0}")]
    InvalidField(&'static str),
    #[error("context and transcript partitions must be distinct")]
    PartitionAlias,
    #[error("main read-session resume identity changed")]
    ResumeIdentityMismatch,
    #[error("main read-session set mixes stage runs")]
    MixedStageRun,
    #[error("main read-session is duplicated")]
    DuplicateSession,
    #[error("{0} was reused across organizations")]
    CrossOrganizationPartitionReuse(&'static str),
}

fn validate_request_id(value: &str) -> Result<(), MainReadSessionError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(MainReadSessionError::InvalidField(
            "owning_stage_run_request_id",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, field: &'static str) -> Result<(), MainReadSessionError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(MainReadSessionError::InvalidField(field));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(MainReadSessionError::InvalidField(field));
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("main read-session digest material is serializable"),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect::<String>();
    format!("sha256:{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(
        organization_id: Uuid,
        stage_run_unit_id: Uuid,
        context_chain_id: Uuid,
        transcript_id: Uuid,
    ) -> MainOrganizationReadSessionV1 {
        MainOrganizationReadSessionV1::host_bind(BindMainOrganizationReadSessionV1 {
            operation_id: Uuid::from_u128(100),
            stage_execution_id: Uuid::from_u128(101),
            owning_stage_run_request_id: "stage-run:fixture".into(),
            stage_run_unit_id,
            organization_id,
            snapshot_id: Uuid::new_v5(&organization_id, b"snapshot"),
            snapshot_sha256: format!("sha256:{}", "a".repeat(64)),
            context_chain_id,
            transcript_partition_id: transcript_id,
        })
        .unwrap()
    }

    #[test]
    fn investigation_main_read_sessions_are_deterministic_and_org_partitioned() {
        let first = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let replay = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let second = bind(
            Uuid::from_u128(5),
            Uuid::from_u128(50),
            Uuid::from_u128(6),
            Uuid::from_u128(7),
        );
        assert_eq!(first, replay);
        assert_ne!(first.main_read_session_id, second.main_read_session_id);
        validate_main_read_session_partition_set(&[first, second]).unwrap();
    }

    #[test]
    fn investigation_main_read_session_rejects_cross_org_resume_partition() {
        let first = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let second = bind(
            Uuid::from_u128(5),
            Uuid::from_u128(50),
            Uuid::from_u128(3),
            Uuid::from_u128(7),
        );
        assert_eq!(
            validate_main_read_session_partition_set(&[first, second]),
            Err(MainReadSessionError::CrossOrganizationPartitionReuse(
                "context_chain_id"
            ))
        );
    }

    #[test]
    fn investigation_main_receipt_is_typed_and_contains_no_raw_context() {
        let session = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let receipt = session
            .host_receipt(
                3,
                format!("sha256:{}", "b".repeat(64)),
                2,
                format!("sha256:{}", "c".repeat(64)),
                1,
                format!("sha256:{}", "d".repeat(64)),
            )
            .unwrap();
        let serialized = serde_json::to_string(&receipt).unwrap();
        assert!(!serialized.contains("raw_context"));
        assert!(!serialized.contains("credential"));
        assert!(serialized.contains("receipt_sha256"));
    }

    #[test]
    fn investigation_main_read_session_set_requires_distinct_org_units() {
        let first = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let second = bind(
            Uuid::from_u128(5),
            Uuid::from_u128(20),
            Uuid::from_u128(6),
            Uuid::from_u128(7),
        );
        assert_eq!(
            validate_main_read_session_partition_set(&[first, second]),
            Err(MainReadSessionError::CrossOrganizationPartitionReuse(
                "stage_run_unit_id"
            ))
        );
    }

    #[test]
    fn investigation_main_read_session_set_rejects_mixed_execution() {
        let first = bind(
            Uuid::from_u128(2),
            Uuid::from_u128(20),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let mut second = bind(
            Uuid::from_u128(5),
            Uuid::from_u128(50),
            Uuid::from_u128(6),
            Uuid::from_u128(7),
        );
        second.stage_execution_id = Uuid::from_u128(102);
        assert_eq!(
            validate_main_read_session_partition_set(&[first, second]),
            Err(MainReadSessionError::MixedStageRun)
        );
    }
}
