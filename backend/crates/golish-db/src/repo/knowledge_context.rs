//! Exact-scope read model for C7 ContextPack retrieval.
//!
//! Every customer query starts from the immutable operation snapshot and its
//! organization membership. There is deliberately no project-path, NULL-org,
//! legacy `memories`, wiki, or global top-k fallback in this repository.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct KnowledgeAuthorizationSnapshotRow {
    pub project_scope_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub organization_id: Uuid,
    pub frozen_organization_ids: Vec<Uuid>,
    pub server_now: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct KnowledgeContextRow {
    pub item_id: String,
    pub value_kind: String,
    pub text_value: Option<String>,
    pub json_value: Option<Value>,
    pub vault_ref: Option<Uuid>,
    pub source_label: String,
    pub project_scope_id: Uuid,
    pub source_operation_id: Uuid,
    pub scope_snapshot_id: Option<Uuid>,
    pub scope_snapshot_hash: String,
    pub organization_id_at_time: Uuid,
    pub classification: String,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub content_hash: Option<String>,
    pub score_micros: i64,
    pub must_revalidate: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn load_authorization_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Option<Uuid>,
    organization_id: Uuid,
    stage_kind: &str,
) -> Result<Option<KnowledgeAuthorizationSnapshotRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT snapshot.project_scope_id,
                  snapshot.operation_id,
                  snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  member.organization_id,
                  ARRAY(
                      SELECT frozen.organization_id
                        FROM operation_org_scope_units AS frozen
                       WHERE frozen.snapshot_id=snapshot.id
                       ORDER BY frozen.ordinal, frozen.organization_id
                  ) AS frozen_organization_ids,
                  clock_timestamp() AS server_now
             FROM operation_state AS operation
             JOIN tasks AS owner_task ON owner_task.id=operation.operation_id
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units AS member
               ON member.snapshot_id=snapshot.id
              AND member.organization_id=$5
             JOIN stage_runs AS execution
               ON execution.id=$2
              AND execution.operation_id=operation.operation_id
              AND execution.stage_kind=$6
             JOIN stage_run_units AS unit
               ON unit.id=$3
              AND unit.operation_id=operation.operation_id
              AND unit.stage_execution_id=execution.id
              AND unit.scope_snapshot_id=snapshot.id
              AND unit.organization_id=member.organization_id
              AND unit.stage_kind=execution.stage_kind
             LEFT JOIN stage_worker_runs AS worker
               ON worker.id=$4
              AND worker.operation_id=operation.operation_id
              AND worker.stage_execution_id=execution.id
              AND worker.stage_run_unit_id=unit.id
              AND worker.organization_id=member.organization_id
            WHERE operation.operation_id=$1
              AND operation.superseded_by IS NULL
              AND ($4::uuid IS NULL OR worker.id IS NOT NULL)"#,
    )
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(stage_kind)
    .fetch_optional(pool)
    .await
}

pub async fn canonical_current(
    pool: &PgPool,
    operation_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'operation:' || operation.operation_id::text AS item_id,
                  'json'::text AS value_kind,
                  NULL::text AS text_value,
                  jsonb_build_object(
                      'profile', operation.profile,
                      'current_stage', operation.current_stage,
                      'runtime_memory_contract', operation.runtime_memory_contract,
                      'stage_started_at', operation.stage_started_at
                  ) AS json_value,
                  NULL::uuid AS vault_ref,
                  'operation_state'::text AS source_label,
                  snapshot.project_scope_id,
                  operation.operation_id AS source_operation_id,
                  snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  member.organization_id AS organization_id_at_time,
                  'restricted'::text AS classification,
                  ARRAY[]::bigint[] AS evidence_refs,
                  operation.stage_started_at AS valid_from,
                  NULL::timestamptz AS valid_to,
                  NULL::text AS content_hash,
                  1000000::bigint AS score_micros,
                  false AS must_revalidate
             FROM operation_state AS operation
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.id=$2 AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units AS member
               ON member.snapshot_id=snapshot.id AND member.organization_id=$3
            WHERE operation.operation_id=$1 AND operation.superseded_by IS NULL"#,
    )
    .bind(operation_id)
    .bind(snapshot_id)
    .bind(organization_id)
    .fetch_all(pool)
    .await
}

pub async fn runtime_current(
    pool: &PgPool,
    operation_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Option<Uuid>,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'stage_unit:' || unit.id::text AS item_id,
                  'json'::text AS value_kind,
                  NULL::text AS text_value,
                  jsonb_build_object(
                      'stage_kind', unit.stage_kind,
                      'generation', unit.generation,
                      'status', unit.status,
                      'gate_attempt', unit.gate_attempt,
                      'pass_watermark', unit.pass_watermark
                  ) AS json_value,
                  NULL::uuid AS vault_ref,
                  'stage_run_unit'::text AS source_label,
                  snapshot.project_scope_id,
                  unit.operation_id AS source_operation_id,
                  snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  unit.organization_id AS organization_id_at_time,
                  'restricted'::text AS classification,
                  ARRAY[]::bigint[] AS evidence_refs,
                  COALESCE(unit.started_at, unit.updated_at) AS valid_from,
                  NULL::timestamptz AS valid_to,
                  NULL::text AS content_hash,
                  950000::bigint AS score_micros,
                  false AS must_revalidate
             FROM stage_run_units AS unit
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.id=unit.scope_snapshot_id AND snapshot.id=$2
            WHERE unit.id=$5 AND unit.operation_id=$1
              AND unit.stage_execution_id=$4 AND unit.organization_id=$3
            UNION ALL
           SELECT 'stage_worker:' || worker.id::text,
                  'json'::text,
                  NULL::text,
                  jsonb_build_object(
                      'status', worker.status,
                      'work_item_kind', worker.work_item_kind,
                      'work_item_key', worker.work_item_key,
                      'attempt_epoch', worker.attempt_epoch,
                      'checkpoint_version', worker.checkpoint_version
                  ),
                  NULL::uuid,
                  'stage_worker_run'::text,
                  snapshot.project_scope_id,
                  worker.operation_id,
                  snapshot.id,
                  snapshot.scope_hash,
                  worker.organization_id,
                  'restricted'::text,
                  ARRAY[]::bigint[],
                  COALESCE(worker.started_at, worker.updated_at),
                  NULL::timestamptz,
                  NULL::text,
                  940000::bigint,
                  false
             FROM stage_worker_runs AS worker
             JOIN stage_run_units AS unit ON unit.id=worker.stage_run_unit_id
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.id=unit.scope_snapshot_id AND snapshot.id=$2
            WHERE worker.id=$6 AND worker.operation_id=$1
              AND worker.stage_execution_id=$4
              AND worker.stage_run_unit_id=$5
              AND worker.organization_id=$3
            ORDER BY item_id"#,
    )
    .bind(operation_id)
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .fetch_all(pool)
    .await
}

pub async fn current_handoffs(
    pool: &PgPool,
    operation_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
    scope_hash: &str,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'handoff:' || handoff.id::text AS item_id,
                  'json'::text AS value_kind,
                  NULL::text AS text_value,
                  handoff.payload AS json_value,
                  NULL::uuid AS vault_ref,
                  'stage_handoff:' || handoff.from_stage_kind AS source_label,
                  snapshot.project_scope_id,
                  handoff.operation_id AS source_operation_id,
                  handoff.scope_snapshot_id,
                  handoff.scope_hash AS scope_snapshot_hash,
                  handoff.organization_id AS organization_id_at_time,
                  'customer_confidential'::text AS classification,
                  handoff.evidence_ids AS evidence_refs,
                  handoff.gate_passed_at AS valid_from,
                  NULL::timestamptz AS valid_to,
                  handoff.payload_sha256 AS content_hash,
                  900000::bigint AS score_micros,
                  false AS must_revalidate
             FROM stage_handoffs AS handoff
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.id=handoff.scope_snapshot_id
            WHERE handoff.operation_id=$1 AND handoff.scope_snapshot_id=$2
              AND handoff.organization_id=$3 AND handoff.scope_hash=$4
              AND handoff.invalidated_at IS NULL
            ORDER BY handoff.gate_passed_at DESC, handoff.id"#,
    )
    .bind(operation_id)
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(scope_hash)
    .fetch_all(pool)
    .await
}

pub async fn current_episodes(
    pool: &PgPool,
    operation_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
    scope_hash: &str,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'episode:' || episode.episode_id::text AS item_id,
                  'json'::text AS value_kind,
                  NULL::text AS text_value,
                  jsonb_build_object(
                      'stage_kind', episode.stage_kind,
                      'verdict', episode.verdict,
                      'wave', episode.wave,
                      'reason_codes', episode.reason_codes,
                      'fact_refs', episode.fact_refs
                  ) AS json_value,
                  NULL::uuid AS vault_ref,
                  'stage_episode:' || episode.stage_kind AS source_label,
                  episode.project_scope_id,
                  episode.source_operation_id,
                  $2::uuid AS scope_snapshot_id,
                  episode.source_scope_snapshot_hash AS scope_snapshot_hash,
                  episode.organization_id_at_time,
                  'customer_confidential'::text AS classification,
                  episode.evidence_refs,
                  episode.ended_at AS valid_from,
                  NULL::timestamptz AS valid_to,
                  NULL::text AS content_hash,
                  850000::bigint AS score_micros,
                  false AS must_revalidate
             FROM stage_episodes AS episode
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.id=$2
              AND snapshot.operation_id=episode.source_operation_id
              AND snapshot.project_scope_id=episode.project_scope_id
              AND snapshot.scope_hash=episode.source_scope_snapshot_hash
              AND snapshot.sealed_at IS NOT NULL
            WHERE episode.source_operation_id=$1
              AND episode.organization_id_at_time=$3
              AND episode.source_scope_snapshot_hash=$4
            ORDER BY episode.ended_at DESC, episode.episode_id"#,
    )
    .bind(operation_id)
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(scope_hash)
    .fetch_all(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn active_assertions(
    pool: &PgPool,
    project_scope_id: Uuid,
    organization_id: Uuid,
    server_now: DateTime<Utc>,
    classification_ceiling: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'assertion:' || assertion.assertion_id::text AS item_id,
                  assertion.object_kind AS value_kind,
                  CASE WHEN assertion.object_kind='json'
                       THEN NULL::text ELSE NULL::text END AS text_value,
                  assertion.object_value AS json_value,
                  assertion.vault_ref,
                  'knowledge_assertion:' || assertion.predicate AS source_label,
                  assertion.project_scope_id,
                  assertion.source_operation_id,
                  NULL::uuid AS scope_snapshot_id,
                  assertion.source_scope_snapshot_hash AS scope_snapshot_hash,
                  assertion.organization_id_at_time,
                  assertion.classification,
                  assertion.evidence_refs,
                  assertion.valid_from,
                  assertion.valid_to,
                  assertion.content_hash,
                  700000::bigint AS score_micros,
                  true AS must_revalidate
             FROM knowledge_assertions AS assertion
            WHERE assertion.visibility='organization_long_term'
              AND assertion.project_scope_id=$1
              AND assertion.organization_id_at_time=$2
              AND assertion.status='active'
              AND assertion.valid_from <= $3
              AND (assertion.valid_to IS NULL OR assertion.valid_to > $3)
              AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $3)
              AND CASE assertion.classification
                    WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                    WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                  END <= CASE $4
                    WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                    WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                  END
              AND (
                    assertion.subject_key ILIKE '%' || $5 || '%'
                 OR assertion.predicate ILIKE '%' || $5 || '%'
                 OR COALESCE(assertion.object_value::text, '') ILIKE '%' || $5 || '%'
              )
            ORDER BY assertion.source_version DESC,
                     assertion.assertion_identity_hash,
                     assertion.assertion_id
            LIMIT $6"#,
    )
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(server_now)
    .bind(classification_ceiling)
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn active_documents(
    pool: &PgPool,
    project_scope_id: Uuid,
    organization_id: Uuid,
    server_now: DateTime<Utc>,
    classification_ceiling: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT 'document:' || document.document_id::text AS item_id,
                  'text'::text AS value_kind,
                  document.redacted_content AS text_value,
                  NULL::jsonb AS json_value,
                  NULL::uuid AS vault_ref,
                  'knowledge_document:' || document.document_type AS source_label,
                  document.project_scope_id,
                  lineage.source_operation_id,
                  NULL::uuid AS scope_snapshot_id,
                  lineage.source_scope_snapshot_hash AS scope_snapshot_hash,
                  lineage.organization_id_at_time,
                  document.classification,
                  lineage.evidence_refs,
                  document.valid_from,
                  document.valid_to,
                  document.content_hash,
                  650000::bigint AS score_micros,
                  true AS must_revalidate
             FROM knowledge_documents AS document
             JOIN LATERAL (
                 SELECT assertion.source_operation_id,
                        assertion.source_scope_snapshot_hash,
                        assertion.organization_id_at_time,
                        array_agg(DISTINCT evidence_id ORDER BY evidence_id) AS evidence_refs
                   FROM unnest(document.assertion_ids) AS assertion_id
                   JOIN knowledge_assertions AS assertion ON assertion.assertion_id=assertion_id
                   CROSS JOIN LATERAL unnest(assertion.evidence_refs) AS evidence_id
                  WHERE assertion.project_scope_id=$1
                    AND assertion.organization_id_at_time=$2
                    AND assertion.status='active'
                    AND assertion.valid_from <= $3
                    AND (assertion.valid_to IS NULL OR assertion.valid_to > $3)
                  GROUP BY assertion.source_operation_id,
                           assertion.source_scope_snapshot_hash,
                           assertion.organization_id_at_time
             ) AS lineage ON true
            WHERE document.project_scope_id=$1 AND document.status='active'
              AND document.valid_from <= $3
              AND (document.valid_to IS NULL OR document.valid_to > $3)
              AND CASE document.classification
                    WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                    WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                  END <= CASE $4
                    WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                    WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                  END
              AND document.redacted_content ILIKE '%' || $5 || '%'
              AND NOT EXISTS (
                  SELECT 1
                    FROM unnest(document.assertion_ids) AS other_id
                    JOIN knowledge_assertions AS other ON other.assertion_id=other_id
                   WHERE other.project_scope_id IS DISTINCT FROM $1
                      OR other.organization_id_at_time IS DISTINCT FROM $2
              )
            ORDER BY document.source_version DESC, document.document_id
            LIMIT $6"#,
    )
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(server_now)
    .bind(classification_ceiling)
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn vector_documents(
    pool: &PgPool,
    project_scope_id: Uuid,
    organization_id: Uuid,
    server_now: DateTime<Utc>,
    classification_ceiling: &str,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<KnowledgeContextRow>, sqlx::Error> {
    let vector = format!(
        "[{}]",
        query_embedding
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    sqlx::query_as(
        r#"WITH scoped AS (
               SELECT document.*, embedding.embedding,
                      lineage.source_operation_id,
                      lineage.source_scope_snapshot_hash,
                      lineage.organization_id_at_time,
                      lineage.evidence_refs
                 FROM knowledge_embeddings AS embedding
                 JOIN knowledge_documents AS document
                   ON document.document_id=embedding.document_id
                 JOIN LATERAL (
                     SELECT assertion.source_operation_id,
                            assertion.source_scope_snapshot_hash,
                            assertion.organization_id_at_time,
                            array_agg(DISTINCT evidence_id ORDER BY evidence_id) AS evidence_refs
                       FROM unnest(document.assertion_ids) AS assertion_id
                       JOIN knowledge_assertions AS assertion ON assertion.assertion_id=assertion_id
                       CROSS JOIN LATERAL unnest(assertion.evidence_refs) AS evidence_id
                      WHERE assertion.project_scope_id=$1
                        AND assertion.organization_id_at_time=$2
                        AND assertion.status='active'
                        AND assertion.valid_from <= $3
                        AND (assertion.valid_to IS NULL OR assertion.valid_to > $3)
                      GROUP BY assertion.source_operation_id,
                               assertion.source_scope_snapshot_hash,
                               assertion.organization_id_at_time
                 ) AS lineage ON true
                WHERE document.project_scope_id=$1
                  AND document.status='active' AND embedding.status='active'
                  AND document.valid_from <= $3
                  AND (document.valid_to IS NULL OR document.valid_to > $3)
                  AND embedding.valid_from <= $3
                  AND (embedding.valid_to IS NULL OR embedding.valid_to > $3)
                  AND CASE document.classification
                        WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                        WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                      END <= CASE $4
                        WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                        WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                      END
                  AND NOT EXISTS (
                      SELECT 1
                        FROM unnest(document.assertion_ids) AS other_id
                        JOIN knowledge_assertions AS other ON other.assertion_id=other_id
                       WHERE other.project_scope_id IS DISTINCT FROM $1
                          OR other.organization_id_at_time IS DISTINCT FROM $2
                  )
           )
           SELECT 'vector:' || scoped.document_id::text AS item_id,
                  'text'::text AS value_kind,
                  scoped.redacted_content AS text_value,
                  NULL::jsonb AS json_value,
                  NULL::uuid AS vault_ref,
                  'knowledge_vector:' || scoped.document_type AS source_label,
                  scoped.project_scope_id,
                  scoped.source_operation_id,
                  NULL::uuid AS scope_snapshot_id,
                  scoped.source_scope_snapshot_hash AS scope_snapshot_hash,
                  scoped.organization_id_at_time,
                  scoped.classification,
                  scoped.evidence_refs,
                  scoped.valid_from,
                  scoped.valid_to,
                  scoped.content_hash,
                  ((1.0 - (scoped.embedding <=> $5::vector)) * 1000000)::bigint AS score_micros,
                  true AS must_revalidate
             FROM scoped
            ORDER BY scoped.embedding <=> $5::vector, scoped.document_id
            LIMIT $6"#,
    )
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(server_now)
    .bind(classification_ceiling)
    .bind(vector)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn scoped_sql_has_no_legacy_global_or_project_path_fallback() {
        let source = include_str!("knowledge_context.rs");
        let nullable_project_path = ["project_path", " IS NULL"].concat();
        let nullable_organization = ["organization_id", " IS NULL"].concat();
        let legacy_memory_table = ["FROM ", "memories"].concat();
        let legacy_wiki_table = ["wiki", "_kb"].concat();
        assert!(!source.contains(&nullable_project_path));
        assert!(!source.contains(&nullable_organization));
        assert!(!source.contains(&legacy_memory_table));
        assert!(!source.contains(&legacy_wiki_table));
        assert!(source.contains("operation_org_scope_units"));
        assert!(source.contains("knowledge_assertions"));
        assert!(source.contains("knowledge_documents"));
        assert!(source.contains("knowledge_embeddings"));
    }
}
