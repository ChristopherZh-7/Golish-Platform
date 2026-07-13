use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use golish_db::repo::knowledge_graph::{
    self, AssertionLineageWrite, EntityIdentityWrite, GraphScopeRecord, LineageWriteDisposition,
    RelationIdentityWrite,
};
use golish_db::PgPool;
use golish_memory_domain::classification::KnowledgeClassification;
use golish_memory_domain::scope::ProjectScopeId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::GraphError;

pub const TEMPORAL_GRAPH_SCHEMA_V1: i32 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphVisibility {
    OrganizationLongTerm,
    GlobalSanitized,
}

impl GraphVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrganizationLongTerm => "organization_long_term",
            Self::GlobalSanitized => "global_sanitized",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphScopeKey(String);

impl GraphScopeKey {
    pub fn organization(project_scope_id: ProjectScopeId, organization_id_at_time: Uuid) -> Self {
        Self(format!(
            "org:{}:{}",
            project_scope_id.0, organization_id_at_time
        ))
    }

    pub fn global_sanitized() -> Self {
        Self("global_sanitized".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalEntityType {
    Organization,
    Target,
    Host,
    Service,
    Endpoint,
    Vulnerability,
    Finding,
    Technique,
}

impl TemporalEntityType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Target => "target",
            Self::Host => "host",
            Self::Service => "service",
            Self::Endpoint => "endpoint",
            Self::Vulnerability => "vulnerability",
            Self::Finding => "finding",
            Self::Technique => "technique",
        }
    }

    fn parse(value: &str) -> Result<Self, GraphError> {
        match value {
            "organization" => Ok(Self::Organization),
            "target" => Ok(Self::Target),
            "host" => Ok(Self::Host),
            "service" => Ok(Self::Service),
            "endpoint" => Ok(Self::Endpoint),
            "vulnerability" => Ok(Self::Vulnerability),
            "finding" => Ok(Self::Finding),
            "technique" => Ok(Self::Technique),
            other => Err(GraphError::UnknownEntityType(other.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRelationType {
    Contains,
    ResolvesTo,
    RunsService,
    ExposesEndpoint,
    HasVulnerability,
    SupportedByFinding,
    AssociatedTechnique,
}

impl TemporalRelationType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::ResolvesTo => "resolves_to",
            Self::RunsService => "runs_service",
            Self::ExposesEndpoint => "exposes_endpoint",
            Self::HasVulnerability => "has_vulnerability",
            Self::SupportedByFinding => "supported_by_finding",
            Self::AssociatedTechnique => "associated_technique",
        }
    }

    fn parse(value: &str) -> Result<Self, GraphError> {
        match value {
            "contains" => Ok(Self::Contains),
            "resolves_to" => Ok(Self::ResolvesTo),
            "runs_service" => Ok(Self::RunsService),
            "exposes_endpoint" => Ok(Self::ExposesEndpoint),
            "has_vulnerability" => Ok(Self::HasVulnerability),
            "supported_by_finding" => Ok(Self::SupportedByFinding),
            "associated_technique" => Ok(Self::AssociatedTechnique),
            other => Err(GraphError::UnknownRelationType(other.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalEntityProjection {
    pub scope_key: GraphScopeKey,
    pub visibility: GraphVisibility,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub canonical_ref: String,
    pub identity_hash: String,
    pub entity_type: TemporalEntityType,
    pub display_name: String,
    pub properties: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalLineageProjection {
    pub canonical_ref: String,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: KnowledgeClassification,
    pub projection_schema_version: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalRelationProjection {
    pub scope_key: GraphScopeKey,
    pub from_canonical_ref: String,
    pub to_canonical_ref: String,
    pub relation_type: TemporalRelationType,
    pub identity_hash: String,
    pub properties: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalRelationLineageProjection {
    pub from_canonical_ref: String,
    pub to_canonical_ref: String,
    pub relation_type: TemporalRelationType,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: KnowledgeClassification,
    pub projection_schema_version: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphProjection {
    pub entities: Vec<TemporalEntityProjection>,
    pub entity_lineages: Vec<TemporalLineageProjection>,
    pub relations: Vec<TemporalRelationProjection>,
    pub relation_lineages: Vec<TemporalRelationLineageProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphInvalidation {
    pub close_assertion_id: Uuid,
    pub valid_to: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionWriteDisposition {
    Applied,
    ExactReplay,
    Stale { current_version: i64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyProjectionResult {
    pub disposition: ProjectionWriteDisposition,
    pub generation_id: Uuid,
    pub entity_count: usize,
    pub relation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphGeneration {
    pub generation_id: Uuid,
    pub scope_key: GraphScopeKey,
    pub projection_schema_version: i32,
    pub status: String,
    pub build_hash: Option<String>,
    pub entity_count: Option<i64>,
    pub relation_count: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebuildAttestation {
    pub build_hash: String,
    pub entity_count: i64,
    pub relation_count: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalLineageFact {
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: KnowledgeClassification,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphFact {
    pub generation_id: Uuid,
    pub entity_id: Uuid,
    pub scope_key: GraphScopeKey,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub canonical_ref: String,
    pub entity_type: TemporalEntityType,
    pub display_name: String,
    pub properties: Value,
    pub lineages: Vec<TemporalLineageFact>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphRelationFact {
    pub generation_id: Uuid,
    pub relation_id: Uuid,
    pub scope_key: GraphScopeKey,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub from_entity_id: Uuid,
    pub from_canonical_ref: String,
    pub from_entity_type: TemporalEntityType,
    pub from_display_name: String,
    pub to_entity_id: Uuid,
    pub to_canonical_ref: String,
    pub to_entity_type: TemporalEntityType,
    pub to_display_name: String,
    pub relation_type: TemporalRelationType,
    pub properties: Value,
    pub lineages: Vec<TemporalLineageFact>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphQueryResult {
    pub entities: Vec<TemporalGraphFact>,
    pub relations: Vec<TemporalGraphRelationFact>,
}

#[derive(Clone, Debug)]
pub struct ScopedGraphQuery {
    scope: GraphScopeRecord,
    pub query: String,
    pub classification_ceiling: KnowledgeClassification,
    pub valid_at: DateTime<Utc>,
    pub projection_schema_version: i32,
    pub limit: i64,
}

impl ScopedGraphQuery {
    pub fn for_organization(
        project_scope_id: ProjectScopeId,
        organization_id_at_time: Uuid,
        query: impl Into<String>,
        valid_at: DateTime<Utc>,
    ) -> Self {
        Self {
            scope: GraphScopeRecord::organization(project_scope_id.0, organization_id_at_time),
            query: query.into(),
            classification_ceiling: KnowledgeClassification::Restricted,
            valid_at,
            projection_schema_version: TEMPORAL_GRAPH_SCHEMA_V1,
            limit: 100,
        }
    }

    pub fn global_sanitized(query: impl Into<String>, valid_at: DateTime<Utc>) -> Self {
        Self {
            scope: GraphScopeRecord::global_sanitized(),
            query: query.into(),
            classification_ceiling: KnowledgeClassification::Internal,
            valid_at,
            projection_schema_version: TEMPORAL_GRAPH_SCHEMA_V1,
            limit: 100,
        }
    }

    pub fn with_classification_ceiling(mut self, ceiling: KnowledgeClassification) -> Self {
        self.classification_ceiling = ceiling;
        self
    }

    pub fn scope_key(&self) -> &str {
        &self.scope.scope_key
    }
}

#[derive(Debug, Clone)]
pub struct TemporalGraphClient {
    pool: PgPool,
}

impl TemporalGraphClient {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn ensure_active_generation(
        &self,
        scope: &TemporalEntityProjection,
        projection_schema_version: i32,
    ) -> Result<GraphGeneration, GraphError> {
        let record = scope_record(scope)?;
        Ok(generation_view(
            knowledge_graph::ensure_active_generation(
                &self.pool,
                &record,
                projection_schema_version,
            )
            .await?,
        ))
    }

    pub async fn apply_projection(
        &self,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, GraphError> {
        let first = projection
            .entities
            .first()
            .ok_or_else(|| GraphError::InvalidArgument("empty temporal projection".to_string()))?;
        let scope = scope_record(first)?;
        let generation =
            knowledge_graph::ensure_active_generation(&self.pool, &scope, TEMPORAL_GRAPH_SCHEMA_V1)
                .await?;
        self.apply_projection_to_generation(generation.generation_id, projection)
            .await
    }

    pub async fn apply_projection_to_generation(
        &self,
        generation_id: Uuid,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, GraphError> {
        let first_entity = projection
            .entities
            .first()
            .ok_or_else(|| GraphError::InvalidArgument("empty temporal projection".to_string()))?;
        let scope = scope_record(first_entity)?;
        let mut tx = self.pool.begin().await?;
        let generation = knowledge_graph::lock_writable_generation(&mut tx, generation_id).await?;
        if generation.scope_key != scope.scope_key
            || generation.project_scope_id != scope.project_scope_id
            || generation.organization_id_at_time != scope.organization_id_at_time
        {
            return Err(GraphError::InvalidArgument(
                "temporal projection scope does not match generation".to_string(),
            ));
        }
        if projection.entities.iter().any(|entity| {
            entity.scope_key.as_str() != generation.scope_key || scope_record(entity).is_err()
        }) || projection
            .relations
            .iter()
            .any(|relation| relation.scope_key.as_str() != generation.scope_key)
        {
            return Err(GraphError::InvalidArgument(
                "mixed temporal projection scope".to_string(),
            ));
        }
        let (source_stream_key, source_version, projection_schema_version) =
            validate_projection_shape(projection)?;

        let current_version = knowledge_graph::max_source_version(
            &mut tx,
            generation_id,
            &source_stream_key,
            projection_schema_version,
        )
        .await?;
        if current_version.is_some_and(|version| version > source_version) {
            tx.rollback().await?;
            return Ok(ApplyProjectionResult {
                disposition: ProjectionWriteDisposition::Stale {
                    current_version: current_version.expect("checked some"),
                },
                generation_id,
                entity_count: 0,
                relation_count: 0,
            });
        }

        let mut entity_ids = HashMap::new();
        for entity in &projection.entities {
            let entity_id = deterministic_entity_id(generation_id, &entity.canonical_ref);
            knowledge_graph::upsert_entity_identity(
                &mut tx,
                &EntityIdentityWrite {
                    entity_id,
                    generation_id,
                    scope: scope.clone(),
                    canonical_ref: entity.canonical_ref.clone(),
                    identity_hash: entity.identity_hash.clone(),
                    entity_type: entity.entity_type.as_str().to_string(),
                    display_name: entity.display_name.clone(),
                    properties: entity.properties.clone(),
                },
            )
            .await?;
            entity_ids.insert(entity.canonical_ref.clone(), entity_id);
        }

        let mut replay_only = true;
        for lineage in &projection.entity_lineages {
            let entity_id = *entity_ids.get(&lineage.canonical_ref).ok_or_else(|| {
                GraphError::InvalidArgument("entity lineage has no identity".to_string())
            })?;
            let (_, disposition) = knowledge_graph::attach_entity_assertion(
                &mut tx,
                entity_id,
                generation_id,
                &lineage_write(lineage),
            )
            .await?;
            replay_only &= disposition == LineageWriteDisposition::ExactReplay;
        }

        let mut relation_ids = HashMap::new();
        for relation in &projection.relations {
            let from_id = *entity_ids
                .get(&relation.from_canonical_ref)
                .ok_or_else(|| {
                    GraphError::InvalidArgument("relation source identity missing".to_string())
                })?;
            let to_id = *entity_ids.get(&relation.to_canonical_ref).ok_or_else(|| {
                GraphError::InvalidArgument("relation target identity missing".to_string())
            })?;
            let relation_id = deterministic_relation_id(
                generation_id,
                &relation.from_canonical_ref,
                &relation.to_canonical_ref,
                relation.relation_type,
            );
            knowledge_graph::upsert_relation_identity(
                &mut tx,
                &RelationIdentityWrite {
                    relation_id,
                    generation_id,
                    scope_key: generation.scope_key.clone(),
                    from_entity_id: from_id,
                    to_entity_id: to_id,
                    relation_type: relation.relation_type.as_str().to_string(),
                    identity_hash: relation.identity_hash.clone(),
                    properties: relation.properties.clone(),
                },
            )
            .await?;
            relation_ids.insert(
                (
                    relation.from_canonical_ref.clone(),
                    relation.to_canonical_ref.clone(),
                    relation.relation_type,
                ),
                relation_id,
            );
        }
        for lineage in &projection.relation_lineages {
            let relation_id = *relation_ids
                .get(&(
                    lineage.from_canonical_ref.clone(),
                    lineage.to_canonical_ref.clone(),
                    lineage.relation_type,
                ))
                .ok_or_else(|| {
                    GraphError::InvalidArgument("relation lineage has no identity".to_string())
                })?;
            let (_, disposition) = knowledge_graph::attach_relation_assertion(
                &mut tx,
                relation_id,
                generation_id,
                &relation_lineage_write(lineage),
            )
            .await?;
            replay_only &= disposition == LineageWriteDisposition::ExactReplay;
        }
        tx.commit().await?;
        Ok(ApplyProjectionResult {
            disposition: if replay_only {
                ProjectionWriteDisposition::ExactReplay
            } else {
                ProjectionWriteDisposition::Applied
            },
            generation_id,
            entity_count: projection.entities.len(),
            relation_count: projection.relations.len(),
        })
    }

    pub async fn close_assertion_lineage(
        &self,
        invalidation: &TemporalGraphInvalidation,
    ) -> Result<(u64, u64), GraphError> {
        Ok(knowledge_graph::close_assertion_lineage(
            &self.pool,
            invalidation.close_assertion_id,
            invalidation.valid_to,
        )
        .await?)
    }

    pub async fn query(
        &self,
        query: ScopedGraphQuery,
    ) -> Result<TemporalGraphQueryResult, GraphError> {
        let entity_rows = knowledge_graph::query_active_entities(
            &self.pool,
            &query.scope,
            query.projection_schema_version,
            &query.query,
            query.classification_ceiling.as_str(),
            query.valid_at,
            query.limit,
        )
        .await?;
        let relation_rows = knowledge_graph::query_active_relations(
            &self.pool,
            &query.scope,
            query.projection_schema_version,
            &query.query,
            query.classification_ceiling.as_str(),
            query.valid_at,
            query.limit,
        )
        .await?;
        let mut entities: BTreeMap<Uuid, TemporalGraphFact> = BTreeMap::new();
        for row in entity_rows {
            let classification = parse_classification(&row.classification)?;
            let fact = entities
                .entry(row.entity_id)
                .or_insert_with(|| TemporalGraphFact {
                    generation_id: row.generation_id,
                    entity_id: row.entity_id,
                    scope_key: GraphScopeKey(row.scope_key.clone()),
                    project_scope_id: row.project_scope_id.map(ProjectScopeId),
                    organization_id_at_time: row.organization_id_at_time,
                    canonical_ref: row.canonical_ref.clone(),
                    entity_type: TemporalEntityType::parse(&row.entity_type)
                        .expect("database entity CHECK is closed"),
                    display_name: row.display_name.clone(),
                    properties: row.properties.clone(),
                    lineages: Vec::new(),
                });
            fact.lineages.push(TemporalLineageFact {
                assertion_id: row.assertion_id,
                source_stream_key: row.source_stream_key,
                source_version: row.source_version,
                evidence_refs: row.evidence_refs,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
                fresh_until: row.fresh_until,
                classification,
            });
        }
        let mut relations: BTreeMap<Uuid, TemporalGraphRelationFact> = BTreeMap::new();
        for row in relation_rows {
            let classification = parse_classification(&row.classification)?;
            let fact =
                relations
                    .entry(row.relation_id)
                    .or_insert_with(|| TemporalGraphRelationFact {
                        generation_id: row.generation_id,
                        relation_id: row.relation_id,
                        scope_key: GraphScopeKey(row.scope_key.clone()),
                        project_scope_id: row.project_scope_id.map(ProjectScopeId),
                        organization_id_at_time: row.organization_id_at_time,
                        from_entity_id: row.from_entity_id,
                        from_canonical_ref: row.from_canonical_ref.clone(),
                        from_entity_type: TemporalEntityType::parse(&row.from_entity_type)
                            .expect("database source entity CHECK is closed"),
                        from_display_name: row.from_display_name.clone(),
                        to_entity_id: row.to_entity_id,
                        to_canonical_ref: row.to_canonical_ref.clone(),
                        to_entity_type: TemporalEntityType::parse(&row.to_entity_type)
                            .expect("database target entity CHECK is closed"),
                        to_display_name: row.to_display_name.clone(),
                        relation_type: TemporalRelationType::parse(&row.relation_type)
                            .expect("database relation CHECK is closed"),
                        properties: row.properties.clone(),
                        lineages: Vec::new(),
                    });
            fact.lineages.push(TemporalLineageFact {
                assertion_id: row.assertion_id,
                source_stream_key: row.source_stream_key,
                source_version: row.source_version,
                evidence_refs: row.evidence_refs,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
                fresh_until: row.fresh_until,
                classification,
            });
        }
        Ok(TemporalGraphQueryResult {
            entities: entities.into_values().collect(),
            relations: relations.into_values().collect(),
        })
    }

    pub async fn begin_rebuild(
        &self,
        scope_key: &GraphScopeKey,
        project_scope_id: Option<ProjectScopeId>,
        organization_id_at_time: Option<Uuid>,
        projection_schema_version: i32,
    ) -> Result<GraphGeneration, GraphError> {
        let scope = scope_from_parts(scope_key, project_scope_id, organization_id_at_time)?;
        Ok(generation_view(
            knowledge_graph::begin_rebuild_generation(
                &self.pool,
                &scope,
                projection_schema_version,
            )
            .await?,
        ))
    }

    pub async fn activate_rebuild(
        &self,
        generation_id: Uuid,
        attestation: &RebuildAttestation,
    ) -> Result<GraphGeneration, GraphError> {
        Ok(generation_view(
            knowledge_graph::activate_rebuild_generation(
                &self.pool,
                generation_id,
                &attestation.build_hash,
                attestation.entity_count,
                attestation.relation_count,
            )
            .await?,
        ))
    }

    pub async fn generation_attestation(
        &self,
        generation_id: Uuid,
    ) -> Result<RebuildAttestation, GraphError> {
        let row = knowledge_graph::generation_attestation(&self.pool, generation_id).await?;
        Ok(RebuildAttestation {
            build_hash: row.build_hash,
            entity_count: row.entity_count,
            relation_count: row.relation_count,
        })
    }

    pub async fn fail_rebuild(&self, generation_id: Uuid, reason: &str) -> Result<(), GraphError> {
        Ok(knowledge_graph::fail_rebuild_generation(&self.pool, generation_id, reason).await?)
    }

    pub async fn discard_rebuild(&self, generation_id: Uuid) -> Result<(), GraphError> {
        Ok(knowledge_graph::discard_failed_generation(&self.pool, generation_id).await?)
    }

    pub async fn active_entity_lineage_count(
        &self,
        entity_id: Uuid,
        valid_at: DateTime<Utc>,
    ) -> Result<i64, GraphError> {
        Ok(knowledge_graph::active_entity_lineage_count(&self.pool, entity_id, valid_at).await?)
    }

    pub async fn active_relation_lineage_count(
        &self,
        relation_id: Uuid,
        valid_at: DateTime<Utc>,
    ) -> Result<i64, GraphError> {
        Ok(
            knowledge_graph::active_relation_lineage_count(&self.pool, relation_id, valid_at)
                .await?,
        )
    }
}

fn validate_projection_shape(
    projection: &TemporalGraphProjection,
) -> Result<(String, i64, i32), GraphError> {
    let first = projection
        .entity_lineages
        .first()
        .map(|lineage| {
            (
                lineage.source_stream_key.clone(),
                lineage.source_version,
                lineage.projection_schema_version,
            )
        })
        .or_else(|| {
            projection.relation_lineages.first().map(|lineage| {
                (
                    lineage.source_stream_key.clone(),
                    lineage.source_version,
                    lineage.projection_schema_version,
                )
            })
        })
        .ok_or_else(|| GraphError::InvalidArgument("projection has no lineage".to_string()))?;
    if first.1 <= 0 || first.2 <= 0 || first.0.trim().is_empty() {
        return Err(GraphError::InvalidArgument(
            "projection lineage source tuple is invalid".to_string(),
        ));
    }
    if projection.entity_lineages.iter().any(|lineage| {
        lineage.source_stream_key != first.0
            || lineage.source_version != first.1
            || lineage.projection_schema_version != first.2
    }) || projection.relation_lineages.iter().any(|lineage| {
        lineage.source_stream_key != first.0
            || lineage.source_version != first.1
            || lineage.projection_schema_version != first.2
    }) {
        return Err(GraphError::InvalidArgument(
            "mixed projection lineage source tuple".to_string(),
        ));
    }

    let entity_refs: HashSet<&str> = projection
        .entities
        .iter()
        .map(|entity| entity.canonical_ref.as_str())
        .collect();
    if entity_refs.len() != projection.entities.len()
        || projection
            .entity_lineages
            .iter()
            .any(|lineage| !entity_refs.contains(lineage.canonical_ref.as_str()))
        || entity_refs.iter().any(|canonical_ref| {
            !projection
                .entity_lineages
                .iter()
                .any(|lineage| lineage.canonical_ref == *canonical_ref)
        })
    {
        return Err(GraphError::InvalidArgument(
            "projection entity lineage shape is invalid".to_string(),
        ));
    }

    let relation_keys: HashSet<(&str, &str, TemporalRelationType)> = projection
        .relations
        .iter()
        .map(|relation| {
            (
                relation.from_canonical_ref.as_str(),
                relation.to_canonical_ref.as_str(),
                relation.relation_type,
            )
        })
        .collect();
    if relation_keys.len() != projection.relations.len()
        || projection.relation_lineages.iter().any(|lineage| {
            !relation_keys.contains(&(
                lineage.from_canonical_ref.as_str(),
                lineage.to_canonical_ref.as_str(),
                lineage.relation_type,
            ))
        })
        || relation_keys.iter().any(|relation_key| {
            !projection.relation_lineages.iter().any(|lineage| {
                (
                    lineage.from_canonical_ref.as_str(),
                    lineage.to_canonical_ref.as_str(),
                    lineage.relation_type,
                ) == *relation_key
            })
        })
    {
        return Err(GraphError::InvalidArgument(
            "projection relation lineage shape is invalid".to_string(),
        ));
    }
    Ok(first)
}

fn scope_record(entity: &TemporalEntityProjection) -> Result<GraphScopeRecord, GraphError> {
    scope_from_parts(
        &entity.scope_key,
        entity.project_scope_id,
        entity.organization_id_at_time,
    )
}

fn scope_from_parts(
    scope_key: &GraphScopeKey,
    project_scope_id: Option<ProjectScopeId>,
    organization_id_at_time: Option<Uuid>,
) -> Result<GraphScopeRecord, GraphError> {
    let scope = match (project_scope_id, organization_id_at_time) {
        (Some(project), Some(organization)) => {
            GraphScopeRecord::organization(project.0, organization)
        }
        (None, None) => GraphScopeRecord::global_sanitized(),
        _ => {
            return Err(GraphError::InvalidArgument(
                "partial temporal graph scope".to_string(),
            ))
        }
    };
    if scope.scope_key != scope_key.as_str() {
        return Err(GraphError::InvalidArgument(
            "temporal scope key mismatch".to_string(),
        ));
    }
    Ok(scope)
}

fn lineage_write(lineage: &TemporalLineageProjection) -> AssertionLineageWrite {
    AssertionLineageWrite {
        assertion_id: lineage.assertion_id,
        source_stream_key: lineage.source_stream_key.clone(),
        source_version: lineage.source_version,
        evidence_refs: lineage.evidence_refs.clone(),
        status: lineage.status.clone(),
        valid_from: lineage.valid_from,
        valid_to: lineage.valid_to,
        fresh_until: lineage.fresh_until,
        classification: lineage.classification.as_str().to_string(),
        projection_schema_version: lineage.projection_schema_version,
    }
}

fn relation_lineage_write(lineage: &TemporalRelationLineageProjection) -> AssertionLineageWrite {
    AssertionLineageWrite {
        assertion_id: lineage.assertion_id,
        source_stream_key: lineage.source_stream_key.clone(),
        source_version: lineage.source_version,
        evidence_refs: lineage.evidence_refs.clone(),
        status: lineage.status.clone(),
        valid_from: lineage.valid_from,
        valid_to: lineage.valid_to,
        fresh_until: lineage.fresh_until,
        classification: lineage.classification.as_str().to_string(),
        projection_schema_version: lineage.projection_schema_version,
    }
}

fn deterministic_entity_id(generation_id: Uuid, canonical_ref: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("temporal-entity:{generation_id}:{canonical_ref}").as_bytes(),
    )
}

fn deterministic_relation_id(
    generation_id: Uuid,
    from: &str,
    to: &str,
    relation_type: TemporalRelationType,
) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(
            "temporal-relation:{generation_id}:{from}:{}:{to}",
            relation_type.as_str()
        )
        .as_bytes(),
    )
}

pub fn identity_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn generation_view(row: knowledge_graph::GraphGenerationRow) -> GraphGeneration {
    GraphGeneration {
        generation_id: row.generation_id,
        scope_key: GraphScopeKey(row.scope_key),
        projection_schema_version: row.projection_schema_version,
        status: row.status,
        build_hash: row.build_hash,
        entity_count: row.entity_count,
        relation_count: row.relation_count,
    }
}

fn parse_classification(value: &str) -> Result<KnowledgeClassification, GraphError> {
    match value {
        "public" => Ok(KnowledgeClassification::Public),
        "internal" => Ok(KnowledgeClassification::Internal),
        "customer_confidential" => Ok(KnowledgeClassification::CustomerConfidential),
        "restricted" => Ok(KnowledgeClassification::Restricted),
        other => Err(GraphError::InvalidArgument(format!(
            "corrupt temporal classification: {other}"
        ))),
    }
}
