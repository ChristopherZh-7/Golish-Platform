//! Stable materialized Hypothesis list/detail queries.

use std::collections::BTreeMap;

use golish_core::investigation_projection::ProjectionEntityV1;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::legacy::{load_legacy_candidate_map_on, unavailable_legacy_projection};
use super::summary::load_exact_actor_topology_on;
use super::types::{
    invalid_payload, InvestigationHypothesisDetail, InvestigationHypothesisListItem,
    InvestigationHypothesisListPage, InvestigationHypothesisListQuery,
    InvestigationHypothesisSortKey, InvestigationPageValidationInput,
    InvestigationProjectionResult, InvestigationStageRunSelector,
};
use super::{apply_expected_page_authority, InvestigationProjectionReadSnapshot};

/// Plan B does not install capability-assessment authority.  This single
/// explicit value is distinct from planning readiness and residual reasons.
pub const PLAN_B_CAPABILITY_STATE_NOT_AVAILABLE: &str = "not_available_plan_c";

#[derive(Debug, sqlx::FromRow)]
struct ProjectionBodyRow {
    entity_id: String,
    entity_version: i64,
    projection_body: Value,
    invalidation_reason: Option<String>,
    organization_ordinal: i32,
    payload_valid: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HypothesisBody {
    source_generation_id: Uuid,
    root_id: Uuid,
    revision_id: Uuid,
    revision_ordinal: i32,
    predecessor_revision_id: Option<Uuid>,
    revision_hash: String,
    revision_ingredients_hash: String,
    semantic_key: SemanticKeyBody,
    semantic_key_hash: String,
    state: String,
    lifecycle_state: String,
    planning_readiness: String,
    target_type_at_time: String,
    target_value_at_time: String,
    origin_decision_hash: String,
    proposal: ProposalBody,
    proof_refs: Vec<Value>,
    refutation_refs: Vec<Value>,
    relation_sources: Vec<RelationSourceBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticKeyBody {
    schema: String,
    organization_id: Uuid,
    subject: SubjectBody,
    predicate: PredicateBody,
    trust_boundary: String,
    polarity: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectBody {
    kind: String,
    identity_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredicateBody {
    schema: String,
    version: u32,
    normalized_arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalBody {
    kind: String,
    proposal_id: Uuid,
    subject_kind: String,
    subject_identity_hash: String,
    predicate: PredicateBody,
    trust_boundary: String,
    polarity: String,
    prose: String,
    confidence: i32,
    priority: i32,
    tags: Vec<String>,
    evidence_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationSourceBody {
    root_id: Uuid,
    revision_id: Uuid,
    relation_kind: String,
}

#[derive(Debug)]
struct ParsedHypothesis {
    body: HypothesisBody,
    source_refs: Vec<SourceRef>,
    entity_version: i64,
    invalidation_reason: Option<String>,
    organization_ordinal: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResidualBody {
    residual_id: Uuid,
    residual_hash: String,
    reason: String,
    #[serde(default)]
    root_id: Option<Uuid>,
    #[serde(default)]
    revision_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
struct ResidualCode {
    reason: String,
    root_id: Option<Uuid>,
    revision_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceRefKind {
    ToolTruthEvidence,
    Finding,
    VerificationReceipt,
    ApplicationContext,
    KnowledgeSignal,
    Gap,
}

#[derive(Debug, Clone)]
struct SourceRef {
    kind: SourceRefKind,
    id: String,
    contradiction: bool,
}

fn parse_projection_body(
    row: ProjectionBodyRow,
) -> InvestigationProjectionResult<ParsedHypothesis> {
    if !row.payload_valid {
        return Err(invalid_payload(
            "Hypothesis projection payload or frozen organization scope is invalid",
        ));
    }
    let entity: ProjectionEntityV1 = serde_json::from_value(row.projection_body)
        .map_err(|error| invalid_payload(format!("typed Hypothesis projection: {error}")))?;
    let ProjectionEntityV1::Hypothesis(record) = entity else {
        return Err(invalid_payload(
            "Hypothesis row contains another entity kind",
        ));
    };
    if record.record().entity_id() != row.entity_id
        || i64::try_from(record.record().entity_version()).ok() != Some(row.entity_version)
    {
        return Err(invalid_payload("Hypothesis projection identity mismatch"));
    }
    let body: HypothesisBody =
        serde_json::from_value(record.record().canonical_redacted_body().as_value().clone())
            .map_err(|error| invalid_payload(format!("Hypothesis redacted body: {error}")))?;
    if body.root_id.to_string() != row.entity_id
        || body.revision_ordinal.checked_add(1).map(i64::from) != Some(row.entity_version)
        || body.semantic_key.schema != "hypothesis_semantic_key.v1"
        || body.proposal.kind != "hypothesis_proposal"
        || body.proposal.subject_kind != body.semantic_key.subject.kind
        || body.proposal.subject_identity_hash != body.semantic_key.subject.identity_hash
        || body.proposal.predicate.schema != body.semantic_key.predicate.schema
        || body.proposal.predicate.version != body.semantic_key.predicate.version
        || body.proposal.predicate.normalized_arguments
            != body.semantic_key.predicate.normalized_arguments
        || body.proposal.trust_boundary != body.semantic_key.trust_boundary
        || body.proposal.polarity != body.semantic_key.polarity
        || body.source_generation_id.is_nil()
        || body.target_type_at_time.trim().is_empty()
        || body.target_value_at_time.trim().is_empty()
        || body.target_value_at_time != body.semantic_key.subject.identity_hash
        || !matches!(
            body.lifecycle_state.as_str(),
            "current" | "superseded" | "closed"
        )
        || readiness_rank(&body.planning_readiness).is_err()
        || !matches!(
            body.state.as_str(),
            "proposed"
                | "supported"
                | "contested"
                | "inconclusive"
                | "verified"
                | "refuted"
                | "invalid"
        )
    {
        return Err(invalid_payload(
            "Hypothesis redacted authority is inconsistent",
        ));
    }
    for value in [
        &body.revision_hash,
        &body.revision_ingredients_hash,
        &body.semantic_key_hash,
        &body.origin_decision_hash,
    ] {
        if !is_sha256(value) {
            return Err(invalid_payload("Hypothesis authority hash is malformed"));
        }
    }
    // These model-authored presentation fields are intentionally parsed only
    // to enforce the exact frozen payload. They are never returned by this
    // read model.
    let _presentation_only = (
        body.proposal.proposal_id,
        body.proposal.prose.as_str(),
        body.proposal.confidence,
        body.proposal.priority,
        body.proposal.tags.as_slice(),
        body.proposal.evidence_refs.as_slice(),
    );
    for relation in &body.relation_sources {
        if relation.root_id.is_nil()
            || relation.revision_id.is_nil()
            || relation.relation_kind.trim().is_empty()
        {
            return Err(invalid_payload("Hypothesis relation source is malformed"));
        }
    }

    let mut source_refs = Vec::new();
    for value in body.proof_refs.iter() {
        source_refs.push(parse_source_ref(value, false)?);
    }
    for value in body.refutation_refs.iter() {
        source_refs.push(parse_source_ref(value, true)?);
    }
    Ok(ParsedHypothesis {
        body,
        source_refs,
        entity_version: row.entity_version,
        invalidation_reason: row.invalidation_reason,
        organization_ordinal: row.organization_ordinal,
    })
}

fn parse_source_ref(
    value: &Value,
    contradiction: bool,
) -> InvestigationProjectionResult<SourceRef> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| invalid_payload("Hypothesis source ref is not exact-one"))?;
    let (kind, id) = object.iter().next().expect("exact-one source ref");
    let id = id
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| invalid_payload("Hypothesis source ref id is invalid"))?;
    let kind = match kind.as_str() {
        "ToolTruthEvidence" => SourceRefKind::ToolTruthEvidence,
        "Finding" => SourceRefKind::Finding,
        "VerificationReceipt" => SourceRefKind::VerificationReceipt,
        "ApplicationContext" => SourceRefKind::ApplicationContext,
        "KnowledgeSignal" => SourceRefKind::KnowledgeSignal,
        "Gap" => SourceRefKind::Gap,
        _ => return Err(invalid_payload("unknown Hypothesis source ref kind")),
    };
    Ok(SourceRef {
        kind,
        id: id.to_owned(),
        contradiction,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn readiness_rank(value: &str) -> InvestigationProjectionResult<i16> {
    match value {
        "ready_for_strategy" => Ok(0),
        "needs_enrichment" => Ok(1),
        "deferred" => Ok(2),
        "out_of_scope" => Ok(3),
        "unsafe" => Ok(4),
        _ => Err(invalid_payload("unknown planning readiness")),
    }
}

fn epistemic_rank(value: &str) -> InvestigationProjectionResult<i16> {
    match value {
        "proposed" => Ok(0),
        "supported" => Ok(1),
        "contested" => Ok(2),
        "inconclusive" => Ok(3),
        "verified" => Ok(4),
        "refuted" => Ok(5),
        "invalid" => Ok(6),
        _ => Err(invalid_payload("unknown epistemic state")),
    }
}

async fn load_latest_page_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
    query: &InvestigationHypothesisListQuery,
) -> InvestigationProjectionResult<Vec<ProjectionBodyRow>> {
    let after = query
        .after
        .clone()
        .unwrap_or(InvestigationHypothesisSortKey {
            organization_ordinal: -1,
            group_key: String::new(),
            readiness_rank: -1,
            epistemic_rank: -1,
            root_id: Uuid::nil(),
            revision_ordinal: -1,
        });
    let limit = i64::from(query.page_size.clamp(1, 100)) + 1;
    Ok(sqlx::query_as::<_, ProjectionBodyRow>(
        r#"WITH latest AS MATERIALIZED (
               SELECT DISTINCT ON(entity.entity_id)
                      entity.entity_id,entity.entity_version,entity.projection_body,
                      entity.invalidation_reason
                 FROM investigation_projection_entity_versions entity
                WHERE entity.operation_id=$1 AND entity.entity_kind='hypothesis'
                  AND entity.change_seq<=$2
                ORDER BY entity.entity_id,entity.entity_version DESC
           ), ranked AS MATERIALIZED (
               SELECT latest.*,
                      COALESCE(scope_unit.ordinal,32767) AS organization_ordinal,
                      COALESCE(
                          scope_unit.ordinal IS NOT NULL
                          AND jsonb_typeof(latest.projection_body #>
                              '{record,canonicalRedactedBody}')='object'
                          AND (latest.projection_body #>
                              '{record,canonicalRedactedBody}') ?& ARRAY[
                                  'source_generation_id','root_id','revision_id',
                                  'revision_ordinal','predecessor_revision_id','revision_hash',
                                  'revision_ingredients_hash','semantic_key','semantic_key_hash',
                                  'state','lifecycle_state','planning_readiness',
                                  'target_type_at_time','target_value_at_time',
                                  'origin_decision_hash','proposal','proof_refs',
                                  'refutation_refs','relation_sources'
                              ]
                          AND latest.projection_body #>>
                              '{record,canonicalRedactedBody,state}' IN (
                                  'proposed','supported','contested','inconclusive',
                                  'verified','refuted','invalid')
                          AND latest.projection_body #>>
                              '{record,canonicalRedactedBody,planning_readiness}' IN (
                                  'ready_for_strategy','needs_enrichment','deferred',
                                  'out_of_scope','unsafe'),
                          FALSE
                      ) AS payload_valid,
                      latest.projection_body #>>
                          '{record,canonicalRedactedBody,semantic_key_hash}' AS group_key,
                      CASE WHEN latest.invalidation_reason IS NOT NULL THEN 4
                           ELSE CASE latest.projection_body #>>
                              '{record,canonicalRedactedBody,planning_readiness}'
                              WHEN 'ready_for_strategy' THEN 0 WHEN 'needs_enrichment' THEN 1
                              WHEN 'deferred' THEN 2 WHEN 'out_of_scope' THEN 3
                              WHEN 'unsafe' THEN 4 ELSE 32767 END
                           END::SMALLINT AS readiness_rank,
                      CASE latest.projection_body #>> '{record,canonicalRedactedBody,state}'
                          WHEN 'proposed' THEN 0 WHEN 'supported' THEN 1
                          WHEN 'contested' THEN 2 WHEN 'inconclusive' THEN 3
                          WHEN 'verified' THEN 4 WHEN 'refuted' THEN 5
                          WHEN 'invalid' THEN 6 ELSE 32767 END::SMALLINT AS epistemic_rank,
                      CASE WHEN latest.projection_body #>>
                              '{record,canonicalRedactedBody,root_id}' ~*
                              '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                           THEN (latest.projection_body #>>
                              '{record,canonicalRedactedBody,root_id}')::UUID
                           ELSE '00000000-0000-0000-0000-000000000000'::UUID END AS root_id,
                      CASE WHEN latest.projection_body #>>
                              '{record,canonicalRedactedBody,revision_ordinal}' ~ '^[0-9]{1,9}$'
                           THEN (latest.projection_body #>>
                              '{record,canonicalRedactedBody,revision_ordinal}')::INTEGER
                           ELSE -1 END AS revision_ordinal,
                      scope_unit.organization_id AS organization_id,
                      CASE WHEN jsonb_typeof(latest.projection_body #>
                              '{record,canonicalRedactedBody,proof_refs}')='array'
                           THEN latest.projection_body #>
                              '{record,canonicalRedactedBody,proof_refs}'
                           ELSE '[]'::JSONB END AS proof_refs,
                      CASE WHEN jsonb_typeof(latest.projection_body #>
                              '{record,canonicalRedactedBody,refutation_refs}')='array'
                           THEN latest.projection_body #>
                              '{record,canonicalRedactedBody,refutation_refs}'
                           ELSE '[]'::JSONB END AS refutation_refs,
                      latest.projection_body #>>
                          '{record,canonicalRedactedBody,state}' AS epistemic_state,
                      CASE WHEN latest.invalidation_reason IS NOT NULL THEN 'unsafe'
                           ELSE latest.projection_body #>>
                              '{record,canonicalRedactedBody,planning_readiness}'
                           END AS readiness_state
                 FROM latest
                 LEFT JOIN operation_org_scope_snapshots scope
                   ON scope.operation_id=$1 AND scope.sealed_at IS NOT NULL
                 LEFT JOIN operation_org_scope_units scope_unit
                   ON scope_unit.snapshot_id=scope.id
                  AND scope_unit.organization_id::TEXT=latest.projection_body #>>
                      '{record,canonicalRedactedBody,semantic_key,organization_id}'
           )
           SELECT entity_id,entity_version,projection_body,invalidation_reason,
                  organization_ordinal,payload_valid
             FROM ranked
            WHERE NOT payload_valid OR (
              (cardinality($3::UUID[])=0 OR organization_id=ANY($3))
              AND (cardinality($4::TEXT[])=0 OR epistemic_state=ANY($4))
              AND (cardinality($5::TEXT[])=0 OR readiness_state=ANY($5))
              AND (cardinality($6::TEXT[])=0 OR
                   'not_available_plan_c'=ANY($6))
              AND (cardinality($7::TEXT[])=0 OR EXISTS(
                  SELECT 1
                    FROM jsonb_array_elements(proof_refs || refutation_refs) source_ref
                    CROSS JOIN LATERAL jsonb_object_keys(source_ref) source_kind
                   WHERE CASE source_kind
                       WHEN 'ToolTruthEvidence' THEN 'tool_truth_evidence'
                       WHEN 'Finding' THEN 'finding'
                       WHEN 'VerificationReceipt' THEN 'verification_receipt'
                       WHEN 'ApplicationContext' THEN 'application_context'
                       WHEN 'KnowledgeSignal' THEN 'knowledge_signal'
                       WHEN 'Gap' THEN 'gap' ELSE '' END = ANY($7)))
              AND (organization_ordinal,group_key,readiness_rank,epistemic_rank,
                   root_id,revision_ordinal)>($8,$9,$10,$11,$12,$13)
            )
            ORDER BY payload_valid,organization_ordinal,group_key,readiness_rank,
                     epistemic_rank,root_id,revision_ordinal
            LIMIT $14"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .bind(&query.filters.organization_ids)
    .bind(&query.filters.epistemic_states)
    .bind(&query.filters.readiness_states)
    .bind(&query.filters.capability_states)
    .bind(&query.filters.source_kinds)
    .bind(after.organization_ordinal)
    .bind(after.group_key)
    .bind(after.readiness_rank)
    .bind(after.epistemic_rank)
    .bind(after.root_id)
    .bind(after.revision_ordinal)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?)
}

async fn load_revision_lineage_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
    revision_id: Uuid,
) -> InvestigationProjectionResult<Vec<ProjectionBodyRow>> {
    Ok(sqlx::query_as::<_, ProjectionBodyRow>(
        r#"WITH target AS MATERIALIZED (
               SELECT entity_id
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='hypothesis' AND change_seq<=$2
                  AND projection_body #>>
                      '{record,canonicalRedactedBody,revision_id}'=$3::TEXT
                ORDER BY entity_version DESC LIMIT 1
           )
           SELECT entity.entity_id,entity.entity_version,entity.projection_body,
                  entity.invalidation_reason,COALESCE(scope_unit.ordinal,32767)
                      AS organization_ordinal,
                  scope_unit.ordinal IS NOT NULL AS payload_valid
             FROM investigation_projection_entity_versions entity
             JOIN target USING(entity_id)
             LEFT JOIN operation_org_scope_snapshots scope
               ON scope.operation_id=$1 AND scope.sealed_at IS NOT NULL
             LEFT JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=scope.id
              AND scope_unit.organization_id::TEXT=entity.projection_body #>>
                  '{record,canonicalRedactedBody,semantic_key,organization_id}'
            WHERE entity.operation_id=$1 AND entity.entity_kind='hypothesis'
              AND entity.change_seq<=$2
            ORDER BY entity.entity_version"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .bind(revision_id.to_string())
    .fetch_all(&mut **tx)
    .await?)
}

async fn load_residual_codes_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
    root_ids: &[Uuid],
    revision_ids: &[Uuid],
) -> InvestigationProjectionResult<Vec<ResidualCode>> {
    if root_ids.is_empty() && revision_ids.is_empty() {
        return Ok(Vec::new());
    }
    let root_ids = root_ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    let revision_ids = revision_ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    let rows: Vec<Value> = sqlx::query_scalar(
        r#"SELECT projection_body FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='residual' AND change_seq<=$2
                  AND (
                    projection_body #>> '{record,canonicalRedactedBody,root_id}' IS NULL
                    OR projection_body #>> '{record,canonicalRedactedBody,root_id}'=ANY($3)
                    OR projection_body #>> '{record,canonicalRedactedBody,revision_id}'=ANY($4)
                  )
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .bind(&root_ids)
    .bind(&revision_ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut codes = BTreeMap::new();
    for value in rows {
        let entity: ProjectionEntityV1 = serde_json::from_value(value)
            .map_err(|error| invalid_payload(format!("typed residual projection: {error}")))?;
        let ProjectionEntityV1::Residual(record) = entity else {
            return Err(invalid_payload("residual row contains another entity kind"));
        };
        let body: ResidualBody =
            serde_json::from_value(record.record().canonical_redacted_body().as_value().clone())
                .map_err(|error| invalid_payload(format!("residual body: {error}")))?;
        if body.residual_id.is_nil()
            || !is_sha256(&body.residual_hash)
            || body.reason.trim().is_empty()
            || body.root_id.is_some_and(|value| value.is_nil())
            || body.revision_id.is_some_and(|value| value.is_nil())
            || (body.revision_id.is_some() && body.root_id.is_none())
        {
            return Err(invalid_payload("residual identity is malformed"));
        }
        codes.insert(
            (body.reason.clone(), body.root_id, body.revision_id),
            ResidualCode {
                reason: body.reason,
                root_id: body.root_id,
                revision_id: body.revision_id,
            },
        );
    }
    Ok(codes.into_values().collect())
}

fn applicable_residual_codes(
    residuals: &[ResidualCode],
    root_id: Uuid,
    revision_id: Uuid,
) -> Vec<String> {
    residuals
        .iter()
        .filter(|residual| {
            residual.root_id.is_none_or(|value| value == root_id)
                && residual
                    .revision_id
                    .is_none_or(|value| value == revision_id)
        })
        .map(|residual| residual.reason.clone())
        .collect()
}

fn source_ref_ids(parsed: &ParsedHypothesis, kind: SourceRefKind) -> Vec<String> {
    let mut ids = parsed
        .source_refs
        .iter()
        .filter(|source| source.kind == kind)
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn item_from_parsed(
    parsed: &ParsedHypothesis,
    latest_entity_version: i64,
    legacy_status: Option<String>,
    residual_codes: &[String],
) -> InvestigationProjectionResult<InvestigationHypothesisListItem> {
    let body = &parsed.body;
    let (lifecycle_state, planning_readiness) = if parsed.invalidation_reason.is_some() {
        ("closed".to_owned(), "unsafe".to_owned())
    } else if parsed.entity_version < latest_entity_version {
        ("superseded".to_owned(), "deferred".to_owned())
    } else {
        (
            body.lifecycle_state.clone(),
            body.planning_readiness.clone(),
        )
    };
    let predicate_summary = format!(
        "{}@{}:{}",
        body.semantic_key.predicate.schema,
        body.semantic_key.predicate.version,
        serde_json::to_string(&body.semantic_key.predicate.normalized_arguments)?
    );
    let support_count = parsed
        .source_refs
        .iter()
        .filter(|source| {
            !source.contradiction
                && matches!(
                    source.kind,
                    SourceRefKind::ToolTruthEvidence
                        | SourceRefKind::Finding
                        | SourceRefKind::VerificationReceipt
                )
        })
        .count() as i64;
    let contradiction_count = parsed
        .source_refs
        .iter()
        .filter(|source| {
            source.contradiction
                && matches!(
                    source.kind,
                    SourceRefKind::ToolTruthEvidence
                        | SourceRefKind::Finding
                        | SourceRefKind::VerificationReceipt
                )
        })
        .count() as i64;
    let gap_count = parsed
        .source_refs
        .iter()
        .filter(|source| source.kind == SourceRefKind::Gap)
        .count() as i64;
    Ok(InvestigationHypothesisListItem {
        sort_key: InvestigationHypothesisSortKey {
            organization_ordinal: parsed.organization_ordinal,
            group_key: body.semantic_key_hash.clone(),
            readiness_rank: readiness_rank(&planning_readiness)?,
            epistemic_rank: epistemic_rank(&body.state)?,
            root_id: body.root_id,
            revision_ordinal: body.revision_ordinal,
        },
        root_id: body.root_id,
        revision_id: body.revision_id,
        organization_id: body.semantic_key.organization_id,
        subject_kind: body.semantic_key.subject.kind.clone(),
        subject_identity_hash: body.semantic_key.subject.identity_hash.clone(),
        target_type_at_time: body.target_type_at_time.clone(),
        target_value_at_time: body.target_value_at_time.clone(),
        predicate_schema: body.semantic_key.predicate.schema.clone(),
        predicate_summary,
        trust_boundary: body.semantic_key.trust_boundary.clone(),
        polarity: body.semantic_key.polarity.clone(),
        epistemic_state: body.state.clone(),
        lifecycle_state,
        planning_readiness,
        support_count,
        contradiction_count,
        gap_count,
        legacy_projection_status: legacy_status,
        residual_codes: residual_codes.to_vec(),
    })
}

pub async fn list_investigation_hypotheses(
    pool: &PgPool,
    operation_id: Uuid,
    query: InvestigationHypothesisListQuery,
) -> InvestigationProjectionResult<InvestigationHypothesisListPage> {
    list_investigation_hypotheses_inner(pool, operation_id, None, query).await
}

pub async fn list_investigation_hypotheses_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
    query: InvestigationHypothesisListQuery,
) -> InvestigationProjectionResult<InvestigationHypothesisListPage> {
    list_investigation_hypotheses_inner(pool, operation_id, Some(selector), query).await
}

async fn list_investigation_hypotheses_inner(
    pool: &PgPool,
    operation_id: Uuid,
    selector: Option<&InvestigationStageRunSelector>,
    query: InvestigationHypothesisListQuery,
) -> InvestigationProjectionResult<InvestigationHypothesisListPage> {
    let mut snapshot = if let Some(selector) = selector {
        InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
            .await?
            .0
    } else {
        InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?
    };
    if let Some(expected) = &query.expected_page_authority {
        apply_expected_page_authority(&mut snapshot, expected)?;
    }
    let head = snapshot.authority.temporal.as_of_change_seq;
    let page_size = query.page_size.clamp(1, 100) as usize;
    let mut parsed = load_latest_page_on(&mut snapshot.tx, operation_id, head, &query)
        .await?
        .into_iter()
        .map(parse_projection_body)
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    let has_more = parsed.len() > page_size;
    parsed.truncate(page_size);
    let root_ids = parsed
        .iter()
        .map(|row| row.body.root_id)
        .collect::<Vec<_>>();
    let revision_ids = parsed
        .iter()
        .map(|row| row.body.revision_id)
        .collect::<Vec<_>>();
    let residual_codes = load_residual_codes_on(
        &mut snapshot.tx,
        operation_id,
        head,
        &root_ids,
        &revision_ids,
    )
    .await?;
    let legacy =
        load_legacy_candidate_map_on(&mut snapshot.tx, operation_id, head, &root_ids).await?;
    let hypotheses = parsed
        .iter()
        .map(|row| {
            item_from_parsed(
                row,
                row.entity_version,
                legacy
                    .get(&row.body.root_id)
                    .and_then(|value| value.status.clone()),
                &applicable_residual_codes(&residual_codes, row.body.root_id, row.body.revision_id),
            )
        })
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    let next_key = has_more
        .then(|| hypotheses.last().map(|item| item.sort_key.clone()))
        .flatten();
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok(InvestigationHypothesisListPage {
        authority,
        hypotheses,
        next_key,
    })
}

async fn verification_objective_summaries_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
    revision_id: Uuid,
) -> InvestigationProjectionResult<Vec<String>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        r#"SELECT projection_body
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND entity_kind='hypothesis_verification_plan'
              AND change_seq<=$2
              AND projection_body #>>
                  '{record,canonicalRedactedBody,revision_id}'=$3::TEXT
            ORDER BY change_seq DESC LIMIT 2"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .bind(revision_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 1 {
        return Err(invalid_payload(
            "revision has multiple materialized verification plans",
        ));
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(Vec::new());
    };
    let entity: ProjectionEntityV1 = serde_json::from_value(row)
        .map_err(|error| invalid_payload(format!("typed verification plan: {error}")))?;
    let ProjectionEntityV1::HypothesisVerificationPlan(record) = entity else {
        return Err(invalid_payload(
            "verification-plan row has another entity kind",
        ));
    };
    let body = record.record().canonical_redacted_body().as_value();
    if body.get("revision_id").and_then(Value::as_str) != Some(revision_id.to_string().as_str()) {
        return Err(invalid_payload("verification plan revision mismatch"));
    }
    let objectives = body
        .get("objectives")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_payload("verification objectives missing"))?;
    let mut summaries = objectives
        .iter()
        .map(|objective| {
            let hash = objective
                .get("objective_hash")
                .and_then(Value::as_str)
                .filter(|value| is_sha256(value))
                .ok_or_else(|| invalid_payload("verification objective hash invalid"))?;
            let outcome = objective
                .get("outcome_requirement")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| invalid_payload("verification objective outcome missing"))?;
            Ok(format!("{outcome}:{hash}"))
        })
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    summaries.sort();
    Ok(summaries)
}

pub async fn get_investigation_hypothesis(
    pool: &PgPool,
    operation_id: Uuid,
    revision_id: Uuid,
) -> InvestigationProjectionResult<Option<InvestigationHypothesisDetail>> {
    get_investigation_hypothesis_inner(pool, operation_id, None, revision_id, None).await
}

pub async fn get_investigation_hypothesis_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
    revision_id: Uuid,
    expected: &InvestigationPageValidationInput,
) -> InvestigationProjectionResult<Option<InvestigationHypothesisDetail>> {
    get_investigation_hypothesis_inner(
        pool,
        operation_id,
        Some(selector),
        revision_id,
        Some(expected),
    )
    .await
}

async fn get_investigation_hypothesis_inner(
    pool: &PgPool,
    operation_id: Uuid,
    selector: Option<&InvestigationStageRunSelector>,
    revision_id: Uuid,
    expected: Option<&InvestigationPageValidationInput>,
) -> InvestigationProjectionResult<Option<InvestigationHypothesisDetail>> {
    let mut snapshot = if let Some(selector) = selector {
        InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
            .await?
            .0
    } else {
        InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?
    };
    if let Some(expected) = expected {
        apply_expected_page_authority(&mut snapshot, expected)?;
    }
    let head = snapshot.authority.temporal.as_of_change_seq;
    let parsed = load_revision_lineage_on(&mut snapshot.tx, operation_id, head, revision_id)
        .await?
        .into_iter()
        .map(parse_projection_body)
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    let Some(target) = parsed
        .iter()
        .find(|row| row.body.revision_id == revision_id)
    else {
        snapshot.finish().await?;
        return Ok(None);
    };
    let latest_entity_version = parsed
        .iter()
        .map(|row| row.entity_version)
        .max()
        .ok_or_else(|| invalid_payload("Hypothesis lineage is empty"))?;
    let root_ids = [target.body.root_id];
    let revision_ids = [target.body.revision_id];
    let residual_codes = load_residual_codes_on(
        &mut snapshot.tx,
        operation_id,
        head,
        &root_ids,
        &revision_ids,
    )
    .await?;
    let legacy_map =
        load_legacy_candidate_map_on(&mut snapshot.tx, operation_id, head, &root_ids).await?;
    let legacy = legacy_map
        .get(&target.body.root_id)
        .cloned()
        .unwrap_or_else(unavailable_legacy_projection);
    let item = item_from_parsed(
        target,
        latest_entity_version,
        legacy.status.clone(),
        &applicable_residual_codes(
            &residual_codes,
            target.body.root_id,
            target.body.revision_id,
        ),
    )?;
    let mut lineage = parsed
        .iter()
        .filter(|row| row.body.root_id == target.body.root_id)
        .map(|row| (row.body.revision_ordinal, row.body.revision_id))
        .collect::<Vec<_>>();
    lineage.sort_by_key(|value| value.0);
    let verification_objective_summaries =
        verification_objective_summaries_on(&mut snapshot.tx, operation_id, head, revision_id)
            .await?;
    let actor_topology = if let Some(selector) = selector {
        load_exact_actor_topology_on(&mut snapshot.tx, operation_id, selector)
            .await?
            .into_iter()
            .filter(|actor| actor.hypothesis_revision_id == Some(revision_id))
            .collect()
    } else {
        Vec::new()
    };
    let authority = snapshot.authority.clone();
    let detail = InvestigationHypothesisDetail {
        authority,
        hypothesis: item,
        predecessor_revision_id: target.body.predecessor_revision_id,
        lineage_revision_ids: lineage.into_iter().map(|value| value.1).collect(),
        support_ref_ids: target
            .source_refs
            .iter()
            .filter(|source| {
                !source.contradiction
                    && matches!(
                        source.kind,
                        SourceRefKind::ToolTruthEvidence
                            | SourceRefKind::Finding
                            | SourceRefKind::VerificationReceipt
                    )
            })
            .map(|source| source.id.clone())
            .collect(),
        contradiction_ref_ids: target
            .source_refs
            .iter()
            .filter(|source| {
                source.contradiction
                    && matches!(
                        source.kind,
                        SourceRefKind::ToolTruthEvidence
                            | SourceRefKind::Finding
                            | SourceRefKind::VerificationReceipt
                    )
            })
            .map(|source| source.id.clone())
            .collect(),
        application_context_ref_ids: source_ref_ids(target, SourceRefKind::ApplicationContext),
        gap_ref_ids: source_ref_ids(target, SourceRefKind::Gap),
        verification_objective_summaries,
        actor_topology,
        legacy_unavailable_fields: legacy.unavailable_fields,
    };
    snapshot.finish().await?;
    Ok(Some(detail))
}
