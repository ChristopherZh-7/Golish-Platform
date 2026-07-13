use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::repo::post_exploit_actions::PostExploitActionRow;
use crate::Result;

pub const TABLE_NAME: &str = "cleanup_obligations";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct CleanupObligationRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source_action_id: Uuid,
    pub source_action_plan_hash: String,
    pub affected_resource_snapshot: Value,
    pub resource_identity_hash: String,
    pub cleanup_strategy: Value,
    pub proof_requirements: Value,
    pub deadline: DateTime<Utc>,
    pub status: String,
    pub residual_risk: Option<Value>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordActionAndObligation {
    pub action_id: Uuid,
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub principal_id: Uuid,
    pub capability_id: String,
    pub side_effect_class: String,
    pub action_plan: Value,
    pub action_plan_hash: String,
    pub action_evidence: Vec<(i64, String)>,
    pub affected_resource_snapshot: Value,
    pub resource_identity_hash: String,
    pub cleanup_strategy: Value,
    pub proof_requirements: Value,
    pub deadline: DateTime<Utc>,
    pub obligation_evidence: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionAndObligationRow {
    pub action: PostExploitActionRow,
    pub obligation: CleanupObligationRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CleanupTerminalSource {
    IndependentAbsence {
        cleanup_attempt_id: Uuid,
        absence_check_id: Uuid,
    },
    OperatorWaiver {
        waiver_id: Uuid,
    },
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_evidence(evidence: &[(i64, String)], roles: &[&str]) -> Result<()> {
    if evidence.is_empty()
        || evidence.len() > 1024
        || evidence
            .iter()
            .any(|(id, role)| *id <= 0 || !roles.contains(&role.as_str()))
    {
        return Err(anyhow::anyhow!("cleanup_evidence_invalid").into());
    }
    let mut unique = evidence.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != evidence.len() {
        return Err(anyhow::anyhow!("cleanup_evidence_duplicate").into());
    }
    Ok(())
}

fn validate(input: &RecordActionAndObligation) -> Result<()> {
    if input.capability_id.trim().is_empty()
        || input.capability_id.len() > 128
        || input.capability_id.chars().any(char::is_control)
        || !matches!(
            input.side_effect_class.as_str(),
            "remote_state_mutation" | "local_artifact_mutation"
        )
        || !input.action_plan.is_object()
        || !input.affected_resource_snapshot.is_object()
        || !input.cleanup_strategy.is_object()
        || !input.proof_requirements.is_array()
        || input
            .proof_requirements
            .as_array()
            .is_none_or(|items| items.is_empty() || items.len() > 64)
        || !is_hash(&input.action_plan_hash)
        || !is_hash(&input.resource_identity_hash)
    {
        return Err(anyhow::anyhow!("cleanup_action_obligation_invalid").into());
    }
    validate_evidence(&input.action_evidence, &["plan", "result", "support"])?;
    validate_evidence(
        &input.obligation_evidence,
        &["source", "strategy", "support"],
    )
}

async fn validate_authority(
    connection: &mut PgConnection,
    input: &RecordActionAndObligation,
) -> Result<()> {
    let principal_is_active: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM operator_principals
                WHERE id=$1 AND principal_kind='local_operator' AND active
           )"#,
    )
    .bind(input.principal_id)
    .fetch_one(&mut *connection)
    .await?;
    if !principal_is_active {
        return Err(anyhow::anyhow!("cleanup_operator_untrusted").into());
    }
    let scope_is_exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM operation_org_scope_snapshots AS snapshot
                 JOIN operation_org_scope_units AS unit
                   ON unit.snapshot_id=snapshot.id
                WHERE snapshot.id=$1 AND snapshot.operation_id=$2
                  AND snapshot.project_scope_id=$3
                  AND snapshot.sealed_at IS NOT NULL
                  AND unit.organization_id=$4
           )"#,
    )
    .bind(input.scope_snapshot_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.organization_id_at_time)
    .fetch_one(&mut *connection)
    .await?;
    if !scope_is_exact {
        return Err(anyhow::anyhow!("cleanup_scope_not_authorized").into());
    }
    let mut evidence_ids = input
        .action_evidence
        .iter()
        .chain(&input.obligation_evidence)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    super::foothold_candidates::validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &evidence_ids,
    )
    .await
}

async fn exact_evidence(
    connection: &mut PgConnection,
    table: &str,
    id_column: &str,
    id: Uuid,
    expected: &[(i64, String)],
) -> Result<()> {
    let query = format!(
        "SELECT evidence_id,role FROM {table} WHERE {id_column}=$1 ORDER BY evidence_id,role"
    );
    let stored = sqlx::query_as::<_, (i64, String)>(&query)
        .bind(id)
        .fetch_all(&mut *connection)
        .await?;
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    if stored != expected {
        return Err(anyhow::anyhow!("cleanup_evidence_replay_conflict").into());
    }
    Ok(())
}

/// Emits the typed Memory Fabric event from persisted canonical rows only.
/// The caller owns the surrounding action + obligation transaction.
pub async fn append_action_prepared_event_with_connection(
    connection: &mut PgConnection,
    action_id: Uuid,
    obligation_id: Uuid,
) -> Result<Uuid> {
    let action = sqlx::query_as::<_, PostExploitActionRow>(
        "SELECT * FROM post_exploit_actions WHERE id=$1 FOR SHARE",
    )
    .bind(action_id)
    .fetch_one(&mut *connection)
    .await?;
    let obligation = sqlx::query_as::<_, CleanupObligationRow>(
        "SELECT * FROM cleanup_obligations WHERE id=$1 FOR SHARE",
    )
    .bind(obligation_id)
    .fetch_one(&mut *connection)
    .await?;
    let action_relation = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT cleanup_obligation_id FROM post_exploit_actions WHERE id=$1",
    )
    .bind(action_id)
    .fetch_one(&mut *connection)
    .await?;
    if action_relation != Some(obligation.id)
        || obligation.source_action_id != action.id
        || action.operation_id != obligation.operation_id
        || action.project_scope_id != obligation.project_scope_id
        || action.scope_snapshot_id != obligation.scope_snapshot_id
        || action.organization_id_at_time != obligation.organization_id_at_time
        || action.plan_hash != obligation.source_action_plan_hash
    {
        return Err(anyhow::anyhow!("cleanup_action_obligation_event_mismatch").into());
    }

    let action_evidence = sqlx::query_scalar::<_, i64>(
        "SELECT evidence_id FROM post_exploit_action_evidence WHERE action_id=$1 ORDER BY evidence_id,role",
    )
    .bind(action.id)
    .fetch_all(&mut *connection)
    .await?;
    let obligation_evidence = sqlx::query_scalar::<_, i64>(
        "SELECT evidence_id FROM cleanup_obligation_evidence WHERE obligation_id=$1 ORDER BY evidence_id,role",
    )
    .bind(obligation.id)
    .fetch_all(&mut *connection)
    .await?;
    let mut evidence_ids = action_evidence
        .into_iter()
        .chain(obligation_evidence)
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    if evidence_ids.is_empty() || evidence_ids.iter().any(|evidence_id| *evidence_id <= 0) {
        return Err(anyhow::anyhow!("cleanup_action_obligation_event_evidence_missing").into());
    }

    let source = SourceRef {
        source_kind: CanonicalSourceKind::PostExploitAction,
        row_id: CanonicalRowId::Uuid(action.id),
        source_stream_key: format!("post-exploit-action:{}", action.id),
        version: 1,
    };
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("PostExploitActionPrepared.v1:{}", action.id).as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(action.project_scope_id)),
        organization_id_at_time: Some(action.organization_id_at_time),
        source_operation_id: action.operation_id,
        event_name: KnowledgeEventNameV1::PostExploitActionPrepared,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            source,
            structured_payload: serde_json::json!({
                "action_id": action.id,
                "obligation_id": obligation.id,
                "capability": action.capability_id.clone(),
                "side_effect_class": action.side_effect_class.clone(),
                "plan_hash": action.plan_hash.clone(),
                "resource_identity_hash": obligation.resource_identity_hash.clone(),
                "evidence_ids": evidence_ids,
            }),
        },
        occurred_at: action.created_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries_with_connection(
        connection, &event,
    )
    .await
    .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}

/// Emits terminal Cleanup truth and all catalog deliveries from canonical rows.
/// The caller owns the surrounding terminal-state transaction.
pub(super) async fn append_terminal_event_with_connection(
    connection: &mut PgConnection,
    obligation_id: Uuid,
    terminal_source: CleanupTerminalSource,
) -> Result<Uuid> {
    let obligation = sqlx::query_as::<_, CleanupObligationRow>(
        "SELECT * FROM cleanup_obligations WHERE id=$1 FOR SHARE",
    )
    .bind(obligation_id)
    .fetch_one(&mut *connection)
    .await?;
    let terminal_at = obligation
        .terminal_at
        .ok_or_else(|| anyhow::anyhow!("cleanup_terminal_event_state_mismatch"))?;

    let (structured_payload, mut evidence_ids) = match terminal_source {
        CleanupTerminalSource::IndependentAbsence {
            cleanup_attempt_id,
            absence_check_id,
        } => {
            let attempt = sqlx::query_as::<_, super::cleanup_attempts::CleanupAttemptRow>(
                "SELECT * FROM cleanup_attempts WHERE id=$1 FOR SHARE",
            )
            .bind(cleanup_attempt_id)
            .fetch_one(&mut *connection)
            .await?;
            let absence =
                sqlx::query_as::<_, super::cleanup_absence_checks::CleanupAbsenceCheckRow>(
                    "SELECT * FROM cleanup_absence_checks WHERE id=$1 FOR SHARE",
                )
                .bind(absence_check_id)
                .fetch_one(&mut *connection)
                .await?;
            if obligation.status != "verified_absent"
                || obligation.residual_risk.is_some()
                || attempt.obligation_id != obligation.id
                || attempt.status != "verified_absent"
                || absence.obligation_id != obligation.id
                || absence.cleanup_attempt_id != attempt.id
                || absence.disposition != "absent"
                || absence.resource_identity_hash != obligation.resource_identity_hash
            {
                return Err(anyhow::anyhow!("cleanup_terminal_event_state_mismatch").into());
            }
            let evidence_ids = sqlx::query_scalar::<_, i64>(
                r#"SELECT evidence_id
                     FROM (
                         SELECT evidence_id FROM cleanup_obligation_evidence
                          WHERE obligation_id=$1
                         UNION
                         SELECT evidence_id FROM cleanup_attempt_evidence
                          WHERE attempt_id=$2
                         UNION
                         SELECT evidence_id FROM cleanup_absence_check_evidence
                          WHERE absence_check_id=$3
                     ) AS terminal_evidence
                    ORDER BY evidence_id"#,
            )
            .bind(obligation.id)
            .bind(attempt.id)
            .bind(absence.id)
            .fetch_all(&mut *connection)
            .await?;
            (
                serde_json::json!({
                    "obligation_id": obligation.id,
                    "terminal_kind": "independent_absence",
                    "terminal_status": "verified_absent",
                    "resource_identity_hash": obligation.resource_identity_hash.clone(),
                    "cleanup_attempt_id": attempt.id,
                    "absence_check_id": absence.id,
                    "evidence_ids": evidence_ids.clone(),
                }),
                evidence_ids,
            )
        }
        CleanupTerminalSource::OperatorWaiver { waiver_id } => {
            let waiver = sqlx::query_as::<_, super::cleanup_waivers::CleanupWaiverRow>(
                "SELECT * FROM cleanup_waivers WHERE id=$1 FOR SHARE",
            )
            .bind(waiver_id)
            .fetch_one(&mut *connection)
            .await?;
            if obligation.status != "waived_by_user"
                || waiver.obligation_id != obligation.id
                || obligation.residual_risk.as_ref() != Some(&waiver.residual_risk)
            {
                return Err(anyhow::anyhow!("cleanup_terminal_event_state_mismatch").into());
            }
            let evidence_ids = sqlx::query_scalar::<_, i64>(
                r#"SELECT evidence_id
                     FROM (
                         SELECT evidence_id FROM cleanup_obligation_evidence
                          WHERE obligation_id=$1
                         UNION
                         SELECT evidence_id FROM cleanup_waiver_evidence
                          WHERE waiver_id=$2
                     ) AS terminal_evidence
                    ORDER BY evidence_id"#,
            )
            .bind(obligation.id)
            .bind(waiver.id)
            .fetch_all(&mut *connection)
            .await?;
            (
                serde_json::json!({
                    "obligation_id": obligation.id,
                    "terminal_kind": "operator_waiver",
                    "terminal_status": "waived_by_user",
                    "resource_identity_hash": obligation.resource_identity_hash.clone(),
                    "waiver_id": waiver.id,
                    "residual_risk": waiver.residual_risk.clone(),
                    "evidence_ids": evidence_ids.clone(),
                }),
                evidence_ids,
            )
        }
    };
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    if evidence_ids.is_empty() || evidence_ids.iter().any(|evidence_id| *evidence_id <= 0) {
        return Err(anyhow::anyhow!("cleanup_terminal_event_evidence_missing").into());
    }

    let source = SourceRef {
        source_kind: CanonicalSourceKind::CleanupObligation,
        row_id: CanonicalRowId::Uuid(obligation.id),
        source_stream_key: format!("cleanup-obligation:{}", obligation.id),
        version: obligation.row_version,
    };
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!(
                "CleanupObligationTerminal.v1:{}:{}",
                obligation.id, obligation.row_version
            )
            .as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(obligation.project_scope_id)),
        organization_id_at_time: Some(obligation.organization_id_at_time),
        source_operation_id: obligation.operation_id,
        event_name: KnowledgeEventNameV1::CleanupObligationTerminal,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            source,
            structured_payload,
        },
        occurred_at: terminal_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries_with_connection(
        connection, &event,
    )
    .await
    .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}

pub async fn record_action_and_obligation(
    pool: &PgPool,
    input: &RecordActionAndObligation,
) -> Result<ActionAndObligationRow> {
    validate(input)?;
    let mut tx = pool.begin().await?;
    validate_authority(&mut tx, input).await?;

    let inserted_action = sqlx::query_as::<_, PostExploitActionRow>(
        r#"INSERT INTO post_exploit_actions(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,capability_id,side_effect_class,plan,
               plan_hash,cleanup_obligation_id,prepared_by_principal_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT DO NOTHING RETURNING *"#,
    )
    .bind(input.action_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(&input.capability_id)
    .bind(&input.side_effect_class)
    .bind(&input.action_plan)
    .bind(&input.action_plan_hash)
    .bind(input.obligation_id)
    .bind(input.principal_id)
    .fetch_optional(&mut *tx)
    .await?;
    let action_inserted = inserted_action.is_some();
    let action = match inserted_action {
        Some(row) => row,
        None => {
            let row = sqlx::query_as::<_, PostExploitActionRow>(
                "SELECT * FROM post_exploit_actions WHERE id=$1 FOR SHARE",
            )
            .bind(input.action_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cleanup_action_identity_conflict"))?;
            let relation = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>)>(
                r#"SELECT cleanup_obligation_id,prepared_by_principal_id
                     FROM post_exploit_actions WHERE id=$1"#,
            )
            .bind(input.action_id)
            .fetch_one(&mut *tx)
            .await?;
            if row.operation_id != input.operation_id
                || row.project_scope_id != input.project_scope_id
                || row.scope_snapshot_id != input.scope_snapshot_id
                || row.organization_id_at_time != input.organization_id_at_time
                || row.capability_id != input.capability_id
                || row.side_effect_class != input.side_effect_class
                || row.plan != input.action_plan
                || row.plan_hash != input.action_plan_hash
                || relation.0 != Some(input.obligation_id)
                || relation.1 != Some(input.principal_id)
            {
                return Err(anyhow::anyhow!("cleanup_action_replay_conflict").into());
            }
            row
        }
    };

    let inserted_obligation = sqlx::query_as::<_, CleanupObligationRow>(
        r#"INSERT INTO cleanup_obligations(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,source_action_id,source_action_plan_hash,
               affected_resource_snapshot,resource_identity_hash,cleanup_strategy,
               proof_requirements,deadline
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           ON CONFLICT DO NOTHING RETURNING *"#,
    )
    .bind(input.obligation_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(input.action_id)
    .bind(&input.action_plan_hash)
    .bind(&input.affected_resource_snapshot)
    .bind(&input.resource_identity_hash)
    .bind(&input.cleanup_strategy)
    .bind(&input.proof_requirements)
    .bind(input.deadline)
    .fetch_optional(&mut *tx)
    .await?;
    let obligation_inserted = inserted_obligation.is_some();
    let obligation = match inserted_obligation {
        Some(row) => row,
        None => {
            let row = sqlx::query_as::<_, CleanupObligationRow>(
                "SELECT * FROM cleanup_obligations WHERE id=$1 FOR SHARE",
            )
            .bind(input.obligation_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cleanup_obligation_identity_conflict"))?;
            if row.operation_id != input.operation_id
                || row.project_scope_id != input.project_scope_id
                || row.scope_snapshot_id != input.scope_snapshot_id
                || row.organization_id_at_time != input.organization_id_at_time
                || row.source_action_id != input.action_id
                || row.source_action_plan_hash != input.action_plan_hash
                || row.affected_resource_snapshot != input.affected_resource_snapshot
                || row.resource_identity_hash != input.resource_identity_hash
                || row.cleanup_strategy != input.cleanup_strategy
                || row.proof_requirements != input.proof_requirements
                || row.deadline != input.deadline
            {
                return Err(anyhow::anyhow!("cleanup_obligation_replay_conflict").into());
            }
            row
        }
    };

    if action_inserted {
        for (evidence_id, role) in &input.action_evidence {
            sqlx::query(
                "INSERT INTO post_exploit_action_evidence(action_id,evidence_id,role) VALUES($1,$2,$3)",
            )
            .bind(action.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }
    }
    if obligation_inserted {
        for (evidence_id, role) in &input.obligation_evidence {
            sqlx::query(
                "INSERT INTO cleanup_obligation_evidence(obligation_id,evidence_id,role) VALUES($1,$2,$3)",
            )
            .bind(obligation.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }
    }
    exact_evidence(
        &mut tx,
        "post_exploit_action_evidence",
        "action_id",
        action.id,
        &input.action_evidence,
    )
    .await?;
    exact_evidence(
        &mut tx,
        "cleanup_obligation_evidence",
        "obligation_id",
        obligation.id,
        &input.obligation_evidence,
    )
    .await?;
    append_action_prepared_event_with_connection(&mut tx, action.id, obligation.id).await?;
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ActionAndObligationRow { action, obligation })
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<CleanupObligationRow>> {
    Ok(
        sqlx::query_as::<_, CleanupObligationRow>("SELECT * FROM cleanup_obligations WHERE id=$1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Vec<CleanupObligationRow>> {
    Ok(sqlx::query_as::<_, CleanupObligationRow>(
        r#"SELECT * FROM cleanup_obligations
            WHERE operation_id=$1 ORDER BY created_at,id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?)
}

pub async fn list_side_effects_missing_obligations(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        r#"SELECT action.id
             FROM post_exploit_actions AS action
             LEFT JOIN cleanup_obligations AS obligation
               ON obligation.source_action_id=action.id
            WHERE action.operation_id=$1 AND action.side_effect_class <> 'none'
              AND action.status IN ('planned','authorized','executing','succeeded','recovery_required')
              AND obligation.id IS NULL
            ORDER BY action.id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?)
}
