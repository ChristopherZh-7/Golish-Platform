use golish_reporting_domain::{validate_report, ReportReadModel, ReportValidationTruth};
use uuid::Uuid;

use crate::{ReportTruthPort, ReportingAppError};

#[derive(Clone, Debug, PartialEq)]
pub struct BuiltReportRevision {
    pub model: ReportReadModel,
    pub validation_truth: ReportValidationTruth,
    pub revision_number: i32,
    pub expected_current_revision_id: Option<Uuid>,
    pub expected_row_version: i64,
}

pub struct ReportReadModelBuilder<T> {
    truth: T,
}

impl<T> ReportReadModelBuilder<T>
where
    T: ReportTruthPort,
{
    pub fn new(truth: T) -> Self {
        Self { truth }
    }

    pub async fn build_and_validate(
        &self,
        operation_id: Uuid,
    ) -> Result<BuiltReportRevision, ReportingAppError> {
        let mut revision = self
            .truth
            .build_repeatable_read_snapshot(operation_id)
            .await?;
        let validation_result = validate_report(&revision.model, &revision.validation_truth)
            .map_err(|error| ReportingAppError::Validation(error.to_string()))?;
        let current = self.truth.current_source_snapshot(operation_id).await?;
        if current.ordered_sources != revision.model.source_snapshot.ordered_sources
            || current.source_set_hash != revision.model.source_snapshot.source_set_hash
        {
            return Err(ReportingAppError::SourceSnapshotStale);
        }
        revision.expected_row_version = self
            .truth
            .persist_validated_revision(&revision, &validation_result)
            .await?;
        Ok(revision)
    }
}
