//! Persisted rollout contract for the operation-scoped runtime-memory store.
//!
//! The contract is a single monotonic state, not independent read/write knobs.
//! That prevents deployments from selecting a writer that the same operation
//! cannot read after restart. Each operation freezes one value at creation.

use std::fmt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::db_traits::RuntimeMemoryError;

/// The complete source selected for a runtime-memory read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMemoryReadStrategy {
    LegacyOnly,
    /// Read one complete V2 record when it is valid; otherwise read one complete
    /// legacy record. Fields from the two sources are never merged.
    CompleteV2ElseLegacy,
    V2Only,
}

/// The atomic destination selected for a runtime-memory write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMemoryWriteStrategy {
    LegacyOnly,
    AtomicDualWrite,
    V2Only,
}

/// Concrete read/write behavior derived from a persisted contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeMemoryPolicy {
    pub read: RuntimeMemoryReadStrategy,
    pub write: RuntimeMemoryWriteStrategy,
    pub may_merge_fields_from_two_sources: bool,
}

/// Monotonic runtime-memory rollout state frozen on an operation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeMemoryContract {
    #[default]
    LegacyV1,
    DualWriteLegacyRead,
    DualWriteV2Preferred,
    V2Only,
}

impl RuntimeMemoryContract {
    pub const ALL: [Self; 4] = [
        Self::LegacyV1,
        Self::DualWriteLegacyRead,
        Self::DualWriteV2Preferred,
        Self::V2Only,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyV1 => "legacy_v1",
            Self::DualWriteLegacyRead => "dual_write_legacy_read",
            Self::DualWriteV2Preferred => "dual_write_v2_preferred",
            Self::V2Only => "v2_only",
        }
    }

    pub const fn policy(self) -> RuntimeMemoryPolicy {
        let (read, write) = match self {
            Self::LegacyV1 => (
                RuntimeMemoryReadStrategy::LegacyOnly,
                RuntimeMemoryWriteStrategy::LegacyOnly,
            ),
            Self::DualWriteLegacyRead => (
                RuntimeMemoryReadStrategy::LegacyOnly,
                RuntimeMemoryWriteStrategy::AtomicDualWrite,
            ),
            Self::DualWriteV2Preferred => (
                RuntimeMemoryReadStrategy::CompleteV2ElseLegacy,
                RuntimeMemoryWriteStrategy::AtomicDualWrite,
            ),
            Self::V2Only => (
                RuntimeMemoryReadStrategy::V2Only,
                RuntimeMemoryWriteStrategy::V2Only,
            ),
        };
        RuntimeMemoryPolicy {
            read,
            write,
            may_merge_fields_from_two_sources: false,
        }
    }

    /// Rollout defaults may remain unchanged or advance exactly one state.
    /// Existing operations never transition; they retain their frozen value.
    pub const fn can_transition_to(self, next: Self) -> bool {
        next as u8 == self as u8 || next as u8 == self as u8 + 1
    }
}

impl fmt::Display for RuntimeMemoryContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRuntimeMemoryContract(String);

impl fmt::Display for UnknownRuntimeMemoryContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown runtime-memory contract: {}", self.0)
    }
}

impl std::error::Error for UnknownRuntimeMemoryContract {}

impl TryFrom<&str> for RuntimeMemoryContract {
    type Error = UnknownRuntimeMemoryContract;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "legacy_v1" => Ok(Self::LegacyV1),
            "dual_write_legacy_read" => Ok(Self::DualWriteLegacyRead),
            "dual_write_v2_preferred" => Ok(Self::DualWriteV2Preferred),
            "v2_only" => Ok(Self::V2Only),
            other => Err(UnknownRuntimeMemoryContract(other.to_string())),
        }
    }
}

/// Resolve a trusted workspace identity from an existing directory.
///
/// The path is provenance; the returned digest is used to detect accidental
/// path rebinding while the database-assigned `project_scope_id` remains the
/// authorization identity.
pub fn canonical_workspace_identity(path: &Path) -> Result<(String, String), RuntimeMemoryError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| RuntimeMemoryError::Storage(format!("canonicalize workspace: {error}")))?;
    if !canonical.is_dir() {
        return Err(RuntimeMemoryError::Conflict {
            code: "workspace_not_directory",
        });
    }
    let canonical = canonical.to_string_lossy().into_owned();
    let digest = Sha256::digest(canonical.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((canonical, digest))
}

/// Authorize the current trusted project identity against the immutable scope
/// frozen on an operation. Legacy V1 rows predate `project_scope_id` and retain
/// their compatibility path; every V2-capable contract fails closed when the
/// binding is absent or points at another project.
pub fn authorize_operation_project_scope(
    persisted_project_scope_id: Option<uuid::Uuid>,
    contract: RuntimeMemoryContract,
    current_project_scope_id: uuid::Uuid,
) -> Result<(), RuntimeMemoryError> {
    match persisted_project_scope_id {
        Some(persisted) if persisted == current_project_scope_id => Ok(()),
        Some(_) => Err(RuntimeMemoryError::IdentityMismatch {
            code: "operation_project_scope_mismatch",
        }),
        None if contract == RuntimeMemoryContract::LegacyV1 => Ok(()),
        None => Err(RuntimeMemoryError::Missing {
            entity: "operation_state.project_scope_id",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_operation_project_scope, canonical_workspace_identity, RuntimeMemoryContract,
        RuntimeMemoryReadStrategy, RuntimeMemoryWriteStrategy,
    };

    #[test]
    fn runtime_memory_contract_exposes_only_safe_read_write_pairs() {
        use RuntimeMemoryContract::{DualWriteLegacyRead, DualWriteV2Preferred, LegacyV1, V2Only};

        let cases = [
            (
                LegacyV1,
                RuntimeMemoryReadStrategy::LegacyOnly,
                RuntimeMemoryWriteStrategy::LegacyOnly,
            ),
            (
                DualWriteLegacyRead,
                RuntimeMemoryReadStrategy::LegacyOnly,
                RuntimeMemoryWriteStrategy::AtomicDualWrite,
            ),
            (
                DualWriteV2Preferred,
                RuntimeMemoryReadStrategy::CompleteV2ElseLegacy,
                RuntimeMemoryWriteStrategy::AtomicDualWrite,
            ),
            (
                V2Only,
                RuntimeMemoryReadStrategy::V2Only,
                RuntimeMemoryWriteStrategy::V2Only,
            ),
        ];

        for (contract, expected_read, expected_write) in cases {
            let policy = contract.policy();
            assert_eq!(policy.read, expected_read);
            assert_eq!(policy.write, expected_write);
            assert!(!policy.may_merge_fields_from_two_sources);
        }
    }

    #[test]
    fn runtime_memory_contract_transition_is_monotonic_and_adjacent() {
        use RuntimeMemoryContract::{DualWriteLegacyRead, DualWriteV2Preferred, LegacyV1, V2Only};

        assert!(LegacyV1.can_transition_to(LegacyV1));
        assert!(LegacyV1.can_transition_to(DualWriteLegacyRead));
        assert!(DualWriteLegacyRead.can_transition_to(DualWriteV2Preferred));
        assert!(DualWriteV2Preferred.can_transition_to(V2Only));
        assert!(V2Only.can_transition_to(V2Only));

        assert!(!LegacyV1.can_transition_to(DualWriteV2Preferred));
        assert!(!LegacyV1.can_transition_to(V2Only));
        assert!(!DualWriteLegacyRead.can_transition_to(V2Only));
        assert!(!DualWriteV2Preferred.can_transition_to(LegacyV1));
        assert!(!V2Only.can_transition_to(DualWriteV2Preferred));
    }

    #[test]
    fn runtime_memory_contract_roundtrips_persisted_value_and_rejects_unknown() {
        for contract in RuntimeMemoryContract::ALL {
            let persisted = contract.as_str();
            assert_eq!(RuntimeMemoryContract::try_from(persisted), Ok(contract));
        }

        assert!(RuntimeMemoryContract::try_from("legacy_read_v2_write").is_err());
        assert!(RuntimeMemoryContract::try_from("future_contract").is_err());
    }

    #[test]
    fn canonical_workspace_identity_uses_the_real_directory_and_stable_sha256() {
        let workspace = tempfile::tempdir().expect("workspace fixture");
        let (canonical, digest) =
            canonical_workspace_identity(workspace.path()).expect("canonical workspace");

        assert_eq!(
            canonical,
            workspace.path().canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            canonical_workspace_identity(workspace.path()).unwrap(),
            (canonical, digest)
        );
    }

    #[test]
    fn canonical_workspace_identity_rejects_missing_or_non_directory_paths() {
        let workspace = tempfile::tempdir().expect("workspace fixture");
        let file = workspace.path().join("file.txt");
        std::fs::write(&file, b"fixture").expect("write fixture");

        assert!(canonical_workspace_identity(&file).is_err());
        assert!(canonical_workspace_identity(&workspace.path().join("missing")).is_err());
    }

    #[test]
    fn operation_project_scope_authorization_rejects_rebinding_but_allows_legacy_v1() {
        let current = uuid::Uuid::new_v4();
        assert!(authorize_operation_project_scope(
            Some(current),
            RuntimeMemoryContract::DualWriteLegacyRead,
            current,
        )
        .is_ok());
        assert!(authorize_operation_project_scope(
            Some(uuid::Uuid::new_v4()),
            RuntimeMemoryContract::DualWriteLegacyRead,
            current,
        )
        .is_err());
        assert!(
            authorize_operation_project_scope(None, RuntimeMemoryContract::LegacyV1, current,)
                .is_ok()
        );
        assert!(authorize_operation_project_scope(
            None,
            RuntimeMemoryContract::DualWriteLegacyRead,
            current,
        )
        .is_err());
    }
}
