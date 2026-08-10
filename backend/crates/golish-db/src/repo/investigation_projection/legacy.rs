//! Read-only legacy compatibility views derived by the projector.

use std::collections::BTreeMap;

use golish_core::investigation_projection::ProjectionEntityV1;
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::types::{InvestigationLegacyProjection, InvestigationProjectionResult};
use super::version::LegacyField;
use crate::repo::hypothesis_legacy_projection::{
    read_legacy_attempt_projection, LegacyCompatibilityReadDisposition,
};

#[derive(Debug, sqlx::FromRow)]
struct LegacyRow {
    entity_id: Uuid,
    projection_status: String,
}

pub(super) async fn load_legacy_candidate_map_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
    root_ids: &[Uuid],
) -> InvestigationProjectionResult<BTreeMap<Uuid, InvestigationLegacyProjection>> {
    if root_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = sqlx::query_as::<_, LegacyRow>(
        r#"SELECT DISTINCT ON(entity_id) entity_id,projection_status
             FROM hypothesis_legacy_candidate_projection_versions
            WHERE operation_id=$1 AND change_seq<=$2 AND entity_id=ANY($3)
            ORDER BY entity_id,entity_version DESC,change_seq DESC"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .bind(root_ids)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let unavailable_fields = match row.projection_status.as_str() {
                "ready" => Vec::new(),
                "unsupported" | "invalidated" => {
                    vec!["legacy_candidate".to_owned(), "legacy_attempt".to_owned()]
                }
                _ => vec!["legacy_projection_status".to_owned()],
            };
            (
                row.entity_id,
                InvestigationLegacyProjection {
                    status: Some(row.projection_status),
                    unavailable_fields,
                },
            )
        })
        .collect())
}

pub(super) fn unavailable_legacy_projection() -> InvestigationLegacyProjection {
    InvestigationLegacyProjection {
        status: None,
        unavailable_fields: vec!["legacy_candidate".to_owned(), "legacy_attempt".to_owned()],
    }
}

/// Read-only grandfathered Attempt observation.  Missing Plan C concepts are
/// represented explicitly; they are never synthesized from legacy prose.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyAttemptHistoryV1 {
    pub attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub organization_id: Uuid,
    pub disposition: String,
    pub action_observation: LegacyField<Value>,
    pub strategy: LegacyField<Value>,
    pub consult: LegacyField<Value>,
    pub concrete_request_packet: LegacyField<Value>,
    pub oracle: LegacyField<Value>,
    pub synthetic_campaign_id: Option<Uuid>,
    pub campaign_terminal_receipt_id: Option<Uuid>,
    pub read_only: bool,
}

/// Load the latest materialized legacy Attempt without joining live canonical
/// tables or creating Campaign/oracle authority.
pub async fn load_attempt_history(
    pool: &PgPool,
    operation_id: Uuid,
    attempt_id: Uuid,
) -> crate::Result<LegacyAttemptHistoryV1> {
    let read = read_legacy_attempt_projection(pool, operation_id, attempt_id).await?;
    let projection = read.projection.ok_or_else(|| {
        crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_PROJECTION_MISSING"))
    })?;
    if projection.projection_status != "ready" {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "LEGACY_ATTEMPT_PROJECTION_UNAVAILABLE"
        )));
    }
    let body = projection.projection_body.ok_or_else(|| {
        crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_PROJECTION_BODY_MISSING"))
    })?;
    let entity: ProjectionEntityV1 = serde_json::from_value(body)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
    let ProjectionEntityV1::LegacyAttemptProjection(record) = entity else {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "LEGACY_ATTEMPT_PROJECTION_KIND_MISMATCH"
        )));
    };
    let body = record.record().canonical_redacted_body().as_value();
    let body_attempt_id = body
        .get("attempt_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_ID_INVALID")))?;
    let candidate_id = body
        .get("candidate_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_CANDIDATE_ID_INVALID"))
        })?;
    let organization_id = body
        .get("organization_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_ORGANIZATION_ID_INVALID"))
        })?;
    if body_attempt_id != attempt_id {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "LEGACY_ATTEMPT_PROJECTION_IDENTITY_MISMATCH"
        )));
    }
    let disposition = body
        .get("disposition")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            crate::DbError::Other(anyhow::anyhow!("LEGACY_ATTEMPT_DISPOSITION_INVALID"))
        })?
        .to_owned();
    let action_observation = json!({
        "candidate_plan_hash":body.get("candidate_plan_hash"),
        "result_hash":body.get("result_hash"),
        "source_occurred_at":projection.source_occurred_at,
        "source_time_status":projection.source_time_status,
        "source_contract_hash":projection.source_contract_hash,
        "projection_hash":projection.projection_hash,
    });
    Ok(LegacyAttemptHistoryV1 {
        attempt_id,
        candidate_id,
        organization_id,
        disposition,
        action_observation: LegacyField::Available(action_observation),
        strategy: LegacyField::LegacyUnavailable,
        consult: LegacyField::LegacyUnavailable,
        concrete_request_packet: LegacyField::LegacyUnavailable,
        oracle: LegacyField::LegacyUnavailable,
        synthetic_campaign_id: None,
        campaign_terminal_receipt_id: None,
        read_only: matches!(
            read.disposition,
            LegacyCompatibilityReadDisposition::Ready
                | LegacyCompatibilityReadDisposition::HistoricalReadOnly
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{LegacyAttemptHistoryV1, LegacyField};
    use uuid::Uuid;

    #[test]
    fn legacy_attempt_shape_never_synthesizes_campaign_or_oracle() {
        let view = LegacyAttemptHistoryV1 {
            attempt_id: Uuid::nil(),
            candidate_id: Uuid::nil(),
            organization_id: Uuid::nil(),
            disposition: "blocked".to_owned(),
            action_observation: LegacyField::LegacyUnavailable,
            strategy: LegacyField::LegacyUnavailable,
            consult: LegacyField::LegacyUnavailable,
            concrete_request_packet: LegacyField::LegacyUnavailable,
            oracle: LegacyField::LegacyUnavailable,
            synthetic_campaign_id: None,
            campaign_terminal_receipt_id: None,
            read_only: true,
        };
        assert_eq!(view.oracle, LegacyField::LegacyUnavailable);
        assert!(view.synthetic_campaign_id.is_none());
        assert!(view.campaign_terminal_receipt_id.is_none());
    }
}
