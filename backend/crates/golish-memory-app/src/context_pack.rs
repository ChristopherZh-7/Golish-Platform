use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_memory_domain::{
    ContextItem, ContextRequest, ContextSubject, KnowledgeClass, KnowledgeClassification,
    ProjectScopeId,
};
use uuid::Uuid;

use crate::ports::{AuthorizationSnapshotReader, OperationDataPolicyReader};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationSnapshot {
    pub project_scope_id: ProjectScopeId,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub organization_id: Uuid,
    pub frozen_organization_ids: BTreeSet<Uuid>,
    pub server_now: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerDataPolicy {
    pub principal_id: Uuid,
    pub allowed_classes: BTreeSet<KnowledgeClass>,
    pub classification_ceiling: KnowledgeClassification,
    pub allow_external_embedding: bool,
    pub server_token_cap: usize,
}

impl ServerDataPolicy {
    pub fn customer_local_only(principal_id: Uuid) -> Self {
        Self {
            principal_id,
            allowed_classes: KnowledgeClass::ALL.into_iter().collect(),
            classification_ceiling: KnowledgeClassification::Restricted,
            allow_external_embedding: false,
            server_token_cap: golish_memory_domain::DEFAULT_CONTEXT_TOKEN_CAP,
        }
    }
}

/// Opaque authorization result. Its fields are deliberately private and the
/// type has no public constructor or deserializer. Only the loader in this
/// crate can combine DB ownership and server policy into this capability.
#[derive(Clone, Debug)]
pub struct TrustedAuthorizationContext {
    principal_id: Uuid,
    project_scope_id: ProjectScopeId,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Option<Uuid>,
    stage_kind: String,
    wave: Option<i32>,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
    organization_id: Uuid,
    frozen_organization_ids: BTreeSet<Uuid>,
    classification_ceiling: KnowledgeClassification,
    allowed_classes: BTreeSet<KnowledgeClass>,
    allow_external_embedding: bool,
    server_token_cap: usize,
    server_now: DateTime<Utc>,
}

impl TrustedAuthorizationContext {
    pub const fn principal_id(&self) -> Uuid {
        self.principal_id
    }

    pub const fn project_scope_id(&self) -> ProjectScopeId {
        self.project_scope_id
    }

    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub const fn stage_execution_id(&self) -> Uuid {
        self.stage_execution_id
    }

    pub const fn stage_run_unit_id(&self) -> Uuid {
        self.stage_run_unit_id
    }

    pub const fn worker_run_id(&self) -> Option<Uuid> {
        self.worker_run_id
    }

    pub fn stage_kind(&self) -> &str {
        &self.stage_kind
    }

    pub const fn wave(&self) -> Option<i32> {
        self.wave
    }

    pub const fn scope_snapshot_id(&self) -> Uuid {
        self.scope_snapshot_id
    }

    pub fn scope_snapshot_hash(&self) -> &str {
        &self.scope_snapshot_hash
    }

    pub const fn organization_id(&self) -> Uuid {
        self.organization_id
    }

    pub fn frozen_organization_ids(&self) -> &BTreeSet<Uuid> {
        &self.frozen_organization_ids
    }

    pub const fn classification_ceiling(&self) -> KnowledgeClassification {
        self.classification_ceiling
    }

    pub fn allowed_classes(&self) -> &BTreeSet<KnowledgeClass> {
        &self.allowed_classes
    }

    pub const fn allows_external_embedding(&self) -> bool {
        self.allow_external_embedding
    }

    pub const fn server_token_cap(&self) -> usize {
        self.server_token_cap
    }

    pub fn server_now(&self) -> DateTime<Utc> {
        self.server_now.to_owned()
    }
}

pub(crate) struct TrustedAuthorizationContextLoader {
    authorization: Arc<dyn AuthorizationSnapshotReader>,
    policy: Arc<dyn OperationDataPolicyReader>,
}

impl TrustedAuthorizationContextLoader {
    pub(crate) fn new(
        authorization: Arc<dyn AuthorizationSnapshotReader>,
        policy: Arc<dyn OperationDataPolicyReader>,
    ) -> Self {
        Self {
            authorization,
            policy,
        }
    }

    pub(crate) async fn load(
        &self,
        subject: &ContextSubject,
    ) -> Result<TrustedAuthorizationContext, ContextError> {
        let snapshot = self.authorization.load(subject).await?;
        let policy = self.policy.resolve(subject, &snapshot).await?;
        if snapshot.operation_id != subject.operation_id()
            || snapshot.organization_id != subject.organization_id()
            || !snapshot
                .frozen_organization_ids
                .contains(&subject.organization_id())
            || snapshot.scope_snapshot_hash.trim().is_empty()
            || policy.allowed_classes.is_empty()
            || policy.server_token_cap == 0
        {
            return Err(ContextError::AuthorizationSnapshotMismatch);
        }
        Ok(TrustedAuthorizationContext {
            principal_id: policy.principal_id,
            project_scope_id: snapshot.project_scope_id,
            operation_id: snapshot.operation_id,
            stage_execution_id: subject.stage_execution_id(),
            stage_run_unit_id: subject.stage_run_unit_id(),
            worker_run_id: subject.worker_run_id(),
            stage_kind: subject.stage_kind().to_string(),
            wave: subject.wave(),
            scope_snapshot_id: snapshot.scope_snapshot_id,
            scope_snapshot_hash: snapshot.scope_snapshot_hash,
            organization_id: snapshot.organization_id,
            frozen_organization_ids: snapshot.frozen_organization_ids,
            classification_ceiling: policy.classification_ceiling,
            allowed_classes: policy.allowed_classes,
            allow_external_embedding: policy.allow_external_embedding,
            server_token_cap: policy.server_token_cap,
            server_now: snapshot.server_now,
        })
    }
}

#[derive(Clone, Debug)]
pub struct EffectiveContextQuery {
    trusted: TrustedAuthorizationContext,
    request: ContextRequest,
    allowed_classes: BTreeSet<KnowledgeClass>,
    token_budget: usize,
}

impl EffectiveContextQuery {
    pub fn intersect(
        trusted: TrustedAuthorizationContext,
        request: ContextRequest,
        stage: &str,
    ) -> Result<Self, ContextError> {
        request
            .validate()
            .map_err(|error| ContextError::InvalidRequest(error.code()))?;
        let stage_classes = classes_for_stage(stage);
        let allowed_classes = request
            .requested_classes
            .intersection(trusted.allowed_classes())
            .copied()
            .collect::<BTreeSet<_>>()
            .intersection(&stage_classes)
            .copied()
            .collect();
        let token_budget = request
            .requested_token_budget
            .min(trusted.server_token_cap());
        Ok(Self {
            trusted,
            request,
            allowed_classes,
            token_budget,
        })
    }

    pub fn trusted(&self) -> &TrustedAuthorizationContext {
        &self.trusted
    }

    pub fn request(&self) -> &ContextRequest {
        &self.request
    }

    pub fn allowed_classes(&self) -> &BTreeSet<KnowledgeClass> {
        &self.allowed_classes
    }

    pub const fn token_budget(&self) -> usize {
        self.token_budget
    }
}

pub(crate) fn classes_for_stage(stage: &str) -> BTreeSet<KnowledgeClass> {
    match stage {
        "reporting" => [
            KnowledgeClass::CanonicalFact,
            KnowledgeClass::RuntimeState,
            KnowledgeClass::PassedHandoff,
            KnowledgeClass::StageEpisode,
        ]
        .into_iter()
        .collect(),
        _ => KnowledgeClass::ALL.into_iter().collect(),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContextOmissionSummary {
    pub omitted_count: usize,
    pub reasons: Vec<String>,
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextPack {
    pub canonical_items: Vec<ContextItem>,
    pub runtime_items: Vec<ContextItem>,
    pub handoff_items: Vec<ContextItem>,
    pub episode_items: Vec<ContextItem>,
    pub assertion_items: Vec<ContextItem>,
    pub document_items: Vec<ContextItem>,
    pub graph_items: Vec<ContextItem>,
    pub vector_items: Vec<ContextItem>,
    pub omitted: ContextOmissionSummary,
}

impl ContextPack {
    pub fn items(&self) -> impl Iterator<Item = &ContextItem> {
        self.canonical_items
            .iter()
            .chain(&self.runtime_items)
            .chain(&self.handoff_items)
            .chain(&self.episode_items)
            .chain(&self.assertion_items)
            .chain(&self.document_items)
            .chain(&self.graph_items)
            .chain(&self.vector_items)
    }
}

#[async_trait]
pub trait ContextPackProvider: Send + Sync {
    async fn retrieve(
        &self,
        subject: ContextSubject,
        request: ContextRequest,
    ) -> Result<ContextPack, ContextError>;
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ContextError {
    #[error("context request rejected: {0}")]
    InvalidRequest(&'static str),
    #[error("authorization snapshot does not match the server runtime subject")]
    AuthorizationSnapshotMismatch,
    #[error("context source failed: {0}")]
    Source(String),
    #[error("context item violates scope, classification, validity, or integrity")]
    InvalidItem,
    #[error(
        "mandatory context exceeds server token cap: required {required_tokens}, cap {server_cap}"
    )]
    MandatoryContextTooLarge {
        required_tokens: usize,
        server_cap: usize,
    },
}

impl ContextError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "knowledge_context_request_invalid",
            Self::AuthorizationSnapshotMismatch => "knowledge_context_authorization_mismatch",
            Self::Source(_) => "knowledge_context_source_failed",
            Self::InvalidItem => "knowledge_context_item_rejected",
            Self::MandatoryContextTooLarge { .. } => "knowledge_context_mandatory_too_large",
        }
    }
}
