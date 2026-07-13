use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "objective_attempts";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct ObjectiveAttemptRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub attack_path_id: Option<Uuid>,
    pub objective_kind: String,
    pub simulation_plan: Value,
    pub simulation_plan_hash: String,
    pub outcome: String,
    pub completed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewObjectiveAttempt {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub attack_path_id: Option<Uuid>,
    pub objective_kind: String,
    pub simulation_plan: Value,
    pub simulation_plan_hash: String,
    pub outcome: String,
    pub completed_at: DateTime<Utc>,
    pub evidence: Vec<(i64, String)>,
}

fn validate(input: &NewObjectiveAttempt) -> Result<()> {
    if input.objective_kind.trim().is_empty()
        || input.objective_kind.len() > 128
        || !input.simulation_plan.is_object()
        || input.simulation_plan_hash.len() != 64
        || !input
            .simulation_plan_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !matches!(
            input.outcome.as_str(),
            "simulated_achievable" | "simulated_blocked" | "insufficient_evidence"
        )
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input.evidence.iter().any(|(id, role)| {
            *id <= 0 || !matches!(role.as_str(), "simulation" | "support" | "blocker")
        })
    {
        return Err(anyhow::anyhow!("post_exploit_objective_attempt_invalid").into());
    }
    let mut evidence = input.evidence.clone();
    evidence.sort_unstable();
    evidence.dedup();
    if evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("post_exploit_objective_evidence_duplicate").into());
    }
    Ok(())
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    input: &NewObjectiveAttempt,
) -> Result<ObjectiveAttemptRow> {
    validate(input)?;
    super::foothold_candidates::validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let row = sqlx::query_as::<_, ObjectiveAttemptRow>(
        r#"INSERT INTO objective_attempts(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,attack_path_id,objective_kind,
               simulation_plan,simulation_plan_hash,outcome,completed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT(operation_id,organization_id_at_time,simulation_plan_hash)
           DO NOTHING RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(input.attack_path_id)
    .bind(&input.objective_kind)
    .bind(&input.simulation_plan)
    .bind(&input.simulation_plan_hash)
    .bind(&input.outcome)
    .bind(input.completed_at)
    .fetch_optional(&mut *connection)
    .await?;
    let (row, inserted_new) = match row {
        Some(row) => (row, true),
        None => {
            let existing = sqlx::query_as::<_, ObjectiveAttemptRow>(
                r#"SELECT * FROM objective_attempts
                    WHERE operation_id=$1 AND organization_id_at_time=$2
                      AND simulation_plan_hash=$3 FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(input.organization_id_at_time)
            .bind(&input.simulation_plan_hash)
            .fetch_one(&mut *connection)
            .await?;
            if existing.id != input.id
                || existing.project_scope_id != input.project_scope_id
                || existing.scope_snapshot_id != input.scope_snapshot_id
                || existing.attack_path_id != input.attack_path_id
                || existing.objective_kind != input.objective_kind
                || existing.simulation_plan != input.simulation_plan
                || existing.outcome != input.outcome
                || existing.completed_at != input.completed_at
            {
                return Err(anyhow::anyhow!("post_exploit_objective_replay_conflict").into());
            }
            (existing, false)
        }
    };
    if inserted_new {
        for (evidence_id, role) in &input.evidence {
            sqlx::query(
                r#"INSERT INTO objective_attempt_evidence(
                       objective_attempt_id,evidence_id,role
                   ) VALUES($1,$2,$3)"#,
            )
            .bind(row.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *connection)
            .await?;
        }
    }
    let stored_evidence = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT evidence_id,role FROM objective_attempt_evidence
            WHERE objective_attempt_id=$1 ORDER BY evidence_id,role"#,
    )
    .bind(row.id)
    .fetch_all(&mut *connection)
    .await?;
    let mut expected_evidence = input.evidence.clone();
    expected_evidence.sort_unstable();
    if stored_evidence != expected_evidence {
        return Err(anyhow::anyhow!("post_exploit_objective_evidence_replay_conflict").into());
    }
    Ok(row)
}

pub async fn create_and_emit_with_connection(
    connection: &mut PgConnection,
    input: &NewObjectiveAttempt,
) -> Result<ObjectiveAttemptRow> {
    let row = insert_with_connection(connection, input).await?;
    let source = SourceRef {
        source_kind: CanonicalSourceKind::ObjectiveOutcome,
        row_id: CanonicalRowId::Uuid(row.id),
        source_stream_key: format!("objective-outcome:{}", row.id),
        version: 1,
    };
    let evidence_ids = input
        .evidence
        .iter()
        .map(|(evidence_id, _)| *evidence_id)
        .collect::<Vec<_>>();
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("PostExploitFactTerminal.v1:objective:{}", row.id).as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(row.project_scope_id)),
        organization_id_at_time: Some(row.organization_id_at_time),
        source_operation_id: row.operation_id,
        event_name: KnowledgeEventNameV1::PostExploitFactTerminal,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source: source.clone(),
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            structured_payload: serde_json::json!({
                "fact_kind": "objective_outcome",
                "objective_attempt_id": row.id,
                "attack_path_id": row.attack_path_id,
                "objective_kind": &row.objective_kind,
                "outcome": &row.outcome,
                "simulation_plan_hash": &row.simulation_plan_hash,
                "evidence_ids": evidence_ids,
            }),
        },
        occurred_at: row.completed_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries_with_connection(
        connection, &event,
    )
    .await
    .map_err(|error| anyhow::anyhow!("post_exploit_objective_outbox_failed: {error}"))?;
    Ok(row)
}

pub async fn create(pool: &PgPool, input: &NewObjectiveAttempt) -> Result<ObjectiveAttemptRow> {
    let mut tx = pool.begin().await?;
    let row = create_and_emit_with_connection(&mut tx, input).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ObjectiveAttemptRow>> {
    Ok(
        sqlx::query_as::<_, ObjectiveAttemptRow>("SELECT * FROM objective_attempts WHERE id=$1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}
