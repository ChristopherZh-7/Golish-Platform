//! Materialized-only operation/generation/count summary.

use golish_core::investigation_projection::ProjectionEntityV1;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::types::{
    invalid_payload, InvestigationActorTopologyNode, InvestigationCoverageDenominator,
    InvestigationGenerationSummary, InvestigationOpenObligationSummary,
    InvestigationPageValidationInput, InvestigationProjectionResult,
    InvestigationSourceCensusMember, InvestigationStageRunReadAuthority,
    InvestigationStageRunSelector, InvestigationSummary, InvestigationWaveSummary,
};
use super::{apply_expected_page_authority, InvestigationProjectionReadSnapshot};

const EXACT_MAIN_ACTOR_SQL: &str = r#"SELECT 'main'::TEXT AS actor_kind,item.organization_id,
              NULL::UUID AS hypothesis_revision_id,NULL::UUID AS task_id,
              NULL::UUID AS subtask_id,worker.id AS worker_run_id,
              $3::TEXT AS owning_stage_run_request_id,
              concat($3::TEXT,'::team:',item.organization_id::TEXT,
                     '::lead:',worker.id::TEXT) AS transcript_request_id,
              NULL::TEXT AS parent_actor_transcript_request_id,
              NULL::TEXT AS parent_dispatch_tool_request_id,worker.status,
              TRUE AS identity_valid
         FROM operation_org_scope_snapshots scope
         JOIN stage_work_items item
           ON item.operation_id=scope.operation_id
          AND item.stage_execution_id=$2
          AND item.scope_snapshot_id=scope.id
          AND item.organization_id=scope.root_organization_id
          AND item.kind='investigation_primary'
          AND item.stable_key='leader:primary'
          AND item.role='investigation'
          AND item.created_by='server_seed'
         JOIN LATERAL (
             SELECT run.id,run.parent_request_id,run.status
               FROM stage_worker_runs run
              WHERE run.work_item_id=item.id
                AND run.operation_id=item.operation_id
                AND run.stage_execution_id=item.stage_execution_id
                AND run.stage_run_unit_id=item.stage_run_unit_id
                AND run.organization_id=item.organization_id
              ORDER BY run.worker_generation DESC
              LIMIT 1
         ) worker ON TRUE
        WHERE scope.operation_id=$1 AND scope.id=$4
        ORDER BY item.id
        LIMIT 2"#;

pub async fn read_investigation_summary(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<InvestigationSummary> {
    Ok(
        read_investigation_summary_inner(pool, operation_id, None, None)
            .await?
            .0,
    )
}

pub async fn read_investigation_summary_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
    expected: Option<&InvestigationPageValidationInput>,
) -> InvestigationProjectionResult<(InvestigationSummary, InvestigationStageRunReadAuthority)> {
    let (summary, stage_run) =
        read_investigation_summary_inner(pool, operation_id, Some(selector), expected).await?;
    if summary.main_actor.is_none() {
        return Err(invalid_payload(
            "exact Investigation Main actor is unavailable",
        ));
    }
    Ok((
        summary,
        stage_run.expect("exact stage-run summary always captures stage authority"),
    ))
}

async fn load_exact_source_census_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
) -> InvestigationProjectionResult<Vec<InvestigationSourceCensusMember>> {
    let rows = sqlx::query_as::<_, InvestigationSourceCensusMember>(
        r#"SELECT organization_id,snapshot_id,context_item_count,
                  context_item_set_sha256,methodology_hit_count,
                  methodology_result_set_sha256,omission_count,omission_set_sha256
             FROM investigation_analysis_snapshot_authorities
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND owning_stage_run_request_id=$3 AND scope_snapshot_id=$4
            ORDER BY organization_id,snapshot_id
            LIMIT 65"#,
    )
    .bind(operation_id)
    .bind(selector.stage_execution_id)
    .bind(&selector.stage_run_request_id)
    .bind(selector.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 64 {
        return Err(invalid_payload(
            "Investigation source census exceeds bounded summary capacity",
        ));
    }
    Ok(rows)
}

async fn load_exact_main_actor_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
) -> InvestigationProjectionResult<Option<InvestigationActorTopologyNode>> {
    let rows = sqlx::query_as::<_, InvestigationActorTopologyNode>(EXACT_MAIN_ACTOR_SQL)
        .bind(operation_id)
        .bind(selector.stage_execution_id)
        .bind(&selector.stage_run_request_id)
        .bind(selector.scope_snapshot_id)
        .fetch_all(&mut **tx)
        .await?;
    if rows.len() > 1 {
        return Err(invalid_payload(
            "exact Investigation Main actor identity is ambiguous",
        ));
    }
    Ok(rows.into_iter().next())
}

pub(super) async fn load_exact_actor_topology_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
) -> InvestigationProjectionResult<Vec<InvestigationActorTopologyNode>> {
    let rows = sqlx::query_as::<_, InvestigationActorTopologyNode>(
        r#"SELECT dispatch.actor_kind,dispatch.organization_id,
                  task.hypothesis_revision_id,
                  CASE WHEN plan.subject_kind='verification_task'
                       THEN plan.subject_id ELSE NULL END AS task_id,
                  dispatch.subtask_id,dispatch.worker_run_id,
                  plan.owning_stage_run_request_id,dispatch.transcript_request_id,
                  dispatch.parent_actor_transcript_request_id,
                  dispatch.parent_dispatch_tool_request_id,worker.status,
                  CASE WHEN dispatch.actor_kind='primary' THEN
                         dispatch.transcript_request_id=concat(
                             $3::TEXT,'::team:',dispatch.organization_id::TEXT,
                             '::lead:',dispatch.worker_run_id::TEXT)
                         AND dispatch.parent_actor_transcript_request_id IS NULL
                         AND dispatch.parent_dispatch_tool_request_id IS NULL
                       ELSE
                         parent.dispatch_receipt_id IS NOT NULL
                         AND parent.transcript_request_id=
                             dispatch.parent_actor_transcript_request_id
                         AND worker.parent_request_id=
                             dispatch.parent_dispatch_tool_request_id
                         AND dispatch.transcript_request_id=concat(
                             dispatch.parent_dispatch_tool_request_id,
                             '::worker:',dispatch.worker_run_id::TEXT)
                  END AS identity_valid
             FROM pentagi_logical_dispatch_receipts dispatch
             JOIN investigation_pentagi_task_plans plan
               ON plan.task_plan_id=dispatch.task_plan_id
              AND plan.operation_id=dispatch.operation_id
              AND plan.stage_execution_id=dispatch.stage_execution_id
              AND plan.stage_run_unit_id=dispatch.stage_run_unit_id
              AND plan.organization_id=dispatch.organization_id
             JOIN stage_worker_runs worker
               ON worker.id=dispatch.worker_run_id
             LEFT JOIN pentagi_logical_dispatch_receipts parent
               ON parent.dispatch_receipt_id=dispatch.parent_dispatch_receipt_id
              AND parent.task_plan_id=dispatch.task_plan_id
             LEFT JOIN hypothesis_verification_tasks task
               ON plan.subject_kind='verification_task'
              AND task.task_id=plan.subject_id
              AND task.operation_id=plan.operation_id
              AND task.stage_execution_id=plan.stage_execution_id
              AND task.organization_id=plan.organization_id
            WHERE dispatch.operation_id=$1 AND dispatch.stage_execution_id=$2
              AND dispatch.scope_snapshot_id=$4
              AND plan.owning_stage_run_request_id=$3
            ORDER BY plan.organization_id,plan.task_plan_id,
                     dispatch.dispatch_ordinal,dispatch.dispatch_receipt_id
            LIMIT 257"#,
    )
    .bind(operation_id)
    .bind(selector.stage_execution_id)
    .bind(&selector.stage_run_request_id)
    .bind(selector.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 256 {
        return Err(invalid_payload(
            "Investigation actor topology exceeds bounded summary capacity",
        ));
    }
    for actor in &rows {
        let parent_required = matches!(actor.actor_kind.as_str(), "worker" | "nested_worker");
        if !actor.identity_valid
            || actor.transcript_request_id.trim().is_empty()
            || parent_required
                != (actor.parent_actor_transcript_request_id.is_some()
                    && actor.parent_dispatch_tool_request_id.is_some())
            || actor.actor_kind == "primary"
                && (actor.subtask_id.is_some()
                    || actor.parent_actor_transcript_request_id.is_some()
                    || actor.parent_dispatch_tool_request_id.is_some())
            || actor.task_id.is_some() != actor.hypothesis_revision_id.is_some()
        {
            return Err(invalid_payload(
                "Investigation actor identity projection is inconsistent",
            ));
        }
    }
    Ok(rows)
}

async fn read_investigation_summary_inner(
    pool: &PgPool,
    operation_id: Uuid,
    selector: Option<&InvestigationStageRunSelector>,
    expected: Option<&InvestigationPageValidationInput>,
) -> InvestigationProjectionResult<(
    InvestigationSummary,
    Option<InvestigationStageRunReadAuthority>,
)> {
    let (mut snapshot, stage_run) = if let Some(selector) = selector {
        let (snapshot, stage_run) =
            InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
                .await?;
        (snapshot, Some(stage_run))
    } else {
        (
            InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?,
            None,
        )
    };
    if let Some(expected) = expected {
        apply_expected_page_authority(&mut snapshot, expected)?;
    }
    let (source_census, main_actor, actor_topology) = if let Some(selector) = selector {
        (
            load_exact_source_census_on(&mut snapshot.tx, operation_id, selector).await?,
            load_exact_main_actor_on(&mut snapshot.tx, operation_id, selector).await?,
            load_exact_actor_topology_on(&mut snapshot.tx, operation_id, selector).await?,
        )
    } else {
        (Vec::new(), None, Vec::new())
    };
    let head = snapshot.authority.temporal.as_of_change_seq;
    let (
        current_hypothesis_count,
        closed_hypothesis_count,
        contested_hypothesis_count,
        hypothesis_payloads_valid,
    ): (i64, i64, i64, bool) = sqlx::query_as(
        r#"WITH latest AS MATERIALIZED (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='hypothesis' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) SELECT
               COUNT(*) FILTER(WHERE projection_body #>>
                   '{record,canonicalRedactedBody,state}' IN(
                       'proposed','supported','contested','inconclusive')),
               COUNT(*) FILTER(WHERE projection_body #>>
                   '{record,canonicalRedactedBody,state}' IN(
                       'verified','refuted','invalid')),
               COUNT(*) FILTER(WHERE projection_body #>>
                   '{record,canonicalRedactedBody,state}'='contested'),
               COALESCE(BOOL_AND(
                   projection_body #>> '{entityKind}'='hypothesis'
                   AND jsonb_typeof(projection_body #>
                       '{record,canonicalRedactedBody}')='object'
                   AND projection_body #>>
                       '{record,canonicalRedactedBody,state}' IN(
                           'proposed','supported','contested','inconclusive',
                           'verified','refuted','invalid')),TRUE)
             FROM latest"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    if !hypothesis_payloads_valid {
        return Err(invalid_payload(
            "summary Hypothesis projection payload is invalid",
        ));
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

    let (residual_count, residual_payloads_valid): (i64, bool) = sqlx::query_as(
        r#"SELECT COUNT(*),COALESCE(BOOL_AND(
                   projection_body #>> '{entityKind}'='residual'
                   AND jsonb_typeof(projection_body #>
                       '{record,canonicalRedactedBody}')='object'
                   AND projection_body #>>
                       '{record,canonicalRedactedBody,residual_id}' ~*
                       '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                   AND projection_body #>>
                       '{record,canonicalRedactedBody,residual_id}'<>
                       '00000000-0000-0000-0000-000000000000'
                   AND projection_body #>>
                       '{record,canonicalRedactedBody,residual_hash}' ~
                       '^sha256:[0-9a-f]{64}$'
                   AND btrim(COALESCE(projection_body #>>
                       '{record,canonicalRedactedBody,reason}',''))<>''
                   AND (projection_body #>>
                           '{record,canonicalRedactedBody,revision_id}' IS NULL
                        OR (projection_body #>>
                               '{record,canonicalRedactedBody,revision_id}' ~*
                               '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                            AND projection_body #>>
                               '{record,canonicalRedactedBody,root_id}' ~*
                               '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'))),TRUE)
             FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='residual' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    if !residual_payloads_valid {
        return Err(invalid_payload("summary residual identity invalid"));
    }
    let generation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM (
               SELECT DISTINCT ON(entity_id) entity_id,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='generation' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    let generation_rows: Vec<(Value, i64)> = sqlx::query_as(
        r#"SELECT projection_body,change_seq FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,change_seq,
                      invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='generation' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL ORDER BY change_seq,entity_id LIMIT 32"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let mut generations = Vec::with_capacity(generation_rows.len());
    for (ordinal, (row, _)) in generation_rows.into_iter().enumerate() {
        let entity: ProjectionEntityV1 = serde_json::from_value(row)
            .map_err(|error| invalid_payload(format!("typed summary Generation: {error}")))?;
        let ProjectionEntityV1::Generation(record) = entity else {
            return Err(invalid_payload(
                "summary Generation row has another entity kind",
            ));
        };
        let body = record.record().canonical_redacted_body().as_value();
        let generation_id = body
            .get("generation_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| invalid_payload("summary Generation id invalid"))?;
        generations.push(InvestigationGenerationSummary {
            generation_id,
            generation_ordinal: i64::try_from(ordinal)
                .map_err(|_| invalid_payload("summary Generation ordinal overflow"))?,
            state: "sealed".to_owned(),
        });
    }
    let campaign_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM (
               SELECT DISTINCT ON(entity_id) entity_id,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='campaign' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    let wave_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT COALESCE(
                    projection_body #>> '{record,canonicalRedactedBody,wave_id}',
                    projection_body #>> '{record,canonicalRedactedBody,wave_denominator_id}'))
             FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='campaign' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
             ) latest WHERE invalidation_reason IS NULL"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    let wave_rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT DISTINCT COALESCE(
                    projection_body #>> '{record,canonicalRedactedBody,wave_id}',
                    projection_body #>> '{record,canonicalRedactedBody,wave_denominator_id}'),
                    (projection_body #>> '{record,canonicalRedactedBody,wave_ordinal}')::BIGINT
             FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='campaign' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
             ) latest WHERE invalidation_reason IS NULL
             ORDER BY 2,1 LIMIT 32"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let waves = wave_rows
        .into_iter()
        .map(|(wave_id, wave_ordinal)| {
            Ok(InvestigationWaveSummary {
                wave_id: Uuid::parse_str(&wave_id)
                    .map_err(|_| invalid_payload("summary Wave id invalid"))?,
                wave_ordinal,
                state: "projected".to_owned(),
            })
        })
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    let obligation_rows: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT entity_id,entity_kind FROM (
               SELECT DISTINCT ON(entity_kind,entity_id) entity_kind,entity_id,
                      projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND change_seq<=$2 AND entity_kind IN (
                    'strategy_obligation','cleanup_obligation','callback_obligation',
                    'enrichment_obligation','application_fact_refinement_obligation'
                )
                ORDER BY entity_kind,entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL
             AND COALESCE(
                 projection_body #>> '{record,canonicalRedactedBody,state}',
                 projection_body #>> '{record,canonicalRedactedBody,status}',
                 projection_body #>> '{record,canonicalRedactedBody,disposition}',
                 'open'
             ) NOT IN ('closed','completed','satisfied','not_applicable','superseded')
             ORDER BY entity_kind,entity_id LIMIT 64"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let open_obligations = obligation_rows
        .into_iter()
        .map(
            |(obligation_id, obligation_kind)| InvestigationOpenObligationSummary {
                obligation_id,
                obligation_kind,
            },
        )
        .collect::<Vec<_>>();
    let open_obligation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM (
               SELECT DISTINCT ON(entity_kind,entity_id) entity_kind,entity_id,
                      projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND change_seq<=$2 AND entity_kind IN (
                    'strategy_obligation','cleanup_obligation','callback_obligation',
                    'enrichment_obligation','application_fact_refinement_obligation'
                )
                ORDER BY entity_kind,entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL
             AND COALESCE(
                 projection_body #>> '{record,canonicalRedactedBody,state}',
                 projection_body #>> '{record,canonicalRedactedBody,status}',
                 projection_body #>> '{record,canonicalRedactedBody,disposition}',
                 'open'
             ) NOT IN ('closed','completed','satisfied','not_applicable','superseded')"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_one(&mut *snapshot.tx)
    .await?;
    let coverage_rows: Vec<Value> = sqlx::query_scalar(
        r#"SELECT projection_body FROM (
               SELECT DISTINCT ON(entity_id) entity_id,projection_body,invalidation_reason
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind='coverage' AND change_seq<=$2
                ORDER BY entity_id,entity_version DESC
           ) latest WHERE invalidation_reason IS NULL LIMIT 65"#,
    )
    .bind(operation_id)
    .bind(head)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    if coverage_rows.len() > 64 {
        return Err(invalid_payload(
            "Investigation coverage summary exceeds bounded capacity",
        ));
    }
    let mut coverage_denominator = InvestigationCoverageDenominator::default();
    for row in coverage_rows {
        let entity: ProjectionEntityV1 = serde_json::from_value(row)
            .map_err(|error| invalid_payload(format!("typed summary Coverage: {error}")))?;
        let ProjectionEntityV1::Coverage(record) = entity else {
            return Err(invalid_payload(
                "summary Coverage row has another entity kind",
            ));
        };
        let body = record.record().canonical_redacted_body().as_value();
        let count = |key: &'static str| -> InvestigationProjectionResult<i64> {
            body.get(key)
                .and_then(Value::as_i64)
                .filter(|value| *value >= 0)
                .ok_or_else(|| invalid_payload(format!("summary Coverage {key} invalid")))
        };
        coverage_denominator.planned = coverage_denominator
            .planned
            .checked_add(count("planned")?)
            .ok_or_else(|| invalid_payload("summary Coverage planned overflow"))?;
        coverage_denominator.tested_complete = coverage_denominator
            .tested_complete
            .checked_add(count("tested_complete")?)
            .ok_or_else(|| invalid_payload("summary Coverage complete overflow"))?;
        coverage_denominator.tested_degraded = coverage_denominator
            .tested_degraded
            .checked_add(count("tested_degraded")?)
            .ok_or_else(|| invalid_payload("summary Coverage degraded overflow"))?;
        coverage_denominator.untested = coverage_denominator
            .untested
            .checked_add(count("untested")?)
            .ok_or_else(|| invalid_payload("summary Coverage untested overflow"))?;
        coverage_denominator.blocked = coverage_denominator
            .blocked
            .checked_add(count("blocked")?)
            .ok_or_else(|| invalid_payload("summary Coverage blocked overflow"))?;
    }
    let partition_total = coverage_denominator
        .tested_complete
        .checked_add(coverage_denominator.tested_degraded)
        .and_then(|value| value.checked_add(coverage_denominator.untested))
        .and_then(|value| value.checked_add(coverage_denominator.blocked))
        .ok_or_else(|| invalid_payload("summary Coverage partition overflow"))?;
    if partition_total != coverage_denominator.planned {
        return Err(invalid_payload(
            "summary Coverage denominator is not an exact partition",
        ));
    }
    let coverage_grade = if coverage_denominator.planned == 0 {
        "not_assessed"
    } else if coverage_denominator.untested == 0 && coverage_denominator.blocked == 0 {
        "complete"
    } else {
        "partial"
    };
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok((
        InvestigationSummary {
            authority,
            active_generation_id,
            active_generation_seal_hash,
            current_hypothesis_count,
            closed_hypothesis_count,
            contested_hypothesis_count,
            residual_count,
            generation_count,
            wave_count,
            campaign_count,
            open_obligation_count,
            control_decision: "not_assessed".to_owned(),
            coverage_grade: coverage_grade.to_owned(),
            coverage_denominator,
            coverage_sufficiency: "not_assessed".to_owned(),
            generations,
            waves,
            open_obligations,
            source_census,
            main_actor,
            actor_topology,
        },
        stage_run,
    ))
}

#[cfg(test)]
mod tests {
    use super::EXACT_MAIN_ACTOR_SQL;

    #[test]
    fn exact_main_actor_uses_the_unified_investigation_primary_identity() {
        for required in [
            "item.kind='investigation_primary'",
            "item.stable_key='leader:primary'",
            "item.role='investigation'",
            "item.created_by='server_seed'",
        ] {
            assert!(
                EXACT_MAIN_ACTOR_SQL.contains(required),
                "missing exact unified Main predicate: {required}"
            );
        }
        assert!(!EXACT_MAIN_ACTOR_SQL.contains("company_stage_controller"));
    }
}
