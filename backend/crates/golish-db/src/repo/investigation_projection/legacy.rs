//! Read-only legacy compatibility views derived by the projector.

use std::collections::BTreeMap;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::types::{InvestigationLegacyProjection, InvestigationProjectionResult};

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
