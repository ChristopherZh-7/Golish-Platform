use std::sync::Mutex;

use async_trait::async_trait;
use golish_memory_app::{
    ports::{DocumentProjectionPort, MemoryError},
    projectors::document::{DocumentProjector, ProjectedDocument},
};
use golish_memory_domain::{
    assertion::{AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion},
    classification::{AssertionVisibility, KnowledgeClassification},
    scope::ProjectScopeId,
    source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef},
};
use uuid::Uuid;

#[derive(Default)]
struct RecordingPort {
    assertions: Vec<KnowledgeAssertion>,
    written: Mutex<Vec<ProjectedDocument>>,
}

#[async_trait]
impl DocumentProjectionPort for RecordingPort {
    async fn load_promoted_assertions(
        &self,
        _event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        Ok(self.assertions.clone())
    }

    async fn upsert_document(&self, document: ProjectedDocument) -> Result<Uuid, MemoryError> {
        let id = document.document_id;
        self.written.lock().expect("document writes").push(document);
        Ok(id)
    }
}

fn assertion(predicate: &str, value: serde_json::Value) -> KnowledgeAssertion {
    KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "target:example.com",
        predicate,
        AssertionObject::Json(value),
        AssertionKind::Observation,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Int64(42),
            source_stream_key: "fact-delta:42".to_string(),
            version: 1,
        },
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("valid fixture assertion")
}

#[tokio::test]
async fn projection_is_deterministic_across_assertion_order() {
    let event_id = Uuid::from_u128(9);
    let first = assertion("http.status", serde_json::json!(200));
    let second = assertion(
        "service.name",
        serde_json::json!({"name": "https", "port": 443}),
    );
    let projector = DocumentProjector::new(3);

    let port_a = RecordingPort {
        assertions: vec![first.clone(), second.clone()],
        ..RecordingPort::default()
    };
    let port_b = RecordingPort {
        assertions: vec![second, first],
        ..RecordingPort::default()
    };

    projector
        .project(&port_a, event_id)
        .await
        .expect("first projection");
    projector
        .project(&port_b, event_id)
        .await
        .expect("second projection");

    let a = port_a.written.lock().expect("first writes")[0].clone();
    let b = port_b.written.lock().expect("second writes")[0].clone();
    assert_eq!(a.document_id, b.document_id);
    assert_eq!(a.document_key, b.document_key);
    assert_eq!(a.redacted_content, b.redacted_content);
    assert_eq!(a.content_hash, b.content_hash);
}
