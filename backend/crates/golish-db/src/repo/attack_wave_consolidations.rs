//! Operation-level FactDelta consolidation and Candidate Wave cursor.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use golish_memory_domain::assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertionDraft,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{
    attack_candidate_work_items, attack_fact_deltas, attack_waves, canonical_fact_refs,
    knowledge_assertions, operation_scope_decisions,
};

#[derive(Debug, Clone, Copy)]
pub struct ConsolidateAttackWave {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub source_wave_run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackWaveConsolidationResult {
    pub consolidation_id: Uuid,
    pub decision_kind: String,
    pub target_wave_run_id: Option<Uuid>,
    pub accepted_fact_delta_ids: Vec<Uuid>,
    pub rejected_fact_delta_ids: Vec<Uuid>,
    pub residual_risk_ids: Vec<Uuid>,
    pub pending_enrichment_count: usize,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ScopeUnitRow {
    organization_id: Uuid,
    ordinal: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct ScopeSnapshotAuthorityRow {
    project_path_at_freeze: String,
    project_scope_id: Uuid,
    scope_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct BarrierUnitRow {
    id: Uuid,
    organization_id: Uuid,
    ordinal: i32,
    status: String,
    review_closed: bool,
    verification_closed: bool,
    consolidation_status: String,
    row_version: i64,
    terminal_at: Option<DateTime<Utc>>,
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct FactDeltaCandidateRow {
    id: Uuid,
    source_attempt_id: Uuid,
    candidate_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    candidate_plan_hash: String,
    canonical_ref_kind: String,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: String,
    delta_kind: String,
    dedupe_hash: String,
    status: String,
    attempt_status: String,
    attempt_created_at: DateTime<Utc>,
    attempt_terminal_at: Option<DateTime<Utc>>,
    scope_ordinal: i32,
    delta_evidence_ids: Vec<i64>,
    attempt_fact_delta_evidence_ids: Vec<i64>,
    delta_evidence_within_attempt: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ConsolidationRow {
    id: Uuid,
    decision_kind: String,
    target_wave_run_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct AcceptedFactDeltaAuthorityRow {
    decision_hash: String,
    accepted_at: DateTime<Utc>,
}

#[derive(Debug)]
enum DeltaValidation {
    Accepted {
        resolved_version: i64,
        resolved_hash: String,
    },
    Rejected {
        reason_code: String,
        resolved_version: Option<i64>,
        resolved_hash: Option<String>,
    },
}

const CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE: &str = "GOLISH-SURFACE-ANALYSIS";
const VERIFY_EXECUTE_CANDIDATE_ACTION: &str = "verify_execute_candidate_action";
const NUCLEI_REPLAY_EVIDENCE_KIND: &str = "verification.nuclei_template_replay_v1";
const ANONYMOUS_REPLAY_EVIDENCE_KIND: &str = "verification.anonymous_request_replay_v1";
const WEB_TECHNIQUES: &[&str] = &[
    "WSTG-INPV-05",
    "WSTG-INPV-01",
    "WSTG-INPV-12",
    "WSTG-ATHN-04",
    "WSTG-ATHN-02",
    "WSTG-SESS-02",
    "WSTG-CONF-05",
    "WSTG-CRYP-03",
    "WSTG-INFO",
    "GOLISH-NDAY",
];
const HOST_TECHNIQUES: &[&str] = &["WSTG-CONF-05", "WSTG-CRYP-03", "GOLISH-NDAY"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactDeltaFollowOnWorkItem {
    delta_kind: String,
    observation_kind: String,
    allowed_techniques: Vec<String>,
    enrichment_required: bool,
    technique: String,
    observation: serde_json::Value,
    evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FactDeltaFollowOnRoute {
    /// A refutation withdraws or lowers confidence in prior truth. It remains
    /// an accepted memory delta but must not create a new attack work item.
    NoAttack,
    /// Exact current verifier evidence can become one classifier-supported
    /// typed Candidate observation immediately.
    DirectWorkItem(FactDeltaFollowOnWorkItem),
    /// Insufficient information is persisted as a bounded pending authority.
    /// It deliberately does not freeze an unexecutable Candidate manifest.
    PendingEnrichment(FactDeltaFollowOnWorkItem),
}

#[derive(Debug, Clone, Copy)]
struct FactDeltaRouteEvidence<'a> {
    evidence_id: i64,
    tool_name: Option<&'a str>,
    kind: Option<&'a str>,
    raw_output: Option<&'a str>,
    evidence_technique: Option<&'a str>,
    evidence_outcome: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct FactDeltaRouteInput<'a> {
    fact_delta_id: Uuid,
    delta_kind: &'a str,
    canonical_ref_kind: &'a str,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: &'a str,
    target_live_id: Option<Uuid>,
    target_type_at_time: &'a str,
    target_value_at_time: &'a str,
    target_identity_hash: &'a str,
    evidence: &'a [FactDeltaRouteEvidence<'a>],
}

fn classifier_supported_techniques(target_type: &str) -> Vec<String> {
    let techniques = match target_type.trim().to_ascii_lowercase().as_str() {
        "ip" => HOST_TECHNIQUES,
        "url" | "domain" | "wildcard" | "other" => WEB_TECHNIQUES,
        _ => &[],
    };
    techniques
        .iter()
        .map(|technique| (*technique).to_string())
        .collect()
}

fn add_fact_delta_route_metadata(
    observation: &mut serde_json::Value,
    input: &FactDeltaRouteInput<'_>,
    observation_kind: &str,
    allowed_techniques: &[String],
    enrichment_required: bool,
) -> Result<(), &'static str> {
    let object = observation
        .as_object_mut()
        .ok_or("attack_fact_delta_route_unsupported")?;
    object.insert(
        "allowed_techniques".to_string(),
        serde_json::json!(allowed_techniques),
    );
    object.insert(
        "canonical_ref".to_string(),
        serde_json::json!({
            "hash": input.canonical_ref_hash,
            "id": input.canonical_ref_id,
            "kind": input.canonical_ref_kind,
            "version": input.canonical_ref_version,
        }),
    );
    object.insert(
        "delta_kind".to_string(),
        serde_json::json!(input.delta_kind),
    );
    object.insert(
        "enrichment_required".to_string(),
        serde_json::json!(enrichment_required),
    );
    object.insert(
        "evidence_ids".to_string(),
        serde_json::json!(input
            .evidence
            .iter()
            .map(|evidence| evidence.evidence_id)
            .collect::<Vec<_>>()),
    );
    object.insert(
        "fact_delta_id".to_string(),
        serde_json::json!(input.fact_delta_id),
    );
    object.insert(
        "observation_kind".to_string(),
        serde_json::json!(observation_kind),
    );
    Ok(())
}

fn typed_nuclei_follow_on(
    input: &FactDeltaRouteInput<'_>,
    evidence: &FactDeltaRouteEvidence<'_>,
) -> Result<FactDeltaFollowOnWorkItem, &'static str> {
    let raw: serde_json::Value = serde_json::from_str(
        evidence
            .raw_output
            .ok_or("attack_fact_delta_route_unsupported")?,
    )
    .map_err(|_| "attack_fact_delta_route_unsupported")?;
    let result = raw
        .get("result")
        .filter(|value| value.is_object())
        .ok_or("attack_fact_delta_route_unsupported")?;
    let target_id = result
        .get("target_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let technique = result
        .get("technique")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("attack_fact_delta_route_unsupported")?;
    let matched_url = result
        .get("matched_url")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("attack_fact_delta_route_unsupported")?;
    let template_id = result
        .get("template_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("attack_fact_delta_route_unsupported")?;
    let target_origin = golish_pentest_domain::canonical_web_origin(input.target_value_at_time)
        .ok_or("attack_fact_delta_route_unsupported")?;
    let matched_origin = golish_pentest_domain::canonical_web_origin(matched_url)
        .ok_or("attack_fact_delta_route_unsupported")?;
    let allowed_techniques = classifier_supported_techniques(input.target_type_at_time);
    if raw.get("schema").and_then(serde_json::Value::as_str) != Some(NUCLEI_REPLAY_EVIDENCE_KIND)
        || raw.get("evidence_role").and_then(serde_json::Value::as_str) != Some("proof")
        || evidence.evidence_outcome != Some("found")
        || evidence.evidence_technique != Some(technique)
        || result.get("completion").and_then(serde_json::Value::as_str) != Some("complete")
        || result
            .get("match_count")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|count| count == 0)
        || result
            .get("matches")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
        || input.target_live_id != target_id
        || target_origin.key != matched_origin.key
        || !allowed_techniques
            .iter()
            .any(|allowed| allowed == technique)
    {
        return Err("attack_fact_delta_route_unsupported");
    }
    let allowed_techniques = vec![technique.to_string()];
    let mut observation = serde_json::json!({
        "schema": "nuclei_match_v1",
        "target_id": target_id,
        "matched_url": matched_url,
        "template_id": template_id,
        "technique": technique,
    });
    add_fact_delta_route_metadata(
        &mut observation,
        input,
        "nuclei_match_v1",
        &allowed_techniques,
        false,
    )?;
    Ok(FactDeltaFollowOnWorkItem {
        delta_kind: input.delta_kind.to_string(),
        observation_kind: "nuclei_match_v1".to_string(),
        allowed_techniques,
        enrichment_required: false,
        technique: technique.to_string(),
        observation,
        evidence_ids: vec![evidence.evidence_id],
    })
}

fn typed_anonymous_follow_on(
    input: &FactDeltaRouteInput<'_>,
    evidence: &FactDeltaRouteEvidence<'_>,
) -> Result<FactDeltaFollowOnWorkItem, &'static str> {
    let raw: serde_json::Value = serde_json::from_str(
        evidence
            .raw_output
            .ok_or("attack_fact_delta_route_unsupported")?,
    )
    .map_err(|_| "attack_fact_delta_route_unsupported")?;
    let mut observation = raw
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or("attack_fact_delta_route_unsupported")?;
    let technique = "WSTG-ATHN-04";
    let allowed_for_target = classifier_supported_techniques(input.target_type_at_time);
    if raw.get("schema").and_then(serde_json::Value::as_str) != Some(ANONYMOUS_REPLAY_EVIDENCE_KIND)
        || raw.get("evidence_role").and_then(serde_json::Value::as_str) != Some("proof")
        || evidence.evidence_outcome != Some("found")
        || evidence.evidence_technique != Some(technique)
        || observation
            .get("schema")
            .and_then(serde_json::Value::as_str)
            != Some("anonymous_access_v1")
        || observation
            .get("no_auth")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || observation
            .get("network_attempted")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || observation
            .get("authority_current_after")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || observation
            .get("verdict")
            .and_then(serde_json::Value::as_str)
            != Some("suspicious")
        || !allowed_for_target
            .iter()
            .any(|allowed| allowed == technique)
    {
        return Err("attack_fact_delta_route_unsupported");
    }
    let allowed_techniques = vec![technique.to_string()];
    add_fact_delta_route_metadata(
        &mut observation,
        input,
        "anonymous_access_v1",
        &allowed_techniques,
        false,
    )?;
    Ok(FactDeltaFollowOnWorkItem {
        delta_kind: input.delta_kind.to_string(),
        observation_kind: "anonymous_access_v1".to_string(),
        allowed_techniques,
        enrichment_required: false,
        technique: technique.to_string(),
        observation,
        evidence_ids: vec![evidence.evidence_id],
    })
}

fn classify_fact_delta_follow_on(
    input: &FactDeltaRouteInput<'_>,
) -> Result<FactDeltaFollowOnRoute, &'static str> {
    match input.delta_kind {
        "refuted" => return Ok(FactDeltaFollowOnRoute::NoAttack),
        "created" | "updated" | "new_surface" => {}
        _ => return Err("attack_fact_delta_route_unsupported"),
    }
    let mut typed = None;
    for evidence in input.evidence {
        let recognized = evidence
            .kind
            .is_some_and(|kind| kind.starts_with("verification."))
            || evidence.tool_name == Some(VERIFY_EXECUTE_CANDIDATE_ACTION);
        if !recognized {
            continue;
        }
        if evidence.tool_name != Some(VERIFY_EXECUTE_CANDIDATE_ACTION) {
            return Err("attack_fact_delta_route_unsupported");
        }
        let route = match evidence.kind {
            Some(NUCLEI_REPLAY_EVIDENCE_KIND) => typed_nuclei_follow_on(input, evidence)?,
            Some(ANONYMOUS_REPLAY_EVIDENCE_KIND) => typed_anonymous_follow_on(input, evidence)?,
            _ => return Err("attack_fact_delta_route_unsupported"),
        };
        if typed.replace(route).is_some() {
            return Err("attack_fact_delta_route_unsupported");
        }
    }
    if let Some(typed) = typed {
        return Ok(FactDeltaFollowOnRoute::DirectWorkItem(typed));
    }
    let allowed_techniques = classifier_supported_techniques(input.target_type_at_time);
    if allowed_techniques.is_empty() {
        return Err("attack_fact_delta_route_unsupported");
    }
    let mut observation = serde_json::json!({
        "schema": "surface_analysis_v2",
        "target_id": input.target_live_id,
        "target_identity": {
            "type": input.target_type_at_time,
            "value": input.target_value_at_time,
            "sha256": input.target_identity_hash,
        },
        "formulaic_coverage": [],
        "upstream_query_required": true,
    });
    add_fact_delta_route_metadata(
        &mut observation,
        input,
        "surface_analysis_v2",
        &allowed_techniques,
        true,
    )?;
    Ok(FactDeltaFollowOnRoute::PendingEnrichment(
        FactDeltaFollowOnWorkItem {
            delta_kind: input.delta_kind.to_string(),
            observation_kind: "surface_analysis_v2".to_string(),
            allowed_techniques,
            enrichment_required: true,
            technique: CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE.to_string(),
            observation,
            evidence_ids: input
                .evidence
                .iter()
                .map(|evidence| evidence.evidence_id)
                .collect(),
        },
    ))
}

#[derive(Debug, sqlx::FromRow)]
struct FactDeltaRouteEvidenceRow {
    id: i64,
    audit_role: String,
    run_id: Option<Uuid>,
    target_id: Option<Uuid>,
    project_path: Option<String>,
    tool_name: Option<String>,
    kind: Option<String>,
    raw_output: Option<String>,
    organization_id: Option<Uuid>,
    evidence_technique: Option<String>,
    evidence_outcome: Option<String>,
}

async fn route_accepted_fact_delta(
    tx: &mut Transaction<'_, Postgres>,
    project_path_at_freeze: &str,
    delta: &FactDeltaCandidateRow,
) -> crate::Result<FactDeltaFollowOnRoute> {
    if delta.delta_kind == "refuted" {
        return Ok(FactDeltaFollowOnRoute::NoAttack);
    }
    let rows = sqlx::query_as::<_, FactDeltaRouteEvidenceRow>(
        r#"SELECT id,audit_role,run_id,target_id,project_path,tool_name,
                  detail->>'kind' AS kind,detail->>'raw_output' AS raw_output,
                  NULLIF(detail->>'organization_id','')::UUID AS organization_id,
                  evidence_technique,evidence_outcome
             FROM audit_log
            WHERE id=ANY($1)
            ORDER BY id
            FOR SHARE"#,
    )
    .bind(&delta.delta_evidence_ids)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() != delta.delta_evidence_ids.len()
        || rows.iter().any(|evidence| {
            evidence.audit_role != "evidence"
                || evidence.run_id != Some(delta.operation_id)
                || evidence.target_id != delta.target_live_id
                || evidence.project_path.as_deref() != Some(project_path_at_freeze)
                || evidence.organization_id != Some(delta.organization_id)
        })
    {
        return Err(conflict("attack_fact_delta_route_evidence_identity_drift"));
    }
    let evidence = rows
        .iter()
        .map(|row| FactDeltaRouteEvidence {
            evidence_id: row.id,
            tool_name: row.tool_name.as_deref(),
            kind: row.kind.as_deref(),
            raw_output: row.raw_output.as_deref(),
            evidence_technique: row.evidence_technique.as_deref(),
            evidence_outcome: row.evidence_outcome.as_deref(),
        })
        .collect::<Vec<_>>();
    let mut route = classify_fact_delta_follow_on(&FactDeltaRouteInput {
        fact_delta_id: delta.id,
        delta_kind: &delta.delta_kind,
        canonical_ref_kind: &delta.canonical_ref_kind,
        canonical_ref_id: delta.canonical_ref_id,
        canonical_ref_version: delta.canonical_ref_version,
        canonical_ref_hash: &delta.canonical_ref_hash,
        target_live_id: delta.target_live_id,
        target_type_at_time: &delta.target_type_at_time,
        target_value_at_time: &delta.target_value_at_time,
        target_identity_hash: &delta.target_identity_hash,
        evidence: &evidence,
    })
    .map_err(conflict)?;
    if let FactDeltaFollowOnRoute::DirectWorkItem(work_item)
    | FactDeltaFollowOnRoute::PendingEnrichment(work_item) = &mut route
    {
        // The typed adapter chooses the observation; the WorkItem still keeps
        // the complete immutable FactDelta evidence set for audit/replay.
        work_item.evidence_ids = delta.delta_evidence_ids.clone();
    }
    Ok(route)
}

fn conflict(code: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(code.to_string()))
}

fn canonical_hash(value: &serde_json::Value) -> String {
    format!("sha256:{}", operation_scope_decisions::sha256_json(value))
}

fn max_wave_count_reached(source_generation: i32, max_waves: i32) -> bool {
    source_generation
        .checked_add(1)
        .is_none_or(|current_wave_count| current_wave_count >= max_waves)
}

fn fact_delta_graph_predicate(canonical_ref_kind: &str) -> &'static str {
    match canonical_ref_kind {
        "api_endpoint" | "directory_entry" => "graph.entity.endpoint",
        "fingerprint" => "graph.entity.service",
        "finding" => "graph.entity.finding",
        _ => "graph.entity.target",
    }
}

fn safe_fact_delta_display_name(delta: &FactDeltaCandidateRow, canonical_ref: &str) -> String {
    let candidate = delta.target_value_at_time.trim();
    if !candidate.is_empty() && candidate.len() <= 256 && !candidate.chars().any(char::is_control) {
        candidate.to_string()
    } else {
        canonical_ref.to_string()
    }
}

async fn promote_accepted_fact_delta_memories(
    tx: &mut Transaction<'_, Postgres>,
    scope: &ScopeSnapshotAuthorityRow,
    consolidation_id: Uuid,
    consolidation_decision_kind: &str,
    target_wave_run_id: Option<Uuid>,
    accepted: &[FactDeltaCandidateRow],
) -> crate::Result<()> {
    for delta in accepted {
        let authority = sqlx::query_as::<_, AcceptedFactDeltaAuthorityRow>(
            r#"SELECT decision.decision_hash,delta.accepted_at
                 FROM attack_fact_delta_decisions AS decision
                 JOIN attack_fact_deltas AS delta ON delta.id=decision.fact_delta_id
                WHERE decision.fact_delta_id=$1
                  AND decision.operation_id=$2
                  AND decision.scope_snapshot_id=$3
                  AND decision.source_wave_run_id=$4
                  AND decision.organization_id=$5
                  AND decision.disposition='accepted'
                  AND delta.status IN ('accepted','consumed')"#,
        )
        .bind(delta.id)
        .bind(delta.operation_id)
        .bind(delta.scope_snapshot_id)
        .bind(delta.wave_run_id)
        .bind(delta.organization_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("fact_delta_memory_authority_missing"))?;
        let source = SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Uuid(delta.id),
            source_stream_key: format!("fact-delta:{}", delta.id),
            version: 1,
        };
        let structured_payload = serde_json::json!({
            "candidate_id": delta.candidate_id,
            "canonical_ref": {
                "hash": delta.canonical_ref_hash,
                "id": delta.canonical_ref_id,
                "kind": delta.canonical_ref_kind,
                "version": delta.canonical_ref_version,
            },
            "consolidation_decision_kind": consolidation_decision_kind,
            "consolidation_id": consolidation_id,
            "delta_kind": delta.delta_kind,
            "evidence_ids": delta.delta_evidence_ids,
            "fact_delta_decision_hash": authority.decision_hash,
            "fact_delta_id": delta.id,
            "frozen_target": {
                "identity_hash": delta.target_identity_hash,
                "type": delta.target_type_at_time,
                "value": delta.target_value_at_time,
            },
            "source_attempt_id": delta.source_attempt_id,
            "source_wave_run_id": delta.wave_run_id,
            "source_wave_unit_id": delta.wave_unit_id,
            "target_wave_run_id": target_wave_run_id,
        });
        let canonical_ref = format!("{}:{}", delta.canonical_ref_kind, delta.canonical_ref_id);
        let object = AssertionObject::Json(serde_json::json!({
            "canonical_ref": canonical_ref,
            "display_name": safe_fact_delta_display_name(delta, &canonical_ref),
            "fact_delta": structured_payload,
            "properties": {
                "canonical_ref_kind": delta.canonical_ref_kind,
                "canonical_ref_version": delta.canonical_ref_version,
                "delta_kind": delta.delta_kind,
                "target_identity_hash": delta.target_identity_hash,
                "target_type_at_time": delta.target_type_at_time,
            },
        }));
        let identity = AssertionIdentity::derive(
            format!("fact_delta:{}", delta.id),
            fact_delta_graph_predicate(&delta.canonical_ref_kind),
            &object,
        )
        .map_err(|error| {
            conflict(&format!(
                "fact_delta_memory_identity_invalid:{}",
                error.code()
            ))
        })?;
        let assertion_id = Uuid::new_v5(
            &delta.id,
            format!("FactDeltaAccepted.v1:assertion:{}", identity.identity_hash).as_bytes(),
        );
        let assertion = KnowledgeAssertionDraft {
            assertion_id,
            visibility: AssertionVisibility::OrganizationLongTerm {
                project_scope_id: ProjectScopeId(scope.project_scope_id),
                organization_id_at_time: delta.organization_id,
            },
            source_operation_id: delta.operation_id,
            source_scope_snapshot_hash: scope.scope_hash.clone(),
            source: source.clone(),
            identity,
            kind: AssertionKind::Observation,
            status: AssertionStatus::Active,
            object,
            classification: KnowledgeClassification::CustomerConfidential,
            evidence_ids: delta.delta_evidence_ids.clone(),
            valid_from: authority.accepted_at,
            valid_to: None,
            fresh_until: None,
        }
        .validate()
        .map_err(|error| {
            conflict(&format!(
                "fact_delta_memory_assertion_invalid:{}",
                error.code()
            ))
        })?;
        let event = KnowledgeEventEnvelopeV1 {
            event_id: Uuid::new_v5(&delta.id, b"FactDeltaAccepted.v1"),
            project_scope_id: Some(ProjectScopeId(scope.project_scope_id)),
            organization_id_at_time: Some(delta.organization_id),
            source_operation_id: delta.operation_id,
            event_name: KnowledgeEventNameV1::FactDeltaAccepted,
            schema_version: 1,
            payload: KnowledgeEventPayloadV1 {
                source_stream_key: source.source_stream_key.clone(),
                source_version: source.version,
                source,
                structured_payload,
            },
            occurred_at: authority.accepted_at,
        };
        knowledge_assertions::promote_assertion_with_event_with_connection(tx, &assertion, &event)
            .await
            .map_err(|error| {
                conflict(&format!(
                    "fact_delta_memory_promotion_failed:{}",
                    error.code()
                ))
            })?;
    }
    Ok(())
}

fn canonical_key(kind: &str, id: Uuid) -> Option<canonical_fact_refs::CanonicalFactKey> {
    match kind {
        "target" => Some(canonical_fact_refs::CanonicalFactKey::Target { target_id: id }),
        "target_asset" => Some(canonical_fact_refs::CanonicalFactKey::TargetAsset {
            target_asset_id: id,
        }),
        "api_endpoint" => Some(canonical_fact_refs::CanonicalFactKey::ApiEndpoint {
            api_endpoint_id: id,
        }),
        "directory_entry" => Some(canonical_fact_refs::CanonicalFactKey::DirectoryEntry {
            directory_entry_id: id,
        }),
        "js_analysis_result" => Some(canonical_fact_refs::CanonicalFactKey::JsAnalysisResult {
            js_analysis_result_id: id,
        }),
        "fingerprint" => {
            Some(canonical_fact_refs::CanonicalFactKey::Fingerprint { fingerprint_id: id })
        }
        "attack_candidate_work_item" => Some(
            canonical_fact_refs::CanonicalFactKey::AttackCandidateWorkItem { work_item_id: id },
        ),
        "finding" => Some(canonical_fact_refs::CanonicalFactKey::Finding { finding_id: id }),
        _ => None,
    }
}

fn fact_delta_dedupe_matches(
    target_identity_hash: &str,
    canonical_ref_kind: &str,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: &str,
    delta_kind: &str,
    persisted_dedupe_hash: &str,
) -> crate::Result<bool> {
    Ok(attack_fact_deltas::semantic_dedupe_hash(
        target_identity_hash,
        canonical_ref_kind,
        canonical_ref_id,
        canonical_ref_version,
        canonical_ref_hash,
        delta_kind,
    )? == persisted_dedupe_hash)
}

async fn validate_delta(
    tx: &mut Transaction<'_, Postgres>,
    project_path_at_freeze: &str,
    delta: &FactDeltaCandidateRow,
) -> crate::Result<DeltaValidation> {
    if !matches!(
        delta.attempt_status.as_str(),
        "verified" | "refuted" | "blocked"
    ) {
        return Ok(DeltaValidation::Rejected {
            reason_code: "source_attempt_not_terminal".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    }
    let Some(attempt_terminal_at) = delta.attempt_terminal_at else {
        return Ok(DeltaValidation::Rejected {
            reason_code: "source_attempt_not_terminal".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    };
    let attempt_evidence = delta
        .attempt_fact_delta_evidence_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if delta.delta_evidence_ids.is_empty()
        || delta
            .delta_evidence_ids
            .iter()
            .any(|evidence_id| !attempt_evidence.contains(evidence_id))
    {
        return Ok(DeltaValidation::Rejected {
            reason_code: "attempt_evidence_mismatch".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    }
    if !delta.delta_evidence_within_attempt {
        return Ok(DeltaValidation::Rejected {
            reason_code: "evidence_time_mismatch".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    }
    if delta.canonical_ref_version != 1 {
        return Ok(DeltaValidation::Rejected {
            reason_code: "canonical_ref_version_unsupported".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    }
    let Some(delta_kind) = attack_fact_deltas::AttackFactDeltaKind::parse(&delta.delta_kind) else {
        return Ok(DeltaValidation::Rejected {
            reason_code: "delta_kind_unsupported".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    };
    if !fact_delta_dedupe_matches(
        &delta.target_identity_hash,
        &delta.canonical_ref_kind,
        delta.canonical_ref_id,
        delta.canonical_ref_version,
        &delta.canonical_ref_hash,
        delta_kind.as_str(),
        &delta.dedupe_hash,
    )
    .unwrap_or(false)
    {
        return Ok(DeltaValidation::Rejected {
            reason_code: "semantic_dedupe_hash_mismatch".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    }
    let Some(key) = canonical_key(&delta.canonical_ref_kind, delta.canonical_ref_id) else {
        return Ok(DeltaValidation::Rejected {
            reason_code: "canonical_ref_kind_unsupported".to_string(),
            resolved_version: None,
            resolved_hash: None,
        });
    };
    let resolved = match canonical_fact_refs::resolve_for_fact_delta(
        tx,
        delta.operation_id,
        delta.organization_id,
        project_path_at_freeze,
        delta.attempt_created_at,
        attempt_terminal_at,
        delta_kind.as_str(),
        &key,
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(canonical_fact_refs::CanonicalFactRefError::Rejected { code }) => {
            return Ok(DeltaValidation::Rejected {
                reason_code: code.to_string(),
                resolved_version: None,
                resolved_hash: None,
            });
        }
        Err(canonical_fact_refs::CanonicalFactRefError::Sqlx(error)) => return Err(error.into()),
    };
    if resolved.content_sha256 != delta.canonical_ref_hash {
        return Ok(DeltaValidation::Rejected {
            reason_code: "canonical_ref_hash_mismatch".to_string(),
            resolved_version: Some(1),
            resolved_hash: Some(resolved.content_sha256),
        });
    }
    Ok(DeltaValidation::Accepted {
        resolved_version: 1,
        resolved_hash: resolved.content_sha256,
    })
}

async fn persist_delta_decision(
    tx: &mut Transaction<'_, Postgres>,
    delta: &FactDeltaCandidateRow,
    validation: &DeltaValidation,
) -> crate::Result<()> {
    let evidence_set_hash = canonical_hash(&serde_json::json!(delta.delta_evidence_ids));
    let (disposition, reason_code, resolved_version, resolved_hash) = match validation {
        DeltaValidation::Accepted {
            resolved_version,
            resolved_hash,
        } => (
            "accepted",
            "accepted".to_string(),
            Some(*resolved_version),
            Some(resolved_hash.clone()),
        ),
        DeltaValidation::Rejected {
            reason_code,
            resolved_version,
            resolved_hash,
        } => (
            "rejected",
            reason_code.clone(),
            *resolved_version,
            resolved_hash.clone(),
        ),
    };
    if delta.status != "proposed" {
        let existing: Option<(String, String, String)> = sqlx::query_as(
            "SELECT disposition,reason_code,decision_hash
               FROM attack_fact_delta_decisions WHERE fact_delta_id=$1 FOR SHARE",
        )
        .bind(delta.id)
        .fetch_optional(&mut **tx)
        .await?;
        let expected_status = if disposition == "accepted" {
            BTreeSet::from(["accepted", "consumed"])
        } else {
            BTreeSet::from(["rejected"])
        };
        if existing.is_none() || !expected_status.contains(delta.status.as_str()) {
            return Err(conflict("fact_delta_decision_missing_or_drifted"));
        }
        return Ok(());
    }
    let decision_payload = serde_json::json!({
        "canonical_ref_hash": delta.canonical_ref_hash,
        "canonical_ref_id": delta.canonical_ref_id,
        "canonical_ref_kind": delta.canonical_ref_kind,
        "canonical_ref_version": delta.canonical_ref_version,
        "disposition": disposition,
        "evidence_set_hash": evidence_set_hash,
        "fact_delta_id": delta.id,
        "reason_code": reason_code,
        "resolved_ref_hash": resolved_hash,
        "resolved_ref_version": resolved_version,
    });
    let decision_hash = canonical_hash(&decision_payload);
    sqlx::query(
        r#"INSERT INTO attack_fact_delta_decisions (
               fact_delta_id,source_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,source_wave_run_id,source_wave_unit_id,
               organization_id,disposition,reason_code,canonical_ref_kind,
               canonical_ref_id,canonical_ref_version,proposed_ref_hash,
               resolved_ref_version,resolved_ref_hash,evidence_set_hash,decision_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18
           )"#,
    )
    .bind(delta.id)
    .bind(delta.source_attempt_id)
    .bind(delta.candidate_id)
    .bind(delta.operation_id)
    .bind(delta.scope_snapshot_id)
    .bind(delta.wave_run_id)
    .bind(delta.wave_unit_id)
    .bind(delta.organization_id)
    .bind(disposition)
    .bind(&reason_code)
    .bind(&delta.canonical_ref_kind)
    .bind(delta.canonical_ref_id)
    .bind(delta.canonical_ref_version)
    .bind(&delta.canonical_ref_hash)
    .bind(resolved_version)
    .bind(&resolved_hash)
    .bind(&evidence_set_hash)
    .bind(&decision_hash)
    .execute(&mut **tx)
    .await?;
    let materialized = if disposition == "accepted" {
        sqlx::query(
            "UPDATE attack_fact_deltas
                SET status='accepted',accepted_at=NOW(),updated_at=NOW()
              WHERE id=$1 AND status='proposed'",
        )
        .bind(delta.id)
        .execute(&mut **tx)
        .await?
    } else {
        sqlx::query(
            "UPDATE attack_fact_deltas SET status='rejected',updated_at=NOW()
              WHERE id=$1 AND status='proposed'",
        )
        .bind(delta.id)
        .execute(&mut **tx)
        .await?
    };
    if materialized.rows_affected() != 1 {
        return Err(conflict("fact_delta_decision_cas_lost"));
    }
    Ok(())
}

async fn load_existing_result(
    tx: &mut Transaction<'_, Postgres>,
    command: ConsolidateAttackWave,
) -> crate::Result<Option<AttackWaveConsolidationResult>> {
    let existing = sqlx::query_as::<_, ConsolidationRow>(
        "SELECT id,decision_kind,target_wave_run_id
           FROM attack_wave_consolidations
          WHERE source_wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
          FOR SHARE",
    )
    .bind(command.source_wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(existing) = existing else {
        let pending_enrichment_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM attack_fact_delta_enrichment_items
              WHERE operation_id=$1 AND scope_snapshot_id=$2 AND source_wave_run_id=$3
                AND status='pending' ORDER BY id",
        )
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.source_wave_run_id)
        .fetch_all(&mut **tx)
        .await?;
        if pending_enrichment_ids.is_empty() {
            return Ok(None);
        }
        let accepted_fact_delta_ids = sqlx::query_scalar(
            "SELECT fact_delta_id FROM attack_fact_delta_decisions
              WHERE operation_id=$1 AND scope_snapshot_id=$2 AND source_wave_run_id=$3
                AND disposition='accepted' ORDER BY fact_delta_id",
        )
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.source_wave_run_id)
        .fetch_all(&mut **tx)
        .await?;
        let rejected_fact_delta_ids = sqlx::query_scalar(
            "SELECT fact_delta_id FROM attack_fact_delta_decisions
              WHERE operation_id=$1 AND scope_snapshot_id=$2 AND source_wave_run_id=$3
                AND disposition='rejected' ORDER BY fact_delta_id",
        )
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.source_wave_run_id)
        .fetch_all(&mut **tx)
        .await?;
        return Ok(Some(AttackWaveConsolidationResult {
            consolidation_id: Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{}:attack-wave-consolidation", command.source_wave_run_id).as_bytes(),
            ),
            decision_kind: "pending_enrichment".to_string(),
            target_wave_run_id: None,
            accepted_fact_delta_ids,
            rejected_fact_delta_ids,
            residual_risk_ids: Vec::new(),
            pending_enrichment_count: pending_enrichment_ids.len(),
            replayed: true,
        }));
    };
    let accepted_fact_delta_ids = sqlx::query_scalar(
        "SELECT fact_delta_id FROM attack_wave_consolidation_members
          WHERE consolidation_id=$1 ORDER BY ordinal",
    )
    .bind(existing.id)
    .fetch_all(&mut **tx)
    .await?;
    let rejected_fact_delta_ids = sqlx::query_scalar(
        "SELECT fact_delta_id FROM attack_fact_delta_decisions
          WHERE source_wave_run_id=$1 AND disposition='rejected' ORDER BY fact_delta_id",
    )
    .bind(command.source_wave_run_id)
    .fetch_all(&mut **tx)
    .await?;
    let residual_risk_ids = sqlx::query_scalar(
        "SELECT residual_risk_id FROM attack_wave_consolidation_members
          WHERE consolidation_id=$1 AND residual_risk_id IS NOT NULL ORDER BY ordinal",
    )
    .bind(existing.id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(Some(AttackWaveConsolidationResult {
        consolidation_id: existing.id,
        decision_kind: existing.decision_kind,
        target_wave_run_id: existing.target_wave_run_id,
        accepted_fact_delta_ids,
        rejected_fact_delta_ids,
        residual_risk_ids,
        pending_enrichment_count: 0,
        replayed: true,
    }))
}

async fn persist_pending_fact_delta_enrichments(
    tx: &mut Transaction<'_, Postgres>,
    pending: &BTreeMap<Uuid, FactDeltaFollowOnWorkItem>,
    accepted: &[FactDeltaCandidateRow],
) -> crate::Result<()> {
    for delta in accepted {
        let Some(route) = pending.get(&delta.id) else {
            continue;
        };
        let enrichment_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}:fact-delta-enrichment", delta.id).as_bytes(),
        );
        let request_hash = canonical_hash(&route.observation);
        sqlx::query(
            r#"INSERT INTO attack_fact_delta_enrichment_items (
                   id,fact_delta_id,source_attempt_id,candidate_id,operation_id,
                   scope_snapshot_id,source_wave_run_id,source_wave_unit_id,
                   organization_id,delta_kind,observation_kind,allowed_techniques,
                   enrichment_required,status,request,request_hash
               ) VALUES (
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,TRUE,'pending',$13,$14
               )
               ON CONFLICT(fact_delta_id) DO NOTHING"#,
        )
        .bind(enrichment_id)
        .bind(delta.id)
        .bind(delta.source_attempt_id)
        .bind(delta.candidate_id)
        .bind(delta.operation_id)
        .bind(delta.scope_snapshot_id)
        .bind(delta.wave_run_id)
        .bind(delta.wave_unit_id)
        .bind(delta.organization_id)
        .bind(&route.delta_kind)
        .bind(&route.observation_kind)
        .bind(&route.allowed_techniques)
        .bind(&route.observation)
        .bind(&request_hash)
        .execute(&mut **tx)
        .await?;
        let exact: bool = sqlx::query_scalar(
            r#"SELECT id=$2
                      AND operation_id=$3
                      AND scope_snapshot_id=$4
                      AND source_wave_run_id=$5
                      AND source_wave_unit_id=$6
                      AND organization_id=$7
                      AND delta_kind=$8
                      AND observation_kind=$9
                      AND allowed_techniques=$10
                      AND enrichment_required
                      AND status='pending'
                      AND request=$11
                      AND request_hash=$12
                 FROM attack_fact_delta_enrichment_items
                WHERE fact_delta_id=$1
                FOR SHARE"#,
        )
        .bind(delta.id)
        .bind(enrichment_id)
        .bind(delta.operation_id)
        .bind(delta.scope_snapshot_id)
        .bind(delta.wave_run_id)
        .bind(delta.wave_unit_id)
        .bind(delta.organization_id)
        .bind(&route.delta_kind)
        .bind(&route.observation_kind)
        .bind(&route.allowed_techniques)
        .bind(&route.observation)
        .bind(&request_hash)
        .fetch_one(&mut **tx)
        .await?;
        if !exact {
            return Err(conflict("attack_fact_delta_enrichment_replay_drift"));
        }
    }
    Ok(())
}

async fn insert_no_attack_consolidation_members(
    tx: &mut Transaction<'_, Postgres>,
    consolidation_id: Uuid,
    command: ConsolidateAttackWave,
    accepted: &[FactDeltaCandidateRow],
    member_ordinals: &BTreeMap<Uuid, i32>,
) -> crate::Result<()> {
    for delta in accepted
        .iter()
        .filter(|delta| delta.delta_kind == "refuted")
    {
        let ordinal = *member_ordinals
            .get(&delta.id)
            .ok_or_else(|| conflict("attack_fact_delta_ordinal_missing"))?;
        let member_hash = canonical_hash(&serde_json::json!({
            "consolidation_id": consolidation_id,
            "fact_delta_id": delta.id,
            "ordinal": ordinal,
            "organization_id": delta.organization_id,
            "route_kind": "no_attack",
        }));
        sqlx::query(
            r#"INSERT INTO attack_wave_consolidation_members (
                   consolidation_id,ordinal,fact_delta_id,source_attempt_id,
                   candidate_id,operation_id,scope_snapshot_id,source_wave_run_id,
                   source_wave_unit_id,organization_id,route_kind,member_hash
               ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'no_attack',$11)"#,
        )
        .bind(consolidation_id)
        .bind(ordinal)
        .bind(delta.id)
        .bind(delta.source_attempt_id)
        .bind(delta.candidate_id)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.source_wave_run_id)
        .bind(delta.wave_unit_id)
        .bind(delta.organization_id)
        .bind(member_hash)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn delta_set_projection(delta: &FactDeltaCandidateRow) -> serde_json::Value {
    serde_json::json!({
        "canonical_ref_hash": delta.canonical_ref_hash,
        "canonical_ref_id": delta.canonical_ref_id,
        "canonical_ref_kind": delta.canonical_ref_kind,
        "canonical_ref_version": delta.canonical_ref_version,
        "candidate_plan_hash": delta.candidate_plan_hash,
        "dedupe_hash": delta.dedupe_hash,
        "delta_kind": delta.delta_kind,
        "evidence_ids": delta.delta_evidence_ids,
        "fact_delta_id": delta.id,
        "organization_id": delta.organization_id,
        "scope_ordinal": delta.scope_ordinal,
        "source_attempt_id": delta.source_attempt_id,
        "source_wave_unit_id": delta.wave_unit_id,
    })
}

#[allow(clippy::too_many_arguments)]
async fn insert_consolidation_parent(
    tx: &mut Transaction<'_, Postgres>,
    consolidation_id: Uuid,
    command: ConsolidateAttackWave,
    source_wave: &attack_waves::AttackWaveRunRow,
    decision_kind: &str,
    target_wave_run_id: Option<Uuid>,
    target_generation: Option<i32>,
    source_barrier_hash: &str,
    fact_delta_set_hash: &str,
    fact_delta_count: i32,
    wave_count: i32,
    candidate_count: i32,
    chain_depth: i32,
    attempt_count: i32,
    reason_code: &str,
) -> crate::Result<()> {
    let decision_hash = canonical_hash(&serde_json::json!({
        "candidate_count": candidate_count,
        "chain_depth": chain_depth,
        "decision_kind": decision_kind,
        "fact_delta_count": fact_delta_count,
        "fact_delta_set_hash": fact_delta_set_hash,
        "operation_id": command.operation_id,
        "policy_hash": source_wave.policy_hash,
        "reason_code": reason_code,
        "scope_snapshot_id": command.scope_snapshot_id,
        "source_barrier_hash": source_barrier_hash,
        "source_wave_run_id": command.source_wave_run_id,
        "source_wave_version_before": source_wave.row_version,
        "target_wave_run_id": target_wave_run_id,
        "wave_count": wave_count,
        "attempt_count": attempt_count,
    }));
    sqlx::query(
        r#"INSERT INTO attack_wave_consolidations (
               id,operation_id,scope_snapshot_id,source_wave_run_id,
               source_generation,decision_kind,target_wave_run_id,target_generation,
               source_wave_version_before,source_wave_version_after,source_barrier_hash,
               policy_hash,fact_delta_set_hash,fact_delta_count,wave_count,
               candidate_count,chain_depth,attempt_count,reason_code,decision_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$9+1,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19
           )"#,
    )
    .bind(consolidation_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.source_wave_run_id)
    .bind(source_wave.generation)
    .bind(decision_kind)
    .bind(target_wave_run_id)
    .bind(target_generation)
    .bind(source_wave.row_version)
    .bind(source_barrier_hash)
    .bind(&source_wave.policy_hash)
    .bind(fact_delta_set_hash)
    .bind(fact_delta_count)
    .bind(wave_count)
    .bind(candidate_count)
    .bind(chain_depth)
    .bind(attempt_count)
    .bind(reason_code)
    .bind(decision_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn close_source_wave(
    tx: &mut Transaction<'_, Postgres>,
    source_wave: &attack_waves::AttackWaveRunRow,
    barrier_units: &[BarrierUnitRow],
) -> crate::Result<()> {
    let unit_ids = barrier_units
        .iter()
        .filter(|unit| unit.status == "verification")
        .map(|unit| unit.id)
        .collect::<Vec<_>>();
    let units = sqlx::query(
        r#"UPDATE attack_wave_units
              SET status='terminal',consolidation_status='terminal',terminal_at=NOW(),
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=ANY($1) AND wave_run_id=$2 AND status='verification'
              AND review_closed AND verification_closed
              AND consolidation_status='ready'"#,
    )
    .bind(&unit_ids)
    .bind(source_wave.id)
    .execute(&mut **tx)
    .await?;
    if units.rows_affected() != unit_ids.len() as u64 {
        return Err(conflict("attack_wave_unit_close_cas_lost"));
    }
    let wave = sqlx::query(
        r#"UPDATE attack_wave_runs
              SET status='terminal',terminal_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND row_version=$2 AND status='verification'"#,
    )
    .bind(source_wave.id)
    .bind(source_wave.row_version)
    .execute(&mut **tx)
    .await?;
    if wave.rows_affected() != 1 {
        return Err(conflict("attack_wave_close_cas_lost"));
    }
    Ok(())
}

/// Consolidate one exact source Wave. The caller owns the short transaction;
/// this function is the only production seam allowed to advance the durable
/// Candidate Wave cursor.
pub async fn consolidate_attack_wave(
    tx: &mut Transaction<'_, Postgres>,
    command: ConsolidateAttackWave,
) -> crate::Result<AttackWaveConsolidationResult> {
    let contracts: Option<(String, String)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract
           FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (runtime_contract, attack_contract) =
        contracts.ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if runtime_contract != "v2_only" || attack_contract != "v2_only" {
        return Err(conflict("attack_wave_contract_not_v2"));
    }
    if let Some(existing) = load_existing_result(tx, command).await? {
        return Ok(existing);
    }

    let source_wave = attack_waves::lock_wave(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.source_wave_run_id,
    )
    .await?;
    if source_wave.status != "verification" || source_wave.terminal_at.is_some() {
        return Err(conflict("attack_wave_not_ready"));
    }
    let scope = sqlx::query_as::<_, ScopeSnapshotAuthorityRow>(
        "SELECT project_path_at_freeze,project_scope_id,scope_hash
           FROM operation_org_scope_snapshots
          WHERE id=$1 AND operation_id=$2 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(command.scope_snapshot_id)
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("attack_wave_not_ready"))?;
    let scope_units = sqlx::query_as::<_, ScopeUnitRow>(
        r#"SELECT scope_unit.organization_id,scope_unit.ordinal
             FROM operation_org_scope_units AS scope_unit
            WHERE scope_unit.snapshot_id=$1
            ORDER BY scope_unit.ordinal,scope_unit.organization_id
            FOR SHARE"#,
    )
    .bind(command.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if scope_units.is_empty() {
        return Err(conflict("attack_wave_not_ready"));
    }
    let barrier_units = sqlx::query_as::<_, BarrierUnitRow>(
        r#"SELECT id,organization_id,ordinal,status,review_closed,verification_closed,
                  consolidation_status,row_version,terminal_at,manifest_hash,
                  manifest_count,manifest_frozen_at
             FROM attack_wave_units
            WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
            ORDER BY ordinal,organization_id
            FOR UPDATE"#,
    )
    .bind(command.source_wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let exact_scope = barrier_units.len() == scope_units.len()
        && barrier_units.iter().zip(&scope_units).all(|(unit, scope)| {
            unit.organization_id == scope.organization_id && unit.ordinal == scope.ordinal
        });
    let every_unit_ready = barrier_units.iter().all(|unit| {
        let active_ready = unit.status == "verification"
            && unit.review_closed
            && unit.verification_closed
            && unit.consolidation_status == "ready"
            && unit.terminal_at.is_none();
        let terminal_no_input = unit.status == "terminal"
            && unit.review_closed
            && unit.verification_closed
            && unit.consolidation_status == "terminal"
            && unit.terminal_at.is_some()
            && unit.manifest_hash.is_none()
            && unit.manifest_count.is_none()
            && unit.manifest_frozen_at.is_none();
        active_ready || terminal_no_input
    });
    if !exact_scope || !every_unit_ready {
        return Err(conflict("attack_wave_not_ready"));
    }
    let source_barrier_hash = canonical_hash(&serde_json::Value::Array(
        barrier_units
            .iter()
            .map(|unit| {
                serde_json::json!({
                    "consolidation_status": unit.consolidation_status,
                    "organization_id": unit.organization_id,
                    "ordinal": unit.ordinal,
                    "review_closed": unit.review_closed,
                    "row_version": unit.row_version,
                    "status": unit.status,
                    "verification_closed": unit.verification_closed,
                    "wave_unit_id": unit.id,
                    "zero_input_terminal": unit.status == "terminal",
                })
            })
            .collect(),
    ));

    let proposals = sqlx::query_as::<_, FactDeltaCandidateRow>(
        r#"SELECT delta.id,delta.source_attempt_id,delta.candidate_id,
                  delta.operation_id,delta.scope_snapshot_id,delta.wave_run_id,
                  delta.wave_unit_id,delta.organization_id,delta.target_live_id,
                  delta.target_type_at_time,delta.target_value_at_time,
                  delta.target_identity_hash,delta.candidate_plan_hash,
                  delta.canonical_ref_kind,delta.canonical_ref_id,
                  delta.canonical_ref_version,delta.canonical_ref_hash,
                  delta.delta_kind,delta.dedupe_hash,delta.status,
                  attempt.status AS attempt_status,
                  attempt.created_at AS attempt_created_at,
                  attempt.terminal_at AS attempt_terminal_at,
                  scope_unit.ordinal AS scope_ordinal,
                  ARRAY(
                      SELECT evidence_id FROM attack_fact_delta_evidence
                       WHERE fact_delta_id=delta.id AND role='fact_delta'
                       ORDER BY evidence_id
                  )::BIGINT[] AS delta_evidence_ids,
                  ARRAY(
                      SELECT evidence_id FROM candidate_attempt_evidence
                       WHERE attempt_id=delta.source_attempt_id AND role='fact_delta'
                       ORDER BY evidence_id
                  )::BIGINT[] AS attempt_fact_delta_evidence_ids,
                  NOT EXISTS (
                      SELECT 1
                        FROM attack_fact_delta_evidence AS link
                        JOIN audit_log AS evidence ON evidence.id=link.evidence_id
                       WHERE link.fact_delta_id=delta.id
                         AND (
                             attempt.terminal_at IS NULL
                             OR evidence.created_at < attempt.created_at
                             OR evidence.created_at > attempt.terminal_at
                         )
                  ) AS delta_evidence_within_attempt
             FROM attack_fact_deltas AS delta
             JOIN candidate_attempts AS attempt
               ON attempt.id=delta.source_attempt_id
              AND attempt.candidate_id=delta.candidate_id
              AND attempt.operation_id=delta.operation_id
              AND attempt.scope_snapshot_id=delta.scope_snapshot_id
              AND attempt.wave_run_id=delta.wave_run_id
              AND attempt.wave_unit_id=delta.wave_unit_id
              AND attempt.organization_id=delta.organization_id
             JOIN operation_org_scope_units AS scope_unit
               ON scope_unit.snapshot_id=delta.scope_snapshot_id
              AND scope_unit.organization_id=delta.organization_id
            WHERE delta.operation_id=$1 AND delta.scope_snapshot_id=$2
              AND delta.wave_run_id=$3 AND delta.status IN ('proposed','accepted')
            ORDER BY scope_unit.ordinal,delta.id
            FOR UPDATE OF delta,attempt"#,
    )
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.source_wave_run_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut accepted = Vec::new();
    let mut rejected_fact_delta_ids = Vec::new();
    for delta in proposals {
        let validation = validate_delta(tx, &scope.project_path_at_freeze, &delta).await?;
        persist_delta_decision(tx, &delta, &validation).await?;
        match validation {
            DeltaValidation::Accepted { .. } => accepted.push(delta),
            DeltaValidation::Rejected { .. } => rejected_fact_delta_ids.push(delta.id),
        }
    }
    let accepted_fact_delta_ids = accepted.iter().map(|delta| delta.id).collect::<Vec<_>>();
    let mut follow_on_routes = BTreeMap::<Uuid, FactDeltaFollowOnWorkItem>::new();
    let mut pending_enrichment_routes = BTreeMap::<Uuid, FactDeltaFollowOnWorkItem>::new();
    for delta in &accepted {
        match route_accepted_fact_delta(tx, &scope.project_path_at_freeze, delta).await? {
            FactDeltaFollowOnRoute::NoAttack => {}
            FactDeltaFollowOnRoute::DirectWorkItem(route) => {
                follow_on_routes.insert(delta.id, route);
            }
            FactDeltaFollowOnRoute::PendingEnrichment(route) => {
                pending_enrichment_routes.insert(delta.id, route);
            }
        }
    }
    let consolidation_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:attack-wave-consolidation", source_wave.id).as_bytes(),
    );
    if !pending_enrichment_routes.is_empty() {
        // A Wave is one atomic cursor step. If even one accepted delta lacks a
        // classifier-supported typed observation, persist only the bounded
        // enrichment authority and leave every source/delta cursor untouched.
        persist_pending_fact_delta_enrichments(tx, &pending_enrichment_routes, &accepted).await?;
        return Ok(AttackWaveConsolidationResult {
            consolidation_id,
            decision_kind: "pending_enrichment".to_string(),
            target_wave_run_id: None,
            accepted_fact_delta_ids,
            rejected_fact_delta_ids,
            residual_risk_ids: Vec::new(),
            pending_enrichment_count: pending_enrichment_routes.len(),
            replayed: false,
        });
    }
    let follow_on_deltas = accepted
        .iter()
        .filter(|delta| follow_on_routes.contains_key(&delta.id))
        .cloned()
        .collect::<Vec<_>>();
    let member_ordinals = accepted
        .iter()
        .enumerate()
        .map(|(ordinal, delta)| {
            i32::try_from(ordinal)
                .map(|ordinal| (delta.id, ordinal))
                .map_err(|_| conflict("attack_fact_delta_set_too_large"))
        })
        .collect::<crate::Result<BTreeMap<_, _>>>()?;
    let fact_delta_set_hash = canonical_hash(&serde_json::Value::Array(
        accepted.iter().map(delta_set_projection).collect(),
    ));
    let fact_delta_count =
        i32::try_from(accepted.len()).map_err(|_| conflict("attack_fact_delta_set_too_large"))?;
    let candidate_count_i64: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_candidates WHERE operation_uuid=$1")
            .bind(command.operation_id)
            .fetch_one(&mut **tx)
            .await?;
    let attempt_count_i64: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM candidate_attempts WHERE operation_id=$1")
            .bind(command.operation_id)
            .fetch_one(&mut **tx)
            .await?;
    let candidate_count = i32::try_from(candidate_count_i64)
        .map_err(|_| conflict("attack_candidate_counter_overflow"))?;
    let attempt_count = i32::try_from(attempt_count_i64)
        .map_err(|_| conflict("attack_attempt_counter_overflow"))?;
    let next_generation = source_wave
        .generation
        .checked_add(1)
        .ok_or_else(|| conflict("attack_wave_generation_overflow"))?;
    let wave_count = next_generation;
    let chain_depth = source_wave.generation;
    let cap_reason = if max_wave_count_reached(source_wave.generation, source_wave.max_waves) {
        Some("max_waves")
    } else if candidate_count >= source_wave.max_candidates_total {
        Some("max_candidates_total")
    } else if next_generation > source_wave.max_chain_depth {
        Some("max_chain_depth")
    } else if attempt_count >= source_wave.max_attempts_total {
        Some("max_attempts_total")
    } else {
        None
    };
    if follow_on_deltas.is_empty() {
        close_source_wave(tx, &source_wave, &barrier_units).await?;
        insert_consolidation_parent(
            tx,
            consolidation_id,
            command,
            &source_wave,
            "closed_no_delta",
            None,
            None,
            &source_barrier_hash,
            &fact_delta_set_hash,
            fact_delta_count,
            wave_count,
            candidate_count,
            chain_depth,
            attempt_count,
            if accepted.is_empty() {
                "no_accepted_fact_delta"
            } else {
                "accepted_refutation_without_attack_follow_on"
            },
        )
        .await?;
        insert_no_attack_consolidation_members(
            tx,
            consolidation_id,
            command,
            &accepted,
            &member_ordinals,
        )
        .await?;
        if !accepted.is_empty() {
            promote_accepted_fact_delta_memories(
                tx,
                &scope,
                consolidation_id,
                "closed_no_delta",
                None,
                &accepted,
            )
            .await?;
        }
        return Ok(AttackWaveConsolidationResult {
            consolidation_id,
            decision_kind: "closed_no_delta".to_string(),
            target_wave_run_id: None,
            accepted_fact_delta_ids,
            rejected_fact_delta_ids,
            residual_risk_ids: Vec::new(),
            pending_enrichment_count: 0,
            replayed: false,
        });
    }
    if let Some(cap_reason) = cap_reason {
        close_source_wave(tx, &source_wave, &barrier_units).await?;
        insert_consolidation_parent(
            tx,
            consolidation_id,
            command,
            &source_wave,
            "exhausted",
            None,
            None,
            &source_barrier_hash,
            &fact_delta_set_hash,
            fact_delta_count,
            wave_count,
            candidate_count,
            chain_depth,
            attempt_count,
            cap_reason,
        )
        .await?;
        let mut residual_risk_ids = Vec::with_capacity(follow_on_deltas.len());
        for delta in &follow_on_deltas {
            let ordinal = *member_ordinals
                .get(&delta.id)
                .ok_or_else(|| conflict("attack_fact_delta_ordinal_missing"))?;
            let residual_risk_id = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{consolidation_id}:{}:residual", delta.id).as_bytes(),
            );
            sqlx::query(
                r#"INSERT INTO attack_residual_risks (
                       id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
                       organization_id,target_live_id,target_type_at_time,
                       target_value_at_time,target_identity_hash,reason_code,
                       reason_detail,policy_hash,wave_count,candidate_count,
                       chain_depth,attempt_count
                   ) VALUES (
                       $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17
                   )"#,
            )
            .bind(residual_risk_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.source_wave_run_id)
            .bind(delta.wave_unit_id)
            .bind(delta.organization_id)
            .bind(delta.target_live_id)
            .bind(&delta.target_type_at_time)
            .bind(&delta.target_value_at_time)
            .bind(&delta.target_identity_hash)
            .bind(cap_reason)
            .bind(format!(
                "accepted FactDelta was not consumed because {cap_reason} was reached"
            ))
            .bind(&source_wave.policy_hash)
            .bind(wave_count)
            .bind(candidate_count)
            .bind(chain_depth)
            .bind(attempt_count)
            .execute(&mut **tx)
            .await?;
            for evidence_id in &delta.delta_evidence_ids {
                sqlx::query(
                    "INSERT INTO attack_residual_risk_evidence(
                         residual_risk_id,evidence_id,role
                     ) VALUES($1,$2,'residual')",
                )
                .bind(residual_risk_id)
                .bind(evidence_id)
                .execute(&mut **tx)
                .await?;
            }
            let member_hash = canonical_hash(&serde_json::json!({
                "consolidation_id": consolidation_id,
                "fact_delta_id": delta.id,
                "ordinal": ordinal,
                "organization_id": delta.organization_id,
                "residual_risk_id": residual_risk_id,
                "route_kind": "residual",
            }));
            sqlx::query(
                r#"INSERT INTO attack_wave_consolidation_members (
                       consolidation_id,ordinal,fact_delta_id,source_attempt_id,
                       candidate_id,operation_id,scope_snapshot_id,source_wave_run_id,
                       source_wave_unit_id,organization_id,residual_risk_id,
                       route_kind,member_hash
                   ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'residual',$12)"#,
            )
            .bind(consolidation_id)
            .bind(ordinal)
            .bind(delta.id)
            .bind(delta.source_attempt_id)
            .bind(delta.candidate_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.source_wave_run_id)
            .bind(delta.wave_unit_id)
            .bind(delta.organization_id)
            .bind(residual_risk_id)
            .bind(member_hash)
            .execute(&mut **tx)
            .await?;
            residual_risk_ids.push(residual_risk_id);
        }
        insert_no_attack_consolidation_members(
            tx,
            consolidation_id,
            command,
            &accepted,
            &member_ordinals,
        )
        .await?;
        promote_accepted_fact_delta_memories(
            tx,
            &scope,
            consolidation_id,
            "exhausted",
            None,
            &accepted,
        )
        .await?;
        return Ok(AttackWaveConsolidationResult {
            consolidation_id,
            decision_kind: "exhausted".to_string(),
            target_wave_run_id: None,
            accepted_fact_delta_ids,
            rejected_fact_delta_ids,
            residual_risk_ids,
            pending_enrichment_count: 0,
            replayed: false,
        });
    }

    let target_wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:candidate-wave:{next_generation}", command.operation_id).as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO attack_wave_runs (
               id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,
               policy_hash,max_waves,max_candidates_total,max_chain_depth,max_attempts_total
           ) VALUES ($1,$2,$3,$4,'open',$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(target_wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(next_generation)
    .bind(&source_wave.policy_snapshot)
    .bind(&source_wave.policy_hash)
    .bind(source_wave.max_waves)
    .bind(source_wave.max_candidates_total)
    .bind(source_wave.max_chain_depth)
    .bind(source_wave.max_attempts_total)
    .execute(&mut **tx)
    .await?;
    close_source_wave(tx, &source_wave, &barrier_units).await?;
    insert_consolidation_parent(
        tx,
        consolidation_id,
        command,
        &source_wave,
        "opened_next_wave",
        Some(target_wave_run_id),
        Some(next_generation),
        &source_barrier_hash,
        &fact_delta_set_hash,
        fact_delta_count,
        wave_count,
        candidate_count,
        chain_depth,
        attempt_count,
        "accepted_fact_delta",
    )
    .await?;

    let mut accepted_by_org = BTreeMap::<Uuid, Vec<&FactDeltaCandidateRow>>::new();
    for delta in &follow_on_deltas {
        accepted_by_org
            .entry(delta.organization_id)
            .or_default()
            .push(delta);
    }
    for scope_unit in &scope_units {
        let target_wave_unit_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{target_wave_run_id}:{}", scope_unit.organization_id).as_bytes(),
        );
        let organization_deltas = accepted_by_org.get(&scope_unit.organization_id);
        if organization_deltas.is_some() {
            sqlx::query(
                r#"INSERT INTO attack_wave_units (
                       id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
                       entry_consolidation_id,ordinal,status
                   ) VALUES ($1,$2,$3,$4,$5,$6,$7,'open')"#,
            )
            .bind(target_wave_unit_id)
            .bind(target_wave_run_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(scope_unit.organization_id)
            .bind(consolidation_id)
            .bind(scope_unit.ordinal)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                r#"INSERT INTO attack_wave_units (
                       id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
                       entry_consolidation_id,ordinal,status,review_closed,
                       verification_closed,consolidation_status,terminal_at
                   ) VALUES (
                       $1,$2,$3,$4,$5,$6,$7,'terminal',TRUE,TRUE,'terminal',NOW()
                   )"#,
            )
            .bind(target_wave_unit_id)
            .bind(target_wave_run_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(scope_unit.organization_id)
            .bind(consolidation_id)
            .bind(scope_unit.ordinal)
            .execute(&mut **tx)
            .await?;
        }
        let Some(organization_deltas) = organization_deltas else {
            continue;
        };
        let observations = organization_deltas
            .iter()
            .map(|delta| {
                let route = follow_on_routes
                    .get(&delta.id)
                    .expect("follow-on deltas are projected from the route map");
                attack_candidate_work_items::SeedAttackObservation {
                    work_item_key: format!("fact_delta:{}", delta.id),
                    target_live_id: delta.target_live_id,
                    target_type_at_time: delta.target_type_at_time.clone(),
                    target_value_at_time: delta.target_value_at_time.clone(),
                    target_identity_hash: delta.target_identity_hash.clone(),
                    technique: route.technique.clone(),
                    observation: route.observation.clone(),
                    observation_hash: canonical_hash(&route.observation),
                    source_fact_delta_id: Some(delta.id),
                    delta_kind: Some(route.delta_kind.clone()),
                    observation_kind: route.observation_kind.clone(),
                    allowed_techniques: route.allowed_techniques.clone(),
                    enrichment_required: route.enrichment_required,
                    evidence_ids: route.evidence_ids.clone(),
                }
            })
            .collect();
        let seeded = attack_candidate_work_items::seed_wave_work_items(
            tx,
            attack_candidate_work_items::SeedAttackWorkItems {
                operation_id: command.operation_id,
                scope_snapshot_id: command.scope_snapshot_id,
                wave_run_id: target_wave_run_id,
                wave_unit_id: target_wave_unit_id,
                organization_id: scope_unit.organization_id,
                observations,
            },
        )
        .await?;
        let work_items = seeded
            .items
            .into_iter()
            .map(|item| (item.work_item.work_item_key.clone(), item.work_item.id))
            .collect::<BTreeMap<_, _>>();
        for delta in organization_deltas {
            let work_item_key = format!("fact_delta:{}", delta.id);
            let target_work_item_id = work_items
                .get(&work_item_key)
                .copied()
                .ok_or_else(|| conflict("attack_fact_delta_work_item_missing"))?;
            let ordinal = *member_ordinals
                .get(&delta.id)
                .ok_or_else(|| conflict("attack_fact_delta_ordinal_missing"))?;
            let member_hash = canonical_hash(&serde_json::json!({
                "consolidation_id": consolidation_id,
                "fact_delta_id": delta.id,
                "ordinal": ordinal,
                "organization_id": delta.organization_id,
                "target_wave_run_id": target_wave_run_id,
                "target_wave_unit_id": target_wave_unit_id,
                "target_work_item_id": target_work_item_id,
                "route_kind": "direct",
            }));
            sqlx::query(
                r#"INSERT INTO attack_wave_consolidation_members (
                       consolidation_id,ordinal,fact_delta_id,source_attempt_id,
                       candidate_id,operation_id,scope_snapshot_id,source_wave_run_id,
                       source_wave_unit_id,organization_id,target_wave_run_id,
                       target_wave_unit_id,target_work_item_id,route_kind,member_hash
                   ) VALUES (
                       $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'direct',$14
                   )"#,
            )
            .bind(consolidation_id)
            .bind(ordinal)
            .bind(delta.id)
            .bind(delta.source_attempt_id)
            .bind(delta.candidate_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.source_wave_run_id)
            .bind(delta.wave_unit_id)
            .bind(delta.organization_id)
            .bind(target_wave_run_id)
            .bind(target_wave_unit_id)
            .bind(target_work_item_id)
            .bind(member_hash)
            .execute(&mut **tx)
            .await?;
            attack_fact_deltas::consume_fact_delta(
                tx,
                delta.id,
                command.operation_id,
                command.scope_snapshot_id,
                delta.organization_id,
                target_wave_run_id,
            )
            .await?;
        }
    }

    insert_no_attack_consolidation_members(
        tx,
        consolidation_id,
        command,
        &accepted,
        &member_ordinals,
    )
    .await?;

    promote_accepted_fact_delta_memories(
        tx,
        &scope,
        consolidation_id,
        "opened_next_wave",
        Some(target_wave_run_id),
        &accepted,
    )
    .await?;

    Ok(AttackWaveConsolidationResult {
        consolidation_id,
        decision_kind: "opened_next_wave".to_string(),
        target_wave_run_id: Some(target_wave_run_id),
        accepted_fact_delta_ids,
        rejected_fact_delta_ids,
        residual_risk_ids: Vec::new(),
        pending_enrichment_count: 0,
        replayed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_zero_counts_as_the_first_wave_for_the_wave_cap() {
        assert!(max_wave_count_reached(2, 3));
        assert!(!max_wave_count_reached(1, 3));
    }

    #[test]
    fn raw_fact_delta_dedupe_must_match_the_semantic_identity() {
        let canonical_ref_id = Uuid::new_v4();
        let expected = attack_fact_deltas::semantic_dedupe_hash(
            "sha256:target",
            "api_endpoint",
            canonical_ref_id,
            1,
            "sha256:canonical",
            "new_surface",
        )
        .expect("hash semantic FactDelta identity");
        assert!(fact_delta_dedupe_matches(
            "sha256:target",
            "api_endpoint",
            canonical_ref_id,
            1,
            "sha256:canonical",
            "new_surface",
            &expected,
        )
        .expect("validate exact semantic hash"));
        assert!(!fact_delta_dedupe_matches(
            "sha256:target",
            "api_endpoint",
            canonical_ref_id,
            1,
            "sha256:canonical",
            "new_surface",
            "sha256:caller-chosen-duplicate",
        )
        .expect("reject caller-selected semantic hash"));
    }

    #[test]
    fn refuted_delta_never_becomes_an_attack_technique() {
        assert_eq!(
            classify_fact_delta_follow_on(&FactDeltaRouteInput {
                fact_delta_id: Uuid::from_u128(1),
                delta_kind: "refuted",
                canonical_ref_kind: "api_endpoint",
                canonical_ref_id: Uuid::from_u128(2),
                canonical_ref_version: 1,
                canonical_ref_hash: "sha256:canonical",
                target_live_id: Some(Uuid::from_u128(3)),
                target_type_at_time: "url",
                target_value_at_time: "https://app.example.test",
                target_identity_hash: "sha256:target",
                evidence: &[],
            })
            .expect("refutation is a supported no-attack route"),
            FactDeltaFollowOnRoute::NoAttack
        );
    }

    #[test]
    fn typed_nuclei_fact_delta_keeps_route_axes_separate() {
        let target_id = Uuid::from_u128(3);
        let raw_output = serde_json::json!({
            "schema": "verification.nuclei_template_replay_v1",
            "evidence_role": "proof",
            "result": {
                "target_id": target_id,
                "matched_url": "https://app.example.test/login",
                "template_id": "CVE-2099-0001",
                "technique": "GOLISH-NDAY",
                "completion": "complete",
                "match_count": 1,
                "matches": [{"template_id": "CVE-2099-0001"}],
                "errors": []
            }
        })
        .to_string();
        let evidence = [FactDeltaRouteEvidence {
            evidence_id: 41,
            tool_name: Some("verify_execute_candidate_action"),
            kind: Some("verification.nuclei_template_replay_v1"),
            raw_output: Some(raw_output.as_str()),
            evidence_technique: Some("GOLISH-NDAY"),
            evidence_outcome: Some("found"),
        }];
        let route = classify_fact_delta_follow_on(&FactDeltaRouteInput {
            fact_delta_id: Uuid::from_u128(1),
            delta_kind: "new_surface",
            canonical_ref_kind: "finding",
            canonical_ref_id: Uuid::from_u128(2),
            canonical_ref_version: 1,
            canonical_ref_hash: "sha256:canonical",
            target_live_id: Some(target_id),
            target_type_at_time: "url",
            target_value_at_time: "https://app.example.test",
            target_identity_hash: "sha256:target",
            evidence: &evidence,
        })
        .expect("exact typed replay evidence should route directly");
        let FactDeltaFollowOnRoute::DirectWorkItem(work_item) = route else {
            panic!("expected a direct work item route");
        };
        assert_eq!(work_item.delta_kind, "new_surface");
        assert_eq!(work_item.observation_kind, "nuclei_match_v1");
        assert_eq!(work_item.technique, "GOLISH-NDAY");
        assert_eq!(work_item.allowed_techniques, vec!["GOLISH-NDAY"]);
        assert!(!work_item.enrichment_required);
        assert_ne!(work_item.technique, work_item.delta_kind);
    }

    #[test]
    fn generic_fact_delta_evidence_creates_only_delta_local_enrichment() {
        let evidence = [FactDeltaRouteEvidence {
            evidence_id: 42,
            tool_name: Some("query_target_data"),
            kind: Some("target.snapshot_v1"),
            raw_output: None,
            evidence_technique: None,
            evidence_outcome: Some("found"),
        }];
        let route = classify_fact_delta_follow_on(&FactDeltaRouteInput {
            fact_delta_id: Uuid::from_u128(1),
            delta_kind: "updated",
            canonical_ref_kind: "api_endpoint",
            canonical_ref_id: Uuid::from_u128(2),
            canonical_ref_version: 1,
            canonical_ref_hash: "sha256:canonical",
            target_live_id: Some(Uuid::from_u128(3)),
            target_type_at_time: "url",
            target_value_at_time: "https://app.example.test",
            target_identity_hash: "sha256:target",
            evidence: &evidence,
        })
        .expect("generic evidence should produce bounded enrichment");
        let FactDeltaFollowOnRoute::PendingEnrichment(work_item) = route else {
            panic!("expected a pending enrichment authority route");
        };
        assert_eq!(work_item.observation_kind, "surface_analysis_v2");
        assert_eq!(work_item.technique, "GOLISH-SURFACE-ANALYSIS");
        assert!(work_item.enrichment_required);
        assert!(!work_item.allowed_techniques.is_empty());
        assert!(!work_item
            .allowed_techniques
            .iter()
            .any(|technique| technique == "updated"));
    }

    #[test]
    fn recognized_unsupported_verification_route_fails_closed() {
        let raw_output = serde_json::json!({
            "schema": "verification.future_adapter_v1",
            "evidence_role": "proof",
            "result": {}
        })
        .to_string();
        let evidence = [FactDeltaRouteEvidence {
            evidence_id: 43,
            tool_name: Some("verify_execute_candidate_action"),
            kind: Some("verification.future_adapter_v1"),
            raw_output: Some(raw_output.as_str()),
            evidence_technique: Some("WSTG-INFO"),
            evidence_outcome: Some("found"),
        }];
        let error = classify_fact_delta_follow_on(&FactDeltaRouteInput {
            fact_delta_id: Uuid::from_u128(1),
            delta_kind: "created",
            canonical_ref_kind: "finding",
            canonical_ref_id: Uuid::from_u128(2),
            canonical_ref_version: 1,
            canonical_ref_hash: "sha256:canonical",
            target_live_id: Some(Uuid::from_u128(3)),
            target_type_at_time: "url",
            target_value_at_time: "https://app.example.test",
            target_identity_hash: "sha256:target",
            evidence: &evidence,
        })
        .expect_err("recognized unsupported adapters must roll back consolidation");
        assert_eq!(error, "attack_fact_delta_route_unsupported");
    }
}
