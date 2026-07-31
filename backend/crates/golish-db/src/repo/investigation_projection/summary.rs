//! Materialized-only operation/generation/count summary.

use golish_core::investigation_projection::ProjectionEntityV1;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::types::{invalid_payload, InvestigationProjectionResult, InvestigationSummary};
use super::InvestigationProjectionReadSnapshot;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryResidualBody {
    residual_id: Uuid,
    residual_hash: String,
    reason: String,
    #[serde(default)]
    root_id: Option<Uuid>,
    #[serde(default)]
    revision_id: Option<Uuid>,
}

pub async fn read_investigation_summary(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<InvestigationSummary> {
    let mut snapshot = InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?;
    let head = snapshot.authority.temporal.as_of_change_seq;
    let hypothesis_rows: Vec<Value> = sqlx::query_scalar(
        r#"SELECT projection_body FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='hypothesis' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let mut current_hypothesis_count = 0i64;
    let mut closed_hypothesis_count = 0i64;
    let mut contested_hypothesis_count = 0i64;
    for row in hypothesis_rows {
        let entity: ProjectionEntityV1 = serde_json::from_value(row)
            .map_err(|error| invalid_payload(format!("typed summary Hypothesis: {error}")))?;
        let ProjectionEntityV1::Hypothesis(record) = entity else {
            return Err(invalid_payload(
                "summary Hypothesis row has another entity kind",
            ));
        };
        let state = record
            .record()
            .canonical_redacted_body()
            .as_value()
            .get("state")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_payload("summary Hypothesis state missing"))?;
        match state {
            "verified" | "refuted" | "invalid" => closed_hypothesis_count += 1,
            "proposed" | "supported" | "contested" | "inconclusive" => {
                current_hypothesis_count += 1
            }
            _ => return Err(invalid_payload("summary Hypothesis state unknown")),
        }
        if state == "contested" {
            contested_hypothesis_count += 1;
        }
    }

    let generation: Option<Value> = sqlx::query_scalar(
        r#"SELECT projection_body
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND entity_kind='generation' AND change_seq<=$2
            ORDER BY change_seq DESC LIMIT 1"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_optional(&mut *snapshot.tx)
    .await?;
    let (active_generation_id, active_generation_seal_hash) = if let Some(row) = generation {
        let entity: ProjectionEntityV1 = serde_json::from_value(row)
            .map_err(|error| invalid_payload(format!("typed generation projection: {error}")))?;
        let ProjectionEntityV1::Generation(record) = entity else {
            return Err(invalid_payload("generation row has another entity kind"));
        };
        let body = record.record().canonical_redacted_body().as_value();
        let generation_id = body
            .get("generation_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| invalid_payload("generation id missing"))?;
        let generation_hash = body
            .get("generation_hash")
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() == 71
                    && value.starts_with("sha256:")
                    && value[7..]
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
            .ok_or_else(|| invalid_payload("generation seal hash missing"))?;
        (Some(generation_id), Some(generation_hash.to_owned()))
    } else {
        (None, None)
    };

    let residual_rows: Vec<Value> = sqlx::query_scalar(
        r#"SELECT projection_body FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='residual' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    for row in &residual_rows {
        let entity = serde_json::from_value::<ProjectionEntityV1>(row.clone())
            .map_err(|error| invalid_payload(format!("summary residual payload: {error}")))?;
        let ProjectionEntityV1::Residual(record) = entity else {
            return Err(invalid_payload(
                "summary residual row has another entity kind",
            ));
        };
        let body: SummaryResidualBody =
            serde_json::from_value(record.record().canonical_redacted_body().as_value().clone())
                .map_err(|error| invalid_payload(format!("summary residual body: {error}")))?;
        if body.residual_id.is_nil()
            || body.reason.trim().is_empty()
            || body.residual_hash.len() != 71
            || !body.residual_hash.starts_with("sha256:")
            || body.revision_id.is_some() && body.root_id.is_none()
        {
            return Err(invalid_payload("summary residual identity invalid"));
        }
    }
    let residual_count = i64::try_from(residual_rows.len())
        .map_err(|_| invalid_payload("residual count overflow"))?;
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok(InvestigationSummary {
        authority,
        active_generation_id,
        active_generation_seal_hash,
        current_hypothesis_count,
        closed_hypothesis_count,
        contested_hypothesis_count,
        residual_count,
    })
}
