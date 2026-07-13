use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_memory_domain::{
    validate_embedding_dimension, KnowledgeClassification, EMBEDDING_DIMENSION_V1,
};
use uuid::Uuid;

use crate::ports::EmbeddingProjectionPort;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn dimension(&self) -> usize;
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentDeliveryStatus {
    Succeeded,
    SucceededSuppressed,
    NotTerminal,
}

#[derive(Clone, Debug)]
pub struct EmbeddingDocument {
    pub document_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub redacted_content: String,
    pub content_hash: String,
    pub classification: KnowledgeClassification,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct DocumentDeliverySnapshot {
    pub status: DocumentDeliveryStatus,
    pub documents: Vec<EmbeddingDocument>,
}

#[derive(Clone, Debug)]
pub struct ProjectedEmbedding {
    pub embedding_id: Uuid,
    pub document_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub provider: String,
    pub model: String,
    pub embedding: Vec<f32>,
    pub content_hash: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmbeddingProjectionOutcome {
    Succeeded { embedding_ids: Vec<Uuid> },
    SucceededSuppressed { reason_code: String },
}

pub struct EmbeddingProjector {
    provider: Arc<dyn EmbeddingProvider>,
}

impl std::fmt::Debug for EmbeddingProjector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbeddingProjector")
            .field("provider", &self.provider.provider_name())
            .field("model", &self.provider.model_name())
            .finish()
    }
}

impl EmbeddingProjector {
    pub fn new(provider: Arc<dyn EmbeddingProvider>) -> Result<Self, EmbeddingProjectionError> {
        if provider.provider_name().trim().is_empty() || provider.model_name().trim().is_empty() {
            return Err(EmbeddingProjectionError::ProviderIdentityInvalid);
        }
        validate_embedding_dimension(provider.dimension())
            .map_err(|_| EmbeddingProjectionError::DimensionMismatch)?;
        Ok(Self { provider })
    }

    pub async fn project<P: EmbeddingProjectionPort + ?Sized>(
        &self,
        port: &P,
        event_id: Uuid,
    ) -> Result<EmbeddingProjectionOutcome, EmbeddingProjectionError> {
        let snapshot = port.load_document_delivery(event_id).await?;
        if snapshot.status != DocumentDeliveryStatus::Succeeded {
            return Ok(EmbeddingProjectionOutcome::SucceededSuppressed {
                reason_code: "knowledge_document_delivery_not_succeeded".to_string(),
            });
        }
        if snapshot.documents.is_empty() {
            return Err(EmbeddingProjectionError::DocumentMissing);
        }
        if snapshot
            .documents
            .iter()
            .any(|document| document.classification == KnowledgeClassification::Restricted)
        {
            return Ok(EmbeddingProjectionOutcome::SucceededSuppressed {
                reason_code: "knowledge_embedding_restricted_classification".to_string(),
            });
        }

        let mut embedding_ids = Vec::with_capacity(snapshot.documents.len());
        for document in snapshot.documents {
            let embedding = self
                .provider
                .embed(&document.redacted_content)
                .await
                .map_err(EmbeddingProjectionError::Provider)?;
            if embedding.len() != EMBEDDING_DIMENSION_V1
                || embedding.iter().any(|value| !value.is_finite())
            {
                return Err(EmbeddingProjectionError::ProviderResultInvalid);
            }
            let identity = format!(
                "{}\0{}\0{}\0{}",
                document.document_id,
                self.provider.provider_name(),
                self.provider.model_name(),
                document.content_hash
            );
            let projected = ProjectedEmbedding {
                embedding_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes()),
                document_id: document.document_id,
                source_stream_key: document.source_stream_key,
                source_version: document.source_version,
                provider: self.provider.provider_name().to_string(),
                model: self.provider.model_name().to_string(),
                embedding,
                content_hash: document.content_hash,
                valid_from: document.valid_from,
                valid_to: document.valid_to,
            };
            embedding_ids.push(port.store_embedding(projected).await?);
        }
        Ok(EmbeddingProjectionOutcome::Succeeded { embedding_ids })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EmbeddingProjectionError {
    #[error("embedding provider dimension must equal 1536")]
    DimensionMismatch,
    #[error("embedding provider identity is invalid")]
    ProviderIdentityInvalid,
    #[error("document projector succeeded but no active document exists")]
    DocumentMissing,
    #[error("embedding provider failed: {0}")]
    Provider(String),
    #[error("embedding provider returned a non-1536 or non-finite vector")]
    ProviderResultInvalid,
    #[error("embedding projection port failed: {0}")]
    Port(String),
}

impl EmbeddingProjectionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DimensionMismatch => "embedding_dimension_mismatch",
            Self::ProviderIdentityInvalid => "embedding_provider_identity_invalid",
            Self::DocumentMissing => "embedding_document_missing",
            Self::Provider(_) => "embedding_provider_failed",
            Self::ProviderResultInvalid => "embedding_provider_result_invalid",
            Self::Port(_) => "embedding_projection_port_failed",
        }
    }
}
