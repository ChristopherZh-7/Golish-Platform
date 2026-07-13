use golish_reporting_domain::{PublicationStatus, ReportReadModel, ValidationStatus};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ContentAddressedArtifact, FinalizePublication, ReportArtifactStore, ReportFormat,
    ReportPublicationPort, ReportingAppError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplicitFinalizeRequest {
    pub principal_id: Uuid,
    pub confirm_final_publish: bool,
    pub expected_row_version: i64,
    pub validation_status: ValidationStatus,
    pub publication_status: PublicationStatus,
}

pub struct ReportFinalizer<S, P> {
    store: S,
    publication: P,
}

fn deterministic_content_key(format: ReportFormat, bytes: &[u8]) -> String {
    let sha256: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let extension = match format {
        ReportFormat::Markdown => "md",
        ReportFormat::Json => "json",
    };
    format!("sha256/{sha256}.{extension}")
}

impl<S, P> ReportFinalizer<S, P>
where
    S: ReportArtifactStore,
    P: ReportPublicationPort,
{
    pub fn new(store: S, publication: P) -> Self {
        Self { store, publication }
    }

    pub async fn finalize(
        &self,
        model: &ReportReadModel,
        request: ExplicitFinalizeRequest,
        rendered: Vec<(ReportFormat, Vec<u8>)>,
    ) -> Result<Vec<ContentAddressedArtifact>, ReportingAppError> {
        if !request.confirm_final_publish {
            return Err(ReportingAppError::FinalizeConfirmationRequired);
        }
        if request.validation_status != ValidationStatus::Validated
            || request.publication_status != PublicationStatus::Unpublished
        {
            return Err(ReportingAppError::RevisionNotValidated);
        }

        let mut output_keys = Vec::with_capacity(rendered.len());
        let mut unique_rendered = BTreeMap::<String, (ReportFormat, Vec<u8>)>::new();
        for (format, bytes) in rendered {
            let content_key = deterministic_content_key(format, &bytes);
            if let Some((existing_format, existing_bytes)) = unique_rendered.get(&content_key) {
                if *existing_format != format || *existing_bytes != bytes {
                    return Err(ReportingAppError::Artifact(
                        "report_artifact_content_key_collision".to_string(),
                    ));
                }
            } else {
                unique_rendered.insert(content_key.clone(), (format, bytes));
            }
            output_keys.push(content_key);
        }

        let mut staged = Vec::with_capacity(unique_rendered.len());
        let mut artifacts = Vec::with_capacity(unique_rendered.len());
        let mut artifacts_by_key = BTreeMap::new();
        let mut reservations = Vec::new();
        for (expected_content_key, (format, bytes)) in unique_rendered {
            let item = self.store.stage(model.revision_id, format, &bytes).await?;
            let reservation = self.store.promote(&item).await?;
            let artifact = reservation.artifact().clone();
            if artifact.content_key != expected_content_key {
                return Err(ReportingAppError::Artifact(
                    "report_artifact_content_key_mismatch".to_string(),
                ));
            }
            if !self.store.verify(&artifact).await? {
                return Err(ReportingAppError::ArtifactVerificationFailed);
            }
            staged.push(item);
            artifacts_by_key.insert(expected_content_key, artifact.clone());
            artifacts.push(artifact);
            reservations.push(reservation);
        }

        self.publication
            .finalize_publication(FinalizePublication {
                operation_id: model.operation_id,
                report_id: model.report_id,
                revision_id: model.revision_id,
                expected_row_version: request.expected_row_version,
                expected_source_snapshot: model.source_snapshot.clone(),
                principal_id: request.principal_id,
                artifacts: artifacts.clone(),
            })
            .await?;

        // Publication has attached the content keys in the database. Release
        // the per-key leases before cleanup, whose storage implementation takes
        // the same lock when discarding staging files.
        drop(reservations);
        for item in &staged {
            self.store.discard_staging(item).await?;
        }
        output_keys
            .iter()
            .map(|content_key| {
                artifacts_by_key.get(content_key).cloned().ok_or_else(|| {
                    ReportingAppError::Artifact(
                        "report_artifact_output_mapping_missing".to_string(),
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use golish_reporting_domain::ReportSourceSnapshot;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{ArtifactPublicationReservation, ReportArtifactStore, StagedArtifact};

    fn content_key(format: ReportFormat, bytes: &[u8]) -> String {
        let sha256: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let extension = match format {
            ReportFormat::Markdown => "md",
            ReportFormat::Json => "json",
        };
        format!("sha256/{sha256}.{extension}")
    }

    fn artifact(format: ReportFormat, bytes: &[u8]) -> ContentAddressedArtifact {
        let content_key = content_key(format, bytes);
        ContentAddressedArtifact {
            format,
            sha256: content_key
                .trim_start_matches("sha256/")
                .split('.')
                .next()
                .expect("sha component")
                .to_string(),
            content_key,
            byte_len: u64::try_from(bytes.len()).expect("artifact byte length"),
        }
    }

    struct TestReservation(ContentAddressedArtifact);

    impl ArtifactPublicationReservation for TestReservation {
        fn artifact(&self) -> &ContentAddressedArtifact {
            &self.0
        }
    }

    #[derive(Clone, Default)]
    struct OrderCheckingStore {
        last_key_by_revision: Arc<Mutex<HashMap<Uuid, String>>>,
        promote_counts: Arc<Mutex<HashMap<String, usize>>>,
    }

    #[async_trait]
    impl ReportArtifactStore for OrderCheckingStore {
        async fn stage(
            &self,
            revision_id: Uuid,
            format: ReportFormat,
            bytes: &[u8],
        ) -> Result<StagedArtifact, ReportingAppError> {
            let artifact = artifact(format, bytes);
            Ok(StagedArtifact {
                revision_id,
                format,
                staging_key: format!("staging/{revision_id}/{}", artifact.sha256),
                sha256: artifact.sha256,
                byte_len: artifact.byte_len,
            })
        }

        async fn promote(
            &self,
            staged: &StagedArtifact,
        ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError> {
            let extension = match staged.format {
                ReportFormat::Markdown => "md",
                ReportFormat::Json => "json",
            };
            let key = format!("sha256/{}.{extension}", staged.sha256);
            let mut last = self
                .last_key_by_revision
                .lock()
                .expect("lock-order map poisoned");
            if last
                .get(&staged.revision_id)
                .is_some_and(|previous| key <= *previous)
            {
                return Err(ReportingAppError::Artifact(
                    "artifact_reservation_order_not_strict".to_string(),
                ));
            }
            last.insert(staged.revision_id, key.clone());
            drop(last);
            *self
                .promote_counts
                .lock()
                .expect("promotion count map poisoned")
                .entry(key.clone())
                .or_default() += 1;
            Ok(Box::new(TestReservation(ContentAddressedArtifact {
                format: staged.format,
                content_key: key,
                sha256: staged.sha256.clone(),
                byte_len: staged.byte_len,
            })))
        }

        async fn verify(
            &self,
            _artifact: &ContentAddressedArtifact,
        ) -> Result<bool, ReportingAppError> {
            Ok(true)
        }

        async fn discard_staging(&self, _staged: &StagedArtifact) -> Result<(), ReportingAppError> {
            Ok(())
        }

        async fn gc(
            &self,
            _now: DateTime<Utc>,
            _referenced_content_keys: BTreeSet<String>,
        ) -> Result<(), ReportingAppError> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct RecordingPublication(Arc<Mutex<Vec<Vec<ContentAddressedArtifact>>>>);

    #[async_trait]
    impl ReportPublicationPort for RecordingPublication {
        async fn finalize_publication(
            &self,
            command: FinalizePublication,
        ) -> Result<(), ReportingAppError> {
            self.0
                .lock()
                .expect("publication recorder poisoned")
                .push(command.artifacts);
            Ok(())
        }
    }

    fn model(revision_id: Uuid) -> ReportReadModel {
        ReportReadModel {
            report_id: Uuid::new_v4(),
            revision_id,
            operation_id: Uuid::new_v4(),
            project_scope_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            scope_snapshot_hash: "a".repeat(64),
            source_snapshot: ReportSourceSnapshot::freeze("finalizer-order", Vec::new())
                .expect("empty source snapshot"),
            organization_sections: Vec::new(),
            findings: Vec::new(),
            cleanup_residuals: Vec::new(),
            citations: Vec::new(),
        }
    }

    fn request() -> ExplicitFinalizeRequest {
        ExplicitFinalizeRequest {
            principal_id: Uuid::new_v4(),
            confirm_final_publish: true,
            expected_row_version: 0,
            validation_status: ValidationStatus::Validated,
            publication_status: PublicationStatus::Unpublished,
        }
    }

    #[tokio::test]
    async fn duplicate_content_key_is_reserved_once_but_output_keeps_input_shape() {
        let store = OrderCheckingStore::default();
        let publication = RecordingPublication::default();
        let rendered = vec![
            (ReportFormat::Markdown, b"same".to_vec()),
            (ReportFormat::Markdown, b"same".to_vec()),
        ];
        let output = ReportFinalizer::new(store.clone(), publication.clone())
            .finalize(&model(Uuid::new_v4()), request(), rendered)
            .await
            .expect("deduplicated finalization");

        assert_eq!(output, vec![artifact(ReportFormat::Markdown, b"same"); 2]);
        assert_eq!(
            store
                .promote_counts
                .lock()
                .expect("promotion count map poisoned")
                .values()
                .copied()
                .sum::<usize>(),
            1
        );
        let publications = publication.0.lock().expect("publication recorder poisoned");
        assert_eq!(publications.len(), 1);
        assert_eq!(publications[0].len(), 1);
    }

    #[tokio::test]
    async fn concurrent_reverse_inputs_reserve_unique_keys_in_one_stable_order() {
        let store = OrderCheckingStore::default();
        let publication = RecordingPublication::default();
        let mut ascending = vec![
            (ReportFormat::Markdown, b"markdown".to_vec()),
            (ReportFormat::Json, br#"{"json":true}"#.to_vec()),
        ];
        ascending.sort_by_cached_key(|(format, bytes)| content_key(*format, bytes));
        let mut descending = ascending.clone();
        descending.reverse();

        let first = ReportFinalizer::new(store.clone(), publication.clone());
        let second = ReportFinalizer::new(store, publication.clone());
        let expected_first = ascending
            .iter()
            .map(|(format, bytes)| artifact(*format, bytes))
            .collect::<Vec<_>>();
        let expected_second = descending
            .iter()
            .map(|(format, bytes)| artifact(*format, bytes))
            .collect::<Vec<_>>();
        let first_model = model(Uuid::new_v4());
        let second_model = model(Uuid::new_v4());
        let (first_result, second_result) =
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::join!(
                    first.finalize(&first_model, request(), ascending),
                    second.finalize(&second_model, request(), descending),
                )
            })
            .await
            .expect("reverse-order finalizers must not deadlock");
        assert_eq!(
            first_result.expect("ascending finalization"),
            expected_first
        );
        assert_eq!(
            second_result.expect("descending finalization"),
            expected_second
        );
        for artifacts in publication
            .0
            .lock()
            .expect("publication recorder poisoned")
            .iter()
        {
            assert!(artifacts
                .windows(2)
                .all(|pair| pair[0].content_key < pair[1].content_key));
        }
    }
}
