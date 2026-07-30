pub use golish_core::hypothesis_semantic_key::{derive_root_id, merge_root_id, split_root_id};

use golish_core::hypothesis_semantic_key::{
    initial_root_id as core_initial_root_id, HypothesisSemanticKeyError, HypothesisSemanticKeyV1,
};
use uuid::Uuid;

/// Compatibility-shaped host wrapper which also verifies that the separately
/// supplied organization matches the sealed semantic key.
pub fn initial_root_id(
    operation_id: Uuid,
    organization_id: Uuid,
    semantic_key: &HypothesisSemanticKeyV1,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    if organization_id != semantic_key.organization_id() {
        return Err(HypothesisSemanticKeyError::NilUuid(
            "organization_id_mismatch",
        ));
    }
    core_initial_root_id(operation_id, semantic_key)
}
