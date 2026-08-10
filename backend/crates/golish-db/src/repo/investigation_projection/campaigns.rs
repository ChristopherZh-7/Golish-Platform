//! Materialized-only Campaign rail and redacted detail reads.

use std::collections::BTreeSet;

use golish_core::investigation_projection::ProjectionEntityV1;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::types::{
    invalid_payload, InvestigationCampaignDetail, InvestigationCampaignListItem,
    InvestigationCampaignListPage, InvestigationCampaignListQuery, InvestigationCampaignSortKey,
    InvestigationPageValidationInput, InvestigationProjectionResult, InvestigationStageRunSelector,
};
use super::{apply_expected_page_authority, InvestigationProjectionReadSnapshot};

#[derive(Debug, sqlx::FromRow)]
struct CampaignProjectionRow {
    projection_body: Value,
    projection_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RelatedProjectionRow {
    entity_kind: String,
    projection_body: Value,
}

fn body<'a>(
    entity: &'a ProjectionEntityV1,
    expected: &str,
) -> super::types::InvestigationProjectionResult<&'a Value> {
    if entity.entity_kind().as_str() != expected {
        return Err(invalid_payload("Campaign projection entity kind mismatch"));
    }
    Ok(entity.record().canonical_redacted_body().as_value())
}

fn uuid_field(value: &Value, key: &'static str) -> InvestigationProjectionResult<Uuid> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload(format!("Campaign {key} missing")))
        .and_then(|value| {
            Uuid::parse_str(value).map_err(|_| invalid_payload(format!("Campaign {key} invalid")))
        })
}

fn optional_uuid_field(
    value: &Value,
    key: &'static str,
) -> InvestigationProjectionResult<Option<Uuid>> {
    value
        .get(key)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| invalid_payload(format!("Campaign {key} invalid")))
                .and_then(|value| {
                    Uuid::parse_str(value)
                        .map_err(|_| invalid_payload(format!("Campaign {key} invalid")))
                })
        })
        .transpose()
}

fn integer_field(value: &Value, key: &'static str) -> InvestigationProjectionResult<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| invalid_payload(format!("Campaign {key} missing or invalid")))
}

fn bounded_string(
    value: &Value,
    key: &'static str,
    max: usize,
) -> InvestigationProjectionResult<String> {
    let value = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload(format!("Campaign {key} missing")))?;
    if value.trim().is_empty()
        || value.len() > max
        || value
            .chars()
            .any(|ch| matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
    {
        return Err(invalid_payload(format!("Campaign {key} invalid")));
    }
    Ok(value.to_owned())
}

fn parse_campaign(
    row: CampaignProjectionRow,
) -> InvestigationProjectionResult<InvestigationCampaignListItem> {
    let entity: ProjectionEntityV1 = serde_json::from_value(row.projection_body)
        .map_err(|error| invalid_payload(format!("typed Campaign projection: {error}")))?;
    let value = body(&entity, "campaign")?;
    let campaign_id = uuid_field(value, "campaign_id")?;
    let wave_id =
        uuid_field(value, "wave_id").or_else(|_| uuid_field(value, "wave_denominator_id"))?;
    let hypothesis_revision_id = uuid_field(value, "hypothesis_revision_id")?;
    let wave_ordinal = integer_field(value, "wave_ordinal")?;
    let campaign_ordinal = integer_field(value, "campaign_ordinal")
        .or_else(|_| integer_field(value, "campaign_version"))?;
    let label = value
        .get("label")
        .map(|_| bounded_string(value, "label", 512))
        .transpose()?
        .unwrap_or_else(|| format!("Campaign {}", campaign_ordinal + 1));
    let state = bounded_string(value, "state", 64)?;
    let coverage_status = value
        .get("coverage_status")
        .map(|_| bounded_string(value, "coverage_status", 64))
        .transpose()?
        .unwrap_or_else(|| "not_assessed".to_owned());
    Ok(InvestigationCampaignListItem {
        sort_key: InvestigationCampaignSortKey {
            wave_ordinal,
            campaign_ordinal,
            campaign_id,
        },
        campaign_id,
        wave_id,
        hypothesis_revision_id,
        label,
        state,
        coverage_status,
        authority_ref_hash: row.projection_hash,
    })
}

pub async fn list_investigation_campaigns(
    pool: &PgPool,
    operation_id: Uuid,
    query: InvestigationCampaignListQuery,
) -> InvestigationProjectionResult<InvestigationCampaignListPage> {
    list_investigation_campaigns_inner(pool, operation_id, None, query).await
}

pub async fn list_investigation_campaigns_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
    query: InvestigationCampaignListQuery,
) -> InvestigationProjectionResult<InvestigationCampaignListPage> {
    list_investigation_campaigns_inner(pool, operation_id, Some(selector), query).await
}

async fn list_investigation_campaigns_inner(
    pool: &PgPool,
    operation_id: Uuid,
    selector: Option<&InvestigationStageRunSelector>,
    query: InvestigationCampaignListQuery,
) -> InvestigationProjectionResult<InvestigationCampaignListPage> {
    let mut snapshot = if let Some(selector) = selector {
        InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
            .await?
            .0
    } else {
        InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?
    };
    if let Some(expected) = query.expected_page_authority.as_ref() {
        apply_expected_page_authority(&mut snapshot, expected)?;
    }
    let head = snapshot.authority.temporal.as_of_change_seq;
    let rows = sqlx::query_as::<_, CampaignProjectionRow>(
        r#"SELECT projection_body,projection_hash FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,projection_hash,
                      invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='campaign' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let wave_ids = query.filters.wave_ids.into_iter().collect::<BTreeSet<_>>();
    let states = query
        .filters
        .campaign_states
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut campaigns = rows
        .into_iter()
        .map(parse_campaign)
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    campaigns.retain(|campaign| {
        (wave_ids.is_empty() || wave_ids.contains(&campaign.wave_id))
            && (states.is_empty() || states.contains(&campaign.state))
            && query
                .after
                .as_ref()
                .is_none_or(|after| campaign.sort_key > *after)
    });
    campaigns.sort_by(|left, right| left.sort_key.cmp(&right.sort_key));
    let page_size = query.page_size.clamp(1, 100) as usize;
    let has_more = campaigns.len() > page_size;
    campaigns.truncate(page_size);
    let next_key = has_more
        .then(|| campaigns.last().map(|campaign| campaign.sort_key.clone()))
        .flatten();
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok(InvestigationCampaignListPage {
        authority,
        campaigns,
        next_key,
    })
}

pub async fn get_investigation_campaign(
    pool: &PgPool,
    operation_id: Uuid,
    campaign_id: Uuid,
    expected_page_authority: Option<super::types::InvestigationPageValidationInput>,
) -> InvestigationProjectionResult<Option<InvestigationCampaignDetail>> {
    get_investigation_campaign_inner(
        pool,
        operation_id,
        None,
        campaign_id,
        expected_page_authority,
    )
    .await
}

pub async fn get_investigation_campaign_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
    campaign_id: Uuid,
    expected: &InvestigationPageValidationInput,
) -> InvestigationProjectionResult<Option<InvestigationCampaignDetail>> {
    get_investigation_campaign_inner(
        pool,
        operation_id,
        Some(selector),
        campaign_id,
        Some(expected.clone()),
    )
    .await
}

async fn get_investigation_campaign_inner(
    pool: &PgPool,
    operation_id: Uuid,
    selector: Option<&InvestigationStageRunSelector>,
    campaign_id: Uuid,
    expected_page_authority: Option<InvestigationPageValidationInput>,
) -> InvestigationProjectionResult<Option<InvestigationCampaignDetail>> {
    let mut snapshot = if let Some(selector) = selector {
        InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
            .await?
            .0
    } else {
        InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?
    };
    if let Some(expected) = expected_page_authority.as_ref() {
        apply_expected_page_authority(&mut snapshot, expected)?;
    }
    let head = snapshot.authority.temporal.as_of_change_seq;
    let row = sqlx::query_as::<_, CampaignProjectionRow>(
        r#"SELECT projection_body,projection_hash FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,projection_hash,
                      invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='campaign' AND change_seq<=$2
                  AND projection_body #>> '{record,canonicalRedactedBody,campaign_id}'=$3
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .bind(campaign_id.to_string())
    .fetch_optional(&mut *snapshot.tx)
    .await?;
    let Some(row) = row else {
        snapshot.finish().await?;
        return Ok(None);
    };
    let campaign = parse_campaign(row)?;
    let organization_rows: Vec<String> = sqlx::query_scalar(
        r#"SELECT projection_body #>>
                     '{record,canonicalRedactedBody,semantic_key,organization_id}'
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND entity_kind='hypothesis' AND change_seq<=$2
              AND projection_body #>>
                    '{record,canonicalRedactedBody,revision_id}'=$3
            ORDER BY change_seq DESC,entity_version DESC
            LIMIT 2"#,
    )
    .bind(operation_id)
    .bind(head)
    .bind(campaign.hypothesis_revision_id.to_string())
    .fetch_all(&mut *snapshot.tx)
    .await?;
    if organization_rows.len() != 1 {
        return Err(invalid_payload(
            "Campaign Hypothesis organization is unavailable or ambiguous",
        ));
    }
    let organization_id = Uuid::parse_str(&organization_rows[0])
        .map_err(|_| invalid_payload("Campaign Hypothesis organization is invalid"))?;
    let related = sqlx::query_as::<_, RelatedProjectionRow>(
        r#"SELECT entity_kind,projection_body FROM (
               SELECT DISTINCT ON(entity_kind,entity_id) entity_kind,entity_id,projection_body,
                      invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND change_seq<=$2
                  AND entity_kind IN ('campaign_round','prepared_action','residual')
                  AND projection_body #>> '{record,canonicalRedactedBody,campaign_id}'=$3
                ORDER BY entity_kind,entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .bind(campaign_id.to_string())
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let mut round_rows = Vec::new();
    let mut action_rows = Vec::new();
    let mut residual_rows = Vec::new();
    for row in related {
        let entity: ProjectionEntityV1 =
            serde_json::from_value(row.projection_body).map_err(|error| {
                invalid_payload(format!("typed Campaign detail projection: {error}"))
            })?;
        let value = body(&entity, row.entity_kind.as_str())?;
        match row.entity_kind.as_str() {
            "campaign_round" => {
                let ordinal = integer_field(value, "round_ordinal")?;
                let id = uuid_field(value, "round_id")?;
                let summary = value
                    .get("redacted_summary")
                    .or_else(|| value.get("disposition_summary"))
                    .and_then(Value::as_str)
                    .unwrap_or("Round recorded");
                if summary.len() > 1024 {
                    return Err(invalid_payload("Campaign round summary is too large"));
                }
                round_rows.push((ordinal, id, summary.to_owned()));
            }
            "prepared_action" => {
                let ordinal = integer_field(value, "action_ordinal")?;
                let id = uuid_field(value, "prepared_action_id")?;
                let state = bounded_string(value, "state", 64)?;
                let residual_id = optional_uuid_field(value, "residual_id")?;
                action_rows.push((ordinal, id, state, residual_id));
            }
            "residual" => residual_rows.push(uuid_field(value, "residual_id")?),
            _ => return Err(invalid_payload("Campaign detail entity kind unknown")),
        }
    }
    round_rows.sort_by_key(|row| (row.0, row.1));
    action_rows.sort_by_key(|row| (row.0, row.1));
    residual_rows.sort_unstable();
    residual_rows.dedup();
    for residual in action_rows.iter().filter_map(|row| row.3) {
        residual_rows.push(residual);
    }
    residual_rows.sort_unstable();
    residual_rows.dedup();
    let authorized_action_count = action_rows
        .iter()
        .filter(|row| {
            matches!(
                row.2.as_str(),
                "authorized" | "started" | "succeeded" | "failed" | "outcome_unknown"
            )
        })
        .count() as u64;
    let blocked_action_count = action_rows
        .iter()
        .filter(|row| {
            matches!(
                row.2.as_str(),
                "compile_rejected" | "denied" | "expired" | "superseded" | "manually_blocked"
            )
        })
        .count() as u64;
    let detail = InvestigationCampaignDetail {
        authority: snapshot.authority.clone(),
        campaign,
        organization_id,
        round_ids: round_rows.iter().map(|row| row.1).collect(),
        prepared_action_ids: action_rows.iter().map(|row| row.1).collect(),
        authorized_action_count,
        blocked_action_count,
        open_residual_ids: residual_rows,
        redacted_round_summaries: round_rows.into_iter().map(|row| row.2).collect(),
    };
    snapshot.finish().await?;
    Ok(Some(detail))
}
