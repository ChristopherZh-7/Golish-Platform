use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use golish_graphiti::temporal_client::identity_hash;
use golish_graphiti::{
    GraphScopeKey, GraphVisibility, TemporalEntityProjection, TemporalEntityType,
    TemporalGraphInvalidation, TemporalGraphProjection, TemporalLineageProjection,
    TemporalRelationLineageProjection, TemporalRelationProjection, TemporalRelationType,
};
use golish_memory_domain::assertion::{AssertionIdentity, AssertionKind, AssertionObject};
use golish_memory_domain::classification::AssertionVisibility;
use golish_memory_domain::KnowledgeAssertion;
use serde_json::{Map, Value};

pub const GRAPH_PROJECTION_SCHEMA_V1: i32 = 1;

pub fn project_assertion(
    assertion: &KnowledgeAssertion,
) -> Result<TemporalGraphProjection, ProjectionError> {
    assertion
        .validate_integrity()
        .map_err(|_| ProjectionError::InvalidAssertion)?;
    assertion
        .source
        .validate()
        .map_err(|_| ProjectionError::InvalidAssertion)?;
    assertion
        .identity
        .validate()
        .map_err(|_| ProjectionError::InvalidAssertion)?;
    let derived_identity = AssertionIdentity::derive(
        assertion.identity.subject_key.clone(),
        assertion.identity.predicate.clone(),
        &assertion.object,
    )
    .map_err(|_| ProjectionError::InvalidAssertion)?;
    if derived_identity != assertion.identity {
        return Err(ProjectionError::InvalidAssertion);
    }
    let object = match &assertion.object {
        AssertionObject::Json(Value::Object(object)) => object,
        AssertionObject::Json(_) => return Err(ProjectionError::ObjectMustBeStructured),
        AssertionObject::VaultRef(_) => return Err(ProjectionError::SensitiveObjectRejected),
    };
    let predicate = assertion.identity.predicate.as_str();
    if let Some(entity_type) = predicate
        .strip_prefix("graph.entity.")
        .and_then(parse_entity_type)
    {
        return project_entity(assertion, object, entity_type);
    }
    if let Some(relation_type) = predicate
        .strip_prefix("graph.relation.")
        .and_then(parse_relation_type)
    {
        return project_relation(assertion, object, relation_type);
    }
    Err(ProjectionError::UnsupportedPredicate(predicate.to_string()))
}

pub fn project_invalidation(
    assertion: &KnowledgeAssertion,
    valid_to: DateTime<Utc>,
) -> TemporalGraphInvalidation {
    TemporalGraphInvalidation {
        close_assertion_id: assertion.assertion_id,
        valid_to,
    }
}

fn project_entity(
    assertion: &KnowledgeAssertion,
    object: &Map<String, Value>,
    entity_type: TemporalEntityType,
) -> Result<TemporalGraphProjection, ProjectionError> {
    let (scope_key, visibility, project_scope_id, organization_id_at_time) =
        projection_scope(assertion, entity_type)?;
    let canonical_ref = required_bounded_string(object, "canonical_ref", 1024)?;
    let display_name = required_bounded_string(object, "display_name", 256)?;
    let properties = allowlisted_properties(
        object.get("properties"),
        entity_property_allowlist(entity_type),
    )?;
    let projection_identity_hash =
        identity_hash(&[scope_key.as_str(), canonical_ref, entity_type.as_str()]);
    let entity = TemporalEntityProjection {
        scope_key,
        visibility,
        project_scope_id,
        organization_id_at_time,
        identity_hash: projection_identity_hash,
        canonical_ref: canonical_ref.to_string(),
        entity_type,
        display_name: display_name.to_string(),
        properties,
    };
    Ok(TemporalGraphProjection {
        entities: vec![entity],
        entity_lineages: vec![entity_lineage(assertion, canonical_ref)],
        relations: Vec::new(),
        relation_lineages: Vec::new(),
    })
}

fn project_relation(
    assertion: &KnowledgeAssertion,
    object: &Map<String, Value>,
    relation_type: TemporalRelationType,
) -> Result<TemporalGraphProjection, ProjectionError> {
    if matches!(assertion.visibility, AssertionVisibility::GlobalSanitized) {
        return Err(ProjectionError::GlobalEntityMustBeTechnique);
    }
    let from_ref = required_bounded_string(object, "from_canonical_ref", 1024)?;
    let to_ref = required_bounded_string(object, "to_canonical_ref", 1024)?;
    if from_ref == to_ref {
        return Err(ProjectionError::SelfRelation);
    }
    let from_type = parse_required_entity_type(object, "from_entity_type")?;
    let to_type = parse_required_entity_type(object, "to_entity_type")?;
    let (scope_key, visibility, project_scope_id, organization_id_at_time) =
        projection_scope(assertion, from_type)?;
    let from_identity_hash = identity_hash(&[scope_key.as_str(), from_ref, from_type.as_str()]);
    let to_identity_hash = identity_hash(&[scope_key.as_str(), to_ref, to_type.as_str()]);
    let relation_identity_hash =
        identity_hash(&[scope_key.as_str(), from_ref, relation_type.as_str(), to_ref]);
    let from_entity = TemporalEntityProjection {
        scope_key: scope_key.clone(),
        visibility,
        project_scope_id,
        organization_id_at_time,
        canonical_ref: from_ref.to_string(),
        identity_hash: from_identity_hash,
        entity_type: from_type,
        display_name: bounded_display_name(object, "from_display_name", from_ref)?.to_string(),
        properties: allowlisted_properties(
            object.get("from_properties"),
            entity_property_allowlist(from_type),
        )?,
    };
    let to_entity = TemporalEntityProjection {
        scope_key: scope_key.clone(),
        visibility,
        project_scope_id,
        organization_id_at_time,
        canonical_ref: to_ref.to_string(),
        identity_hash: to_identity_hash,
        entity_type: to_type,
        display_name: bounded_display_name(object, "to_display_name", to_ref)?.to_string(),
        properties: allowlisted_properties(
            object.get("to_properties"),
            entity_property_allowlist(to_type),
        )?,
    };
    let properties = allowlisted_properties(
        object.get("properties"),
        &["confidence", "port", "protocol"],
    )?;
    let relation = TemporalRelationProjection {
        scope_key,
        from_canonical_ref: from_ref.to_string(),
        to_canonical_ref: to_ref.to_string(),
        relation_type,
        identity_hash: relation_identity_hash,
        properties,
    };
    Ok(TemporalGraphProjection {
        entities: vec![from_entity, to_entity],
        entity_lineages: vec![
            entity_lineage(assertion, from_ref),
            entity_lineage(assertion, to_ref),
        ],
        relations: vec![relation],
        relation_lineages: vec![TemporalRelationLineageProjection {
            from_canonical_ref: from_ref.to_string(),
            to_canonical_ref: to_ref.to_string(),
            relation_type,
            assertion_id: assertion.assertion_id,
            source_stream_key: assertion.source.source_stream_key.clone(),
            source_version: assertion.source.version,
            evidence_refs: assertion.evidence_ids.clone(),
            status: assertion.status.as_str().to_string(),
            valid_from: assertion.valid_from,
            valid_to: assertion.valid_to,
            fresh_until: assertion.fresh_until,
            classification: assertion.classification,
            projection_schema_version: GRAPH_PROJECTION_SCHEMA_V1,
        }],
    })
}

fn projection_scope(
    assertion: &KnowledgeAssertion,
    entity_type: TemporalEntityType,
) -> Result<
    (
        GraphScopeKey,
        GraphVisibility,
        Option<golish_memory_domain::scope::ProjectScopeId>,
        Option<uuid::Uuid>,
    ),
    ProjectionError,
> {
    match &assertion.visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } => Ok((
            GraphScopeKey::organization(*project_scope_id, *organization_id_at_time),
            GraphVisibility::OrganizationLongTerm,
            Some(*project_scope_id),
            Some(*organization_id_at_time),
        )),
        AssertionVisibility::GlobalSanitized => {
            if assertion.kind != AssertionKind::TechniqueExperience
                || entity_type != TemporalEntityType::Technique
                || !assertion.classification.allows_global_sanitized()
            {
                return Err(ProjectionError::GlobalEntityMustBeTechnique);
            }
            Ok((
                GraphScopeKey::global_sanitized(),
                GraphVisibility::GlobalSanitized,
                None,
                None,
            ))
        }
    }
}

fn entity_lineage(
    assertion: &KnowledgeAssertion,
    canonical_ref: &str,
) -> TemporalLineageProjection {
    TemporalLineageProjection {
        canonical_ref: canonical_ref.to_string(),
        assertion_id: assertion.assertion_id,
        source_stream_key: assertion.source.source_stream_key.clone(),
        source_version: assertion.source.version,
        evidence_refs: assertion.evidence_ids.clone(),
        status: assertion.status.as_str().to_string(),
        valid_from: assertion.valid_from,
        valid_to: assertion.valid_to,
        fresh_until: assertion.fresh_until,
        classification: assertion.classification,
        projection_schema_version: GRAPH_PROJECTION_SCHEMA_V1,
    }
}

fn required_bounded_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
    max_bytes: usize,
) -> Result<&'a str, ProjectionError> {
    optional_bounded_string(object, key, max_bytes)?.ok_or(ProjectionError::MissingField(key))
}

fn optional_bounded_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
    max_bytes: usize,
) -> Result<Option<&'a str>, ProjectionError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let value = value.as_str().ok_or(ProjectionError::FieldInvalid(key))?;
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(ProjectionError::FieldInvalid(key));
    }
    Ok(Some(value))
}

fn bounded_display_name<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
    fallback: &'a str,
) -> Result<&'a str, ProjectionError> {
    if let Some(display_name) = optional_bounded_string(object, key, 256)? {
        return Ok(display_name);
    }
    if fallback.len() > 256 || fallback.chars().any(char::is_control) {
        return Err(ProjectionError::FieldInvalid(key));
    }
    Ok(fallback)
}

fn parse_required_entity_type(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<TemporalEntityType, ProjectionError> {
    required_bounded_string(object, key, 64)
        .ok()
        .and_then(parse_entity_type)
        .ok_or(ProjectionError::MissingField(key))
}

fn allowlisted_properties(
    value: Option<&Value>,
    allowlist: &[&str],
) -> Result<Value, ProjectionError> {
    let Some(value) = value else {
        return Ok(Value::Object(Map::new()));
    };
    let object = value
        .as_object()
        .ok_or(ProjectionError::PropertiesMustBeObject)?;
    let mut sorted = BTreeMap::new();
    for key in allowlist {
        if let Some(value) = object.get(*key) {
            validate_property_value(value)?;
            sorted.insert((*key).to_string(), value.clone());
        }
    }
    let mut result = Map::new();
    for (key, value) in sorted {
        result.insert(key, value);
    }
    let projected = Value::Object(result);
    if serde_json::to_vec(&projected)
        .map_err(|_| ProjectionError::PropertyValueRejected)?
        .len()
        > 4096
    {
        return Err(ProjectionError::PropertyBudgetExceeded);
    }
    Ok(projected)
}

fn validate_property_value(value: &Value) -> Result<(), ProjectionError> {
    match value {
        Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(value)
            if value.len() <= 512 && !value.chars().any(char::is_control) =>
        {
            Ok(())
        }
        Value::Array(values)
            if values.len() <= 16
                && values.iter().all(|value| {
                    matches!(value, Value::Bool(_) | Value::Number(_))
                        || matches!(value, Value::String(value) if value.len() <= 256 && !value.chars().any(char::is_control))
                }) =>
        {
            Ok(())
        }
        _ => Err(ProjectionError::PropertyValueRejected),
    }
}

fn entity_property_allowlist(entity_type: TemporalEntityType) -> &'static [&'static str] {
    match entity_type {
        TemporalEntityType::Organization => &["country", "industry"],
        TemporalEntityType::Target => &["target_type"],
        TemporalEntityType::Host => &["address_family", "os_family"],
        TemporalEntityType::Service => &["port", "product", "protocol", "version"],
        TemporalEntityType::Endpoint => &["method", "path", "status_code"],
        TemporalEntityType::Vulnerability => &["cve", "cvss", "severity"],
        TemporalEntityType::Finding => &["severity", "status"],
        TemporalEntityType::Technique => {
            &["category", "conditions", "failure_mode", "technique_id"]
        }
    }
}

fn parse_entity_type(value: &str) -> Option<TemporalEntityType> {
    match value {
        "organization" => Some(TemporalEntityType::Organization),
        "target" => Some(TemporalEntityType::Target),
        "host" => Some(TemporalEntityType::Host),
        "service" => Some(TemporalEntityType::Service),
        "endpoint" => Some(TemporalEntityType::Endpoint),
        "vulnerability" => Some(TemporalEntityType::Vulnerability),
        "finding" => Some(TemporalEntityType::Finding),
        "technique" => Some(TemporalEntityType::Technique),
        _ => None,
    }
}

fn parse_relation_type(value: &str) -> Option<TemporalRelationType> {
    match value {
        "contains" => Some(TemporalRelationType::Contains),
        "resolves_to" => Some(TemporalRelationType::ResolvesTo),
        "runs_service" => Some(TemporalRelationType::RunsService),
        "exposes_endpoint" => Some(TemporalRelationType::ExposesEndpoint),
        "has_vulnerability" => Some(TemporalRelationType::HasVulnerability),
        "supported_by_finding" => Some(TemporalRelationType::SupportedByFinding),
        "associated_technique" => Some(TemporalRelationType::AssociatedTechnique),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    #[error("assertion is not internally valid")]
    InvalidAssertion,
    #[error("assertion object must be a structured JSON object")]
    ObjectMustBeStructured,
    #[error("VaultRef/secret assertion objects cannot enter the graph")]
    SensitiveObjectRejected,
    #[error("unsupported graph predicate: {0}")]
    UnsupportedPredicate(String),
    #[error("missing or invalid graph field: {0}")]
    MissingField(&'static str),
    #[error("graph field exceeds its typed boundary: {0}")]
    FieldInvalid(&'static str),
    #[error("graph properties must be an object")]
    PropertiesMustBeObject,
    #[error("graph property values must be bounded scalars or small scalar arrays")]
    PropertyValueRejected,
    #[error("graph properties exceed the projection budget")]
    PropertyBudgetExceeded,
    #[error("global-sanitized graph projection only supports safe techniques")]
    GlobalEntityMustBeTechnique,
    #[error("self-relations are not allowed")]
    SelfRelation,
}

impl ProjectionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidAssertion => "knowledge_graph_assertion_invalid",
            Self::ObjectMustBeStructured => "knowledge_graph_object_unstructured",
            Self::SensitiveObjectRejected => "knowledge_graph_sensitive_object_rejected",
            Self::UnsupportedPredicate(_) => "knowledge_graph_predicate_unsupported",
            Self::MissingField(_) => "knowledge_graph_field_missing",
            Self::FieldInvalid(_) => "knowledge_graph_field_invalid",
            Self::PropertiesMustBeObject => "knowledge_graph_properties_invalid",
            Self::PropertyValueRejected => "knowledge_graph_property_value_rejected",
            Self::PropertyBudgetExceeded => "knowledge_graph_property_budget_exceeded",
            Self::GlobalEntityMustBeTechnique => "knowledge_graph_global_policy_violation",
            Self::SelfRelation => "knowledge_graph_self_relation",
        }
    }
}
