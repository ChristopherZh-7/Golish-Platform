use golish_cleanup_domain::{AbsenceResult, CleanupError};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndependentAbsenceProof {
    pub executor_worker_run_id: Option<Uuid>,
    pub verifier_worker_run_id: Option<Uuid>,
    pub expected_resource_identity_hash: String,
    pub observed_resource_identity_hash: String,
    pub cleanup_evidence_ids: Vec<i64>,
    pub absence_evidence_ids: Vec<i64>,
    pub result: AbsenceResult,
}

pub fn validate_independent_absence(proof: &IndependentAbsenceProof) -> Result<(), CleanupError> {
    if proof.expected_resource_identity_hash != proof.observed_resource_identity_hash
        || proof.expected_resource_identity_hash.len() != 64
        || proof
            .executor_worker_run_id
            .zip(proof.verifier_worker_run_id)
            .is_some_and(|(executor, verifier)| executor == verifier)
        || proof.absence_evidence_ids.is_empty()
        || proof.absence_evidence_ids.iter().any(|id| *id <= 0)
        || proof
            .absence_evidence_ids
            .iter()
            .any(|id| proof.cleanup_evidence_ids.contains(id))
    {
        return Err(CleanupError::InvalidEvidence);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_and_evidence_must_be_independent() {
        let worker = Uuid::new_v4();
        let proof = IndependentAbsenceProof {
            executor_worker_run_id: Some(worker),
            verifier_worker_run_id: Some(worker),
            expected_resource_identity_hash: "a".repeat(64),
            observed_resource_identity_hash: "a".repeat(64),
            cleanup_evidence_ids: vec![1],
            absence_evidence_ids: vec![1],
            result: AbsenceResult::Absent,
        };
        assert_eq!(
            validate_independent_absence(&proof),
            Err(CleanupError::InvalidEvidence)
        );
    }
}
