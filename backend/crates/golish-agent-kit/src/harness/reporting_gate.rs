//! Pure, DB-free Reporting stage gate contract.
//!
//! The application adapter re-reads canonical Reporting truth and supplies this
//! narrow snapshot. The gate never consumes model prose, renderer output, RAG,
//! Graph/KG state, project paths, actor ids, or artifact publication status as
//! evidence that a report is validated.

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportingGateTruth {
    pub operation_id: Uuid,
    pub report_id: Uuid,
    pub current_revision_id: Uuid,
    pub revision_id: Uuid,
    pub validation_status: String,
    pub publication_status: String,
    pub stored_source_set_hash: String,
    pub current_source_set_hash: String,
    pub source_snapshot_exact: bool,
    pub claims_citations_valid: bool,
    pub validation_attestation_valid: bool,
    pub cleanup_closeout_valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportingGateBlock {
    pub code: &'static str,
}

impl std::fmt::Display for ReportingGateBlock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ReportingGateBlock {}

pub fn validate_reporting_gate_truth(truth: &ReportingGateTruth) -> Result<(), ReportingGateBlock> {
    let block = |code| ReportingGateBlock { code };
    if truth.current_revision_id != truth.revision_id {
        return Err(block("REPORT_REVISION_NOT_CURRENT"));
    }
    if truth.validation_status != "validated" {
        return Err(block("REPORT_REVISION_NOT_VALIDATED"));
    }
    if truth.publication_status == "superseded" {
        return Err(block("REPORT_REVISION_SUPERSEDED"));
    }
    if !matches!(truth.publication_status.as_str(), "unpublished" | "final") {
        return Err(block("REPORT_PUBLICATION_STATE_INVALID"));
    }
    if !truth.source_snapshot_exact || truth.stored_source_set_hash != truth.current_source_set_hash
    {
        return Err(block("REPORT_SOURCE_SNAPSHOT_STALE"));
    }
    if !truth.claims_citations_valid {
        return Err(block("REPORT_CITATION_UNRESOLVED"));
    }
    if !truth.validation_attestation_valid {
        return Err(block("REPORT_VALIDATION_ATTESTATION_INVALID"));
    }
    if !truth.cleanup_closeout_valid {
        return Err(block("CLEANUP_CLOSEOUT_BLOCKED"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ReportingGateTruth {
        let revision_id = Uuid::new_v4();
        ReportingGateTruth {
            operation_id: Uuid::new_v4(),
            report_id: Uuid::new_v4(),
            current_revision_id: revision_id,
            revision_id,
            validation_status: "validated".to_string(),
            publication_status: "unpublished".to_string(),
            stored_source_set_hash: "a".repeat(64),
            current_source_set_hash: "a".repeat(64),
            source_snapshot_exact: true,
            claims_citations_valid: true,
            validation_attestation_valid: true,
            cleanup_closeout_valid: true,
        }
    }

    #[test]
    fn reporting_read_model_gate_current_validated_unpublished_or_final_revision_passes() {
        let mut truth = valid();
        validate_reporting_gate_truth(&truth).expect("validated draft passes");
        truth.publication_status = "final".to_string();
        validate_reporting_gate_truth(&truth).expect("final publication keeps validation pass");
    }

    #[test]
    fn reporting_read_model_gate_noncurrent_revision_blocks() {
        let mut truth = valid();
        truth.current_revision_id = Uuid::new_v4();
        let error = validate_reporting_gate_truth(&truth).expect_err("noncurrent must block");
        assert_eq!(error.code, "REPORT_REVISION_NOT_CURRENT");
    }

    #[test]
    fn reporting_read_model_gate_draft_or_superseded_revision_blocks() {
        let mut truth = valid();
        truth.validation_status = "draft".to_string();
        let draft = validate_reporting_gate_truth(&truth).expect_err("draft must block");
        assert_eq!(draft.code, "REPORT_REVISION_NOT_VALIDATED");

        truth.validation_status = "validated".to_string();
        truth.publication_status = "superseded".to_string();
        let superseded = validate_reporting_gate_truth(&truth).expect_err("superseded must block");
        assert_eq!(superseded.code, "REPORT_REVISION_SUPERSEDED");
    }

    #[test]
    fn reporting_read_model_gate_stale_complete_source_snapshot_blocks() {
        let mut truth = valid();
        truth.current_source_set_hash = "b".repeat(64);
        let changed = validate_reporting_gate_truth(&truth).expect_err("changed source blocks");
        assert_eq!(changed.code, "REPORT_SOURCE_SNAPSHOT_STALE");

        truth.current_source_set_hash = truth.stored_source_set_hash.clone();
        truth.source_snapshot_exact = false;
        let deleted = validate_reporting_gate_truth(&truth).expect_err("deleted source blocks");
        assert_eq!(deleted.code, "REPORT_SOURCE_SNAPSHOT_STALE");
    }

    #[test]
    fn reporting_read_model_gate_unresolved_claim_or_invalid_attestation_blocks() {
        let mut truth = valid();
        truth.claims_citations_valid = false;
        let citation = validate_reporting_gate_truth(&truth).expect_err("citation must resolve");
        assert_eq!(citation.code, "REPORT_CITATION_UNRESOLVED");

        truth.claims_citations_valid = true;
        truth.validation_attestation_valid = false;
        let attestation =
            validate_reporting_gate_truth(&truth).expect_err("attestation must validate");
        assert_eq!(attestation.code, "REPORT_VALIDATION_ATTESTATION_INVALID");
    }

    #[test]
    fn reporting_read_model_gate_cleanup_closeout_regression_blocks_even_after_validation() {
        let mut truth = valid();
        truth.cleanup_closeout_valid = false;
        let error = validate_reporting_gate_truth(&truth).expect_err("cleanup regression blocks");
        assert_eq!(error.code, "CLEANUP_CLOSEOUT_BLOCKED");
    }
}
